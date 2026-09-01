// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Repo-reference resolution (🎯T29).
//!
//! Maps a partial reference — a leaf name (`spyder`), an `org/repo`
//! fragment (`marcelocantos/spyder`), or a full host+org+repo path —
//! to an absolute repo root by suffix-matching against a workspace
//! scan. Absolute paths pass through unchanged. Ambiguity is
//! surfaced; bullseye never silently picks a winner.
//!
//! Scan reuses the `bullseye_portfolio` walk semantics (default
//! `~/work/`, max depth 5, skip hidden/`node_modules`/`target`/
//! `vendor`, treat any directory containing `bullseye.yaml` as a
//! repo). Results are memoised per-process keyed by workspace root,
//! same shape as the 🎯T28 git-history cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cache;
use std::sync::Mutex;

const MAX_DEPTH: usize = 5;

#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// Reference matched no repo under the workspace root.
    NotFound {
        reference: String,
        workspace_root: PathBuf,
    },
    /// Reference matched more than one repo. Lists every candidate
    /// so the caller can pick one with a more specific reference.
    Ambiguous {
        reference: String,
        workspace_root: PathBuf,
        candidates: Vec<PathBuf>,
    },
    /// Reference is an absolute path, but no repo (directory containing
    /// `bullseye.yaml`) lives there. Returned separately from `NotFound`
    /// so the error message can name the path rather than the workspace.
    AbsoluteNotFound { path: PathBuf },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::NotFound {
                reference,
                workspace_root,
            } => write!(
                f,
                "no repo matched `{reference}` under {workspace_root}. Call \
                 `bullseye_portfolio` to see the available repos.",
                workspace_root = workspace_root.display(),
            ),
            ResolveError::Ambiguous {
                reference,
                workspace_root,
                candidates,
            } => {
                writeln!(
                    f,
                    "`{reference}` matched {n} repos under {workspace_root}. Re-issue with \
                     more path qualifiers (`org/repo` or the full host+org+repo path):",
                    n = candidates.len(),
                    workspace_root = workspace_root.display(),
                )?;
                for c in candidates {
                    writeln!(f, "  - {}", c.display())?;
                }
                Ok(())
            }
            ResolveError::AbsoluteNotFound { path } => write!(
                f,
                "absolute path {path} is not a bullseye-tracked repo (no `bullseye.yaml` \
                 there).",
                path = path.display(),
            ),
        }
    }
}

/// Resolve a repo reference to an absolute repo root.
///
/// `reference` may be:
/// - An absolute path (passed through after verifying it contains
///   `bullseye.yaml`).
/// - A `/`-separated suffix of a repo's path components (e.g.
///   `spyder`, `marcelocantos/spyder`, `github.com/marcelocantos/spyder`).
///
/// The scan walks `workspace_root` looking for `bullseye.yaml`,
/// matching the portfolio scanner's semantics. Results are
/// memoised per-process; call [`clear_cache_for_tests`] from tests
/// that mutate the workspace between calls.
pub fn resolve(workspace_root: &Path, reference: &str) -> Result<PathBuf, ResolveError> {
    let trimmed = reference.trim().trim_end_matches('/');
    let candidate_path = Path::new(trimmed);
    if candidate_path.is_absolute() {
        if candidate_path.join("bullseye.yaml").is_file() {
            return Ok(candidate_path.to_path_buf());
        }
        return Err(ResolveError::AbsoluteNotFound {
            path: candidate_path.to_path_buf(),
        });
    }

    let ref_parts: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    if ref_parts.is_empty() {
        return Err(ResolveError::NotFound {
            reference: reference.to_string(),
            workspace_root: workspace_root.to_path_buf(),
        });
    }

    let repos = scan_repos(workspace_root);
    let matches: Vec<PathBuf> = repos
        .iter()
        .filter(|repo| repo_matches_suffix(repo, &ref_parts))
        .cloned()
        .collect();

    match matches.len() {
        0 => Err(ResolveError::NotFound {
            reference: reference.to_string(),
            workspace_root: workspace_root.to_path_buf(),
        }),
        1 => Ok(matches.into_iter().next().expect("len == 1")),
        _ => Err(ResolveError::Ambiguous {
            reference: reference.to_string(),
            workspace_root: workspace_root.to_path_buf(),
            candidates: matches,
        }),
    }
}

/// True if `ref_parts` matches the trailing components of `repo`'s path.
fn repo_matches_suffix(repo: &Path, ref_parts: &[&str]) -> bool {
    use std::path::Component;
    let repo_parts: Vec<&str> = repo
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    if ref_parts.len() > repo_parts.len() {
        return false;
    }
    let start = repo_parts.len() - ref_parts.len();
    repo_parts[start..] == *ref_parts
}

/// Per-process scan cache. Keyed by workspace root.
/// Workspace scans, stamped with the time they were taken.
///
/// Unlike the id-allocation and parse caches this has no exactness
/// check to fall back on — a workspace gains and loses repos by
/// filesystem operations bullseye never sees. The TTL is therefore the
/// correctness mechanism, not merely eviction: without it a repo cloned
/// after the process started stays invisible for the life of that
/// process, which was one agent session under stdio and is unbounded
/// under a daemon (🎯T78.1).
type StampedScan = (Instant, Vec<PathBuf>);

static CACHE: Mutex<Option<HashMap<PathBuf, StampedScan>>> = Mutex::new(None);

fn scan_repos(workspace_root: &Path) -> Vec<PathBuf> {
    {
        let guard = CACHE.lock().expect("resolve cache poisoned");
        if let Some(map) = guard.as_ref()
            && let Some((stamped, hit)) = map.get(workspace_root)
            && !cache::expired(*stamped)
        {
            return hit.clone();
        }
    }
    // Walk outside the lock: it touches the filesystem and must not
    // block every other workspace resolution while it runs.
    let repos = walk_for_repos(workspace_root);
    let mut guard = CACHE.lock().expect("resolve cache poisoned");
    let map = guard.get_or_insert_with(HashMap::new);
    cache::sweep(map);
    map.insert(
        workspace_root.to_path_buf(),
        (Instant::now(), repos.clone()),
    );
    repos
}

fn walk_for_repos(workspace_root: &Path) -> Vec<PathBuf> {
    let mut repos: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(workspace_root.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if dir.join("bullseye.yaml").is_file() {
            repos.push(dir);
            continue;
        }
        if depth >= MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.')
                || name == "node_modules"
                || name == "target"
                || name == "vendor"
            {
                continue;
            }
            stack.push((path, depth + 1));
        }
    }

    repos.sort();
    repos
}

/// Test helper. Drop the memoised scan so the next `resolve` call
/// re-walks the workspace.
pub fn clear_cache_for_tests() {
    CACHE.lock().expect("resolve cache poisoned").take();
}
