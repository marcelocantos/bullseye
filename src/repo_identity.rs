// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Identify a checkout by the repository it clones (🎯T113).
//!
//! External ledgers used to be keyed only by absolute path, so a linked
//! worktree, a second clone, a moved repo or another machine each lost
//! the ledger. The `origin` remote names the repository independently of
//! where it is checked out, and every worktree of a clone shares it, so
//! it is the primary key; the path stays as the fallback for checkouts
//! that have no usable remote.
//!
//! Forks deliberately get distinct identities: a fork's `origin` is the
//! fork. Linking a fork to its upstream is 🎯T112.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use crate::bounded::{GIT_QUERY_TIMEOUT, git_query};
use crate::cache;

/// Where a checkout sits relative to the repository it clones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIdentity {
    /// Normalised remote identity: host first, then each path segment
    /// of the remote (`["github.com", "owner", "repo"]`). Lowercased,
    /// `.git` stripped. GitLab subgroups contribute extra segments.
    pub segments: Vec<String>,
    /// Work-tree root of this checkout (`git rev-parse --show-toplevel`).
    pub toplevel: PathBuf,
    /// The queried directory relative to [`Self::toplevel`]; empty at
    /// the root. Keeps monorepo subprojects on separate ledgers.
    pub prefix: PathBuf,
}

impl RepoIdentity {
    /// `host/owner/repo` form, for messages.
    pub fn key(&self) -> String {
        self.segments.join("/")
    }
}

type IdentityCache = HashMap<PathBuf, (Instant, Option<RepoIdentity>)>;

static CACHE: Mutex<Option<IdentityCache>> = Mutex::new(None);

/// Identity of the checkout containing `dir`, or `None` when `dir` is
/// not in a git work tree, has no `origin`, or the URL does not parse.
///
/// Memoised per directory for [`cache::ttl`]: discovery runs on every
/// tool call and this costs two git subprocesses. The TTL also bounds
/// how long a remote change goes unnoticed.
pub fn identify(dir: &Path) -> Option<RepoIdentity> {
    let key = dir.to_path_buf();
    {
        let guard = CACHE.lock().expect("repo identity cache poisoned");
        if let Some((stamped, hit)) = guard.as_ref().and_then(|m| m.get(&key))
            && !cache::expired(*stamped)
        {
            return hit.clone();
        }
    }
    let found = identify_uncached(dir);
    let mut guard = CACHE.lock().expect("repo identity cache poisoned");
    let map = guard.get_or_insert_with(HashMap::new);
    cache::sweep(map);
    map.insert(key, (Instant::now(), found.clone()));
    found
}

fn identify_uncached(dir: &Path) -> Option<RepoIdentity> {
    // One call yields both lines: toplevel, then the prefix (which is
    // an empty line at the root).
    let out = git_query(
        dir,
        &["rev-parse", "--show-toplevel", "--show-prefix"],
        GIT_QUERY_TIMEOUT,
    )?;
    let mut lines = out.lines();
    let toplevel = PathBuf::from(lines.next()?.trim());
    if toplevel.as_os_str().is_empty() {
        return None;
    }
    let prefix = PathBuf::from(lines.next().unwrap_or("").trim().trim_end_matches('/'));

    let url = git_query(
        dir,
        &["config", "--get", "remote.origin.url"],
        GIT_QUERY_TIMEOUT,
    )?;
    // Hosts and GitHub owner/repo names are case-insensitive, so the
    // key is lowercased; `remote_segments` keeps the original case for
    // callers that build paths a human chose.
    let segments = remote_segments(url.trim())?
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    Some(RepoIdentity {
        segments,
        toplevel,
        prefix,
    })
}

/// Parse a git remote URL into identity segments (`host`, then the
/// remote path's components), case preserved and `.git` stripped. Returns `None` for
/// local paths, `file://` URLs, and anything else without a host and
/// at least an owner and a repo.
///
/// Accepted shapes:
///
/// - scp-like: `[user@]host:owner/repo[.git]`
/// - `https://`, `http://`, `ssh://`, `git://`, with optional
///   `user@` and `:port`
pub fn remote_segments(url: &str) -> Option<Vec<String>> {
    let (host, path) = split_remote(url)?;
    let host = host
        .rsplit_once('@')
        .map_or(host, |(_, h)| h)
        .split(':')
        .next()?
        .to_string();
    if host.is_empty() {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segments = vec![host];
    for part in path.split('/') {
        // Reject anything that could escape or alias a directory once
        // the segments become path components.
        if part.is_empty() || part == "." || part == ".." || part.contains('\\') {
            return None;
        }
        segments.push(part.to_string());
    }
    // host + owner + repo at minimum.
    if segments.len() < 3 {
        return None;
    }
    Some(segments)
}

fn split_remote(url: &str) -> Option<(&str, &str)> {
    if let Some((scheme, rest)) = url.split_once("://") {
        return match scheme {
            "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git" => rest.split_once('/'),
            _ => None,
        };
    }
    // scp-like syntax: the colon must come before any slash, otherwise
    // git treats the string as a local path.
    let colon = url.find(':')?;
    if url[..colon].contains('/') {
        return None;
    }
    Some((&url[..colon], &url[colon + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(url: &str) -> Option<String> {
        remote_segments(url).map(|s| s.join("/").to_ascii_lowercase())
    }

    #[test]
    fn scp_forms() {
        assert_eq!(
            segs("git@github.com:marcelocantos/bullseye.git").as_deref(),
            Some("github.com/marcelocantos/bullseye")
        );
        assert_eq!(
            segs("git@github.com:marcelocantos/bullseye").as_deref(),
            Some("github.com/marcelocantos/bullseye")
        );
        assert_eq!(
            segs("org-1@github.com:Owner/Repo.git").as_deref(),
            Some("github.com/owner/repo")
        );
    }

    #[test]
    fn url_forms() {
        assert_eq!(
            segs("https://github.com/marcelocantos/bullseye.git").as_deref(),
            Some("github.com/marcelocantos/bullseye")
        );
        assert_eq!(
            segs("https://GitHub.com/MarceloCantos/Bullseye/").as_deref(),
            Some("github.com/marcelocantos/bullseye")
        );
        assert_eq!(
            segs("ssh://git@github.com:22/o/r.git").as_deref(),
            Some("github.com/o/r")
        );
        assert_eq!(
            segs("https://user:tok@gitlab.com/group/sub/project").as_deref(),
            Some("gitlab.com/group/sub/project")
        );
    }

    #[test]
    fn segments_preserve_case() {
        assert_eq!(
            remote_segments("git@github.com:Health-Management-Systems/hms.git"),
            Some(vec![
                "github.com".to_string(),
                "Health-Management-Systems".to_string(),
                "hms".to_string()
            ])
        );
    }

    #[test]
    fn same_repo_over_ssh_and_https_is_one_identity() {
        assert_eq!(
            remote_segments("git@github.com:o/r.git"),
            remote_segments("https://github.com/o/r")
        );
    }

    #[test]
    fn rejects_non_remotes() {
        for url in [
            "",
            "not-a-url",
            "file:///some/path",
            "/abs/path/repo",
            "../relative/repo",
            "./a:b/c",
            "https://github.com/onlyowner",
            "https://github.com/o/../r",
            "git@github.com:o//r",
        ] {
            assert_eq!(remote_segments(url), None, "{url}");
        }
    }
}
