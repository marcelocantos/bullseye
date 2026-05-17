// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Refuse mutations in unsafe repo states (🎯T24).
//!
//! When `bullseye_put`/`_retire`/`_revert`/`_set_aside` (or any other
//! tool that calls [`crate::git_commit::auto_commit_yaml`]) runs from
//! a directory that turns out to be a *submodule* clone of the target
//! repo — or a working tree with detached HEAD — the auto-commit step
//! lands on a branch that nobody pushes. The user later finds the work
//! "lost" on a dangling local branch in a worktree they didn't realise
//! was being mutated.
//!
//! This module is the early-refusal step: every mutating handler calls
//! [`check_mutation_allowed`] before reading or writing the targets
//! file. If the repo is in either of the unsafe states, we return a
//! typed [`MutationGuard`] error that the handler converts to an MCP
//! error response. Read-only handlers (`list`, `frontier`, `get`,
//! `summary`, `graph`, `validate`, etc.) deliberately bypass this
//! check — agents doing research from inside a parent project's
//! submodule should still be able to see targets.
//!
//! The check is run against the *repo containing the discovered
//! `bullseye.yaml`*, NOT the agent's caller-supplied `cwd` (which may
//! be deeper). `git rev-parse --show-superproject-working-tree` walks
//! up from the given directory and reports a non-empty path when the
//! current repo is itself nested as a submodule of another.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Guard error: the repo containing `bullseye.yaml` is in a state
/// where auto-committing would silently lose work.
///
/// Render via [`MutationGuard::message`] for the actionable text the
/// agent (and human reading the error) sees.
#[derive(Debug, Clone)]
pub enum MutationGuard {
    /// The repo is a submodule of another project. Auto-commits land
    /// on a branch that's local to the submodule worktree, not the
    /// canonical clone the user expects.
    Submodule {
        /// Absolute path to the repo containing the discovered
        /// `bullseye.yaml`.
        repo_root: PathBuf,
        /// Absolute path of the parent project's working tree (the
        /// output of `git rev-parse --show-superproject-working-tree`).
        superproject: PathBuf,
        /// Suggested canonical clone location for the mutating call,
        /// derived from the repo's `origin` remote URL when available.
        /// `None` if the repo has no usable remote.
        suggested_path: Option<PathBuf>,
    },
    /// HEAD is detached (the repo is at a tag, an arbitrary SHA, or a
    /// dangling commit). Auto-committing creates a fresh local branch
    /// that goes nowhere.
    DetachedHead {
        /// Absolute path to the repo containing the discovered
        /// `bullseye.yaml`.
        repo_root: PathBuf,
        /// SHA of the current HEAD (short form when available, full
        /// SHA otherwise).
        current_sha: String,
    },
}

impl MutationGuard {
    /// Render the guard as an actionable error message naming the
    /// actual cwd, the offending state, and how to retry. The message
    /// is the wire-level error the agent sees, so wording matters —
    /// it must be specific enough that the next call goes somewhere
    /// useful.
    pub fn message(&self, cwd: &Path) -> String {
        match self {
            MutationGuard::Submodule {
                repo_root,
                superproject,
                suggested_path,
            } => {
                let mut msg = format!(
                    "bullseye refuses to mutate targets here: the repo at {repo} is a \
                     submodule of {sup}. Auto-committing the change would land on a \
                     branch local to this submodule worktree and never reach the \
                     canonical clone, so the user would later find the work \"lost\".\n\
                     \n\
                     cwd was: {cwd}\n\
                     submodule repo: {repo}\n\
                     superproject:   {sup}",
                    repo = repo_root.display(),
                    sup = superproject.display(),
                    cwd = cwd.display(),
                );
                if let Some(canonical) = suggested_path {
                    msg.push_str(&format!(
                        "\n\nRetry from the canonical clone, e.g.\n  \
                         cd {canonical} && <re-issue the mutation>\n\
                         (or pass that path as the `cwd` parameter).",
                        canonical = canonical.display(),
                    ));
                } else {
                    msg.push_str(
                        "\n\nThe repo has no `origin` remote configured, so bullseye \
                         can't suggest a canonical path. Locate the canonical clone \
                         manually and re-issue the mutation with its path as `cwd`.",
                    );
                }
                msg
            }
            MutationGuard::DetachedHead {
                repo_root,
                current_sha,
            } => format!(
                "bullseye refuses to mutate targets here: the repo at {repo} has \
                 detached HEAD (currently at {sha}). Auto-committing the change \
                 would create a dangling local branch with no upstream, so the work \
                 would be invisible to the rest of the project.\n\
                 \n\
                 cwd was: {cwd}\n\
                 repo:    {repo}\n\
                 \n\
                 Check out a named branch first (e.g. `git -C {repo} checkout master`, \
                 or create a new branch with `git -C {repo} switch -c <name>`) and \
                 re-issue the mutation.",
                repo = repo_root.display(),
                sha = current_sha,
                cwd = cwd.display(),
            ),
        }
    }
}

/// Refuse the mutation if the repo containing `bullseye.yaml` is in an
/// unsafe state for auto-committing.
///
/// `repo_root` is the directory containing the discovered
/// `bullseye.yaml` — i.e. the working tree that the auto-commit step
/// will operate on. NOT the caller's `cwd`, which may sit deeper in
/// the same tree (or in a different one entirely if the file was
/// resolved via the external shadow root).
///
/// Returns:
///
/// - `Ok(())` if the repo is on a named branch in a non-submodule
///   working tree — or if `repo_root` isn't inside a git repo at all
///   (e.g. shadow-root files have no auto-commit semantics in the
///   first place, so there's nothing to guard).
/// - `Err(MutationGuard::Submodule { .. })` if `git rev-parse
///   --show-superproject-working-tree` returns a non-empty path.
/// - `Err(MutationGuard::DetachedHead { .. })` if `git symbolic-ref -q
///   HEAD` exits non-zero (HEAD doesn't resolve to a named ref).
///
/// The submodule check is evaluated first; a submodule worktree with
/// detached HEAD reports as Submodule (the more actionable error: fix
/// the cwd, not the branch).
pub fn check_mutation_allowed(repo_root: &Path) -> Result<(), MutationGuard> {
    // No git repo at all → nothing to guard. The auto-commit step
    // already no-ops in this case (see [`crate::git_commit::auto_commit_yaml`]),
    // so refusing here would block valid external-shadow-tree usage.
    if !is_git_repo(repo_root) {
        return Ok(());
    }

    if let Some(superproject) = superproject_working_tree(repo_root) {
        return Err(MutationGuard::Submodule {
            repo_root: repo_root.to_path_buf(),
            superproject,
            suggested_path: suggested_canonical_path(repo_root),
        });
    }

    if is_detached_head(repo_root) {
        let current_sha = current_head_sha(repo_root).unwrap_or_else(|| "<unknown>".to_string());
        return Err(MutationGuard::DetachedHead {
            repo_root: repo_root.to_path_buf(),
            current_sha,
        });
    }

    Ok(())
}

/// True iff `dir` is inside a git working tree. `git rev-parse
/// --is-inside-work-tree` prints `true` and exits 0; any other shape
/// (no repo, bare repo, missing git binary) returns false.
fn is_git_repo(dir: &Path) -> bool {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8(o.stdout).unwrap_or_default();
            s.trim() == "true"
        }
        _ => false,
    }
}

/// Return the absolute path of the parent project's working tree if
/// `dir`'s repo is a submodule, or `None` if it isn't (or git can't
/// be invoked). Wraps `git rev-parse --show-superproject-working-tree`,
/// which prints the parent path on success and an empty string for
/// non-submodule repos.
fn superproject_working_tree(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-superproject-working-tree"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// True iff `HEAD` is detached. `git symbolic-ref -q HEAD` exits 0
/// with the ref name on a normal branch and exits non-zero on detached
/// HEAD; any spawn failure is treated as "not detached" (we'd rather
/// false-pass than refuse a mutation when git is broken).
fn is_detached_head(dir: &Path) -> bool {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["symbolic-ref", "-q", "HEAD"])
        .output();
    match out {
        Ok(o) => !o.status.success(),
        Err(_) => false,
    }
}

/// Best-effort short SHA of the current HEAD. Used purely for error
/// messages; failures fall back to the empty string at the call site.
fn current_head_sha(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Suggest a canonical clone path under `~/work/<host>/<owner>/<repo>/`
/// based on the repo's `origin` remote URL. Returns `None` if there's
/// no origin, the URL doesn't look like a recognised host/owner/repo
/// shape, or `$HOME` is unset.
///
/// Recognised URL shapes:
///
/// - `git@github.com:owner/repo.git`
/// - `git@github.com:owner/repo`
/// - `https://github.com/owner/repo.git`
/// - `https://github.com/owner/repo`
/// - same patterns with other hosts (gitlab.com, bitbucket.org, …)
fn suggested_canonical_path(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8(out.stdout).ok()?;
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    let (host, owner_repo) = parse_remote_url(url)?;
    let (owner, repo) = owner_repo.split_once('/')?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    let repo = repo.strip_suffix(".git").unwrap_or(repo);

    let home = std::env::var("HOME").ok()?;
    let mut path = PathBuf::from(home);
    path.push("work");
    path.push(host);
    path.push(owner);
    path.push(repo);
    Some(path)
}

/// Parse a git remote URL into `(host, "owner/repo")` so the canonical
/// path can be assembled under `~/work/`. Returns `None` for shapes we
/// don't recognise — callers fall back to omitting the suggestion.
fn parse_remote_url(url: &str) -> Option<(&str, &str)> {
    // SSH form: `git@host:owner/repo[.git]`.
    if let Some(rest) = url.strip_prefix("git@")
        && let Some((host, path)) = rest.split_once(':')
    {
        return Some((host, path));
    }

    // HTTPS / HTTP / git protocol: strip the scheme, then peel the
    // first component as host and the remainder as `owner/repo[.git]`.
    for scheme in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            // ssh URLs may carry `user@host/...` — drop the user part.
            let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
            if let Some((host, path)) = rest.split_once('/') {
                return Some((host, path));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Initialise a git repo with a stable identity so commits don't
    /// depend on the developer's `~/.gitconfig`. Mirrors the helper in
    /// `git_commit::tests::git_init` — kept local rather than shared so
    /// the modules stay independently testable.
    fn git_init(dir: &Path) {
        run_git(dir, &["init", "-q", "-b", "master"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test"]);
        run_git(dir, &["config", "commit.gpgsign", "false"]);
        // Empty hooks dir so the developer's global hooks don't fire.
        let empty = dir.join(".git/empty-hooks");
        std::fs::create_dir_all(&empty).unwrap();
        run_git(dir, &["config", "core.hooksPath", empty.to_str().unwrap()]);
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git invocation failed");
        if !out.status.success() {
            panic!(
                "git {args:?} failed in {dir:?}:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            );
        }
    }

    #[test]
    fn allows_mutation_in_clean_branch_repo() {
        let tmp = TempDir::new().unwrap();
        git_init(tmp.path());
        std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
        run_git(tmp.path(), &["add", "a.txt"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);

        check_mutation_allowed(tmp.path()).expect("clean repo on master should pass");
    }

    #[test]
    fn allows_mutation_outside_git_repo() {
        // No `.git` here at all. The auto-commit step itself no-ops in
        // this case, so the guard must not refuse — refusing would
        // break legitimate external-shadow-tree usage where the
        // bullseye.yaml lives outside any working tree.
        let tmp = TempDir::new().unwrap();
        check_mutation_allowed(tmp.path()).expect("non-git dir should pass");
    }

    #[test]
    fn refuses_mutation_in_detached_head() {
        let tmp = TempDir::new().unwrap();
        git_init(tmp.path());
        std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
        run_git(tmp.path(), &["add", "a.txt"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "first"]);
        // Detach HEAD by checking out the SHA directly.
        run_git(tmp.path(), &["checkout", "-q", "--detach", "HEAD"]);

        let err = check_mutation_allowed(tmp.path()).expect_err("detached HEAD must refuse");
        match err {
            MutationGuard::DetachedHead { current_sha, .. } => {
                assert!(!current_sha.is_empty(), "should report a SHA");
            }
            other => panic!("expected DetachedHead, got {other:?}"),
        }
    }

    #[test]
    fn parse_remote_url_ssh() {
        assert_eq!(
            parse_remote_url("git@github.com:marcelocantos/bullseye.git"),
            Some(("github.com", "marcelocantos/bullseye.git")),
        );
        assert_eq!(
            parse_remote_url("git@github.com:marcelocantos/bullseye"),
            Some(("github.com", "marcelocantos/bullseye")),
        );
    }

    #[test]
    fn parse_remote_url_https() {
        assert_eq!(
            parse_remote_url("https://github.com/marcelocantos/bullseye.git"),
            Some(("github.com", "marcelocantos/bullseye.git")),
        );
        assert_eq!(
            parse_remote_url("https://gitlab.com/group/project"),
            Some(("gitlab.com", "group/project")),
        );
    }

    #[test]
    fn parse_remote_url_unknown_returns_none() {
        assert_eq!(parse_remote_url("file:///some/path"), None);
        assert_eq!(parse_remote_url("not-a-url"), None);
        assert_eq!(parse_remote_url(""), None);
    }

    #[test]
    fn message_names_cwd_repo_and_canonical() {
        let cwd = Path::new("/tmp/cwd-stub");
        let guard = MutationGuard::Submodule {
            repo_root: PathBuf::from("/parent/sub/bullseye"),
            superproject: PathBuf::from("/parent"),
            suggested_path: Some(PathBuf::from("/Users/x/work/github.com/o/r")),
        };
        let msg = guard.message(cwd);
        assert!(msg.contains("/tmp/cwd-stub"), "should name cwd: {msg}");
        assert!(
            msg.contains("/parent/sub/bullseye"),
            "should name repo: {msg}"
        );
        assert!(msg.contains("/parent"), "should name superproject: {msg}");
        assert!(
            msg.contains("/Users/x/work/github.com/o/r"),
            "should name canonical path: {msg}",
        );
        assert!(msg.contains("submodule"), "should mention submodule: {msg}");
    }

    #[test]
    fn message_detached_head_names_branch_recovery() {
        let cwd = Path::new("/tmp/cwd-stub");
        let guard = MutationGuard::DetachedHead {
            repo_root: PathBuf::from("/some/repo"),
            current_sha: "deadbee".to_string(),
        };
        let msg = guard.message(cwd);
        assert!(msg.contains("detached"), "should mention detached: {msg}");
        assert!(msg.contains("deadbee"), "should name SHA: {msg}");
        assert!(
            msg.contains("checkout") || msg.contains("switch"),
            "should suggest a branch checkout: {msg}",
        );
    }
}
