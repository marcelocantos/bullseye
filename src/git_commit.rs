// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Auto-commit `bullseye.yaml` after a mutation (🎯T22).
//!
//! The project's invariants check ("✗ dirty tree" in `make bullseye`)
//! fires whenever `bullseye.yaml` carries uncommitted changes, which
//! blocks `bullseye_convergence` from making progress. Rather than
//! expecting the agent to commit by hand after every target update,
//! bullseye self-commits the file so the working tree stays clean
//! between mutations.
//!
//! Decision tree:
//!
//! - If the file's parent directory isn't inside a git repo, no-op.
//! - If `bullseye.yaml` has no uncommitted changes, no-op.
//! - If the most recent commit on `HEAD` is unpushed and its set of
//!   changed files is exactly `{bullseye.yaml}`, fold the new state
//!   into it via `git commit --amend --no-edit -- bullseye.yaml`
//!   (the existing message is preserved).
//! - Otherwise, create a new commit `Update bullseye.yaml` with just
//!   the dirty `bullseye.yaml` — any other staged changes are left
//!   alone.
//!
//! Failures (broken repo state, missing git binary, hooks rejecting
//! the commit) are logged to stderr but never propagated. Auto-commit
//! is best-effort: a failure leaves the file dirty so the user can
//! resolve it manually, the same outcome as before this feature.
//!
//! Every git invocation here is bounded (🎯T62). `git commit` runs the
//! repository's own `pre-commit` / `commit-msg` hooks and, with
//! `commit.gpgsign`, a `gpg` that may wait on a pinentry prompt that
//! will never arrive under an MCP server. Unbounded, any of those hangs
//! the calling tool forever with no response — `bullseye_convergence`
//! auto-commits before it does anything else, so a blocking hook hung
//! the whole convergence report. Bounded, the step gives up, kills the
//! hook's process group, and reports [`AutoCommitOutcome::TimedOut`] for
//! the caller to render.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::bounded::{BoundedError, bounded_output};

/// Wall-clock bound for `git commit`. Generous because the commit fires
/// the project's own hooks, which legitimately run linters or a quick
/// test pass; anything past two minutes is stuck, not slow. Matches the
/// invariants-hook bound in [`crate::convergence`] for the same reason.
const GIT_COMMIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Wall-clock bound for read-only git plumbing (`rev-parse`, `diff
/// --cached`, `rev-list`, `show`). These never run hooks, so seconds are
/// already pathological — but a repo on a stalled network mount or
/// behind a wedged fsmonitor daemon can still block indefinitely.
const GIT_PLUMBING_TIMEOUT: Duration = Duration::from_secs(30);

thread_local! {
    /// Thread-local `(commit, plumbing)` bound override, following the
    /// same pattern as [`crate::config`]'s test overrides. Production
    /// never sets it. Tests use it to drive the timeout path through
    /// callers that take no bounds of their own — `convergence`
    /// auto-commits before anything else, and exercising that end to end
    /// against the production bound would mean a two-minute test.
    static TIMEOUT_OVERRIDE: Cell<Option<(Duration, Duration)>> = const { Cell::new(None) };
}

/// Install a thread-local override for the `(commit, plumbing)` bounds
/// used by [`auto_commit_yaml`]; `None` restores the defaults. Tests
/// only — production code never calls this.
pub fn set_timeout_override(bounds: Option<(Duration, Duration)>) {
    TIMEOUT_OVERRIDE.with(|o| o.set(bounds));
}

/// What [`auto_commit_yaml`] did, so a caller that renders a report can
/// tell the user why the ledger file is still dirty.
#[derive(Debug, PartialEq, Eq)]
pub enum AutoCommitOutcome {
    /// Nothing to do: not a git repo, path outside the work tree, or the
    /// file had no uncommitted changes.
    NoOp,
    /// The file was committed (or folded into the previous commit).
    Committed,
    /// Git ran and refused (hook rejected, broken repo state, no git).
    Failed,
    /// A git step ran past its bound and was killed. The file is left
    /// dirty and the caller should say so out loud.
    TimedOut {
        /// The git subcommand that was killed, e.g. `commit`.
        step: &'static str,
        /// The bound that was exceeded, in seconds.
        secs: u64,
    },
}

/// Best-effort auto-commit of `path` (a `bullseye.yaml`). See module
/// docs for the full decision tree.
pub fn auto_commit_yaml(path: &Path) -> AutoCommitOutcome {
    let (commit, plumbing) = TIMEOUT_OVERRIDE
        .with(|o| o.get())
        .unwrap_or((GIT_COMMIT_TIMEOUT, GIT_PLUMBING_TIMEOUT));
    auto_commit_yaml_with_timeouts(path, commit, plumbing)
}

/// [`auto_commit_yaml`] with injectable bounds, so the timeout path can
/// be tested in milliseconds instead of minutes (🎯T62).
pub fn auto_commit_yaml_with_timeouts(
    path: &Path,
    commit_timeout: Duration,
    plumbing_timeout: Duration,
) -> AutoCommitOutcome {
    let Some(parent) = path.parent() else {
        return AutoCommitOutcome::NoOp;
    };

    let Some(repo_top) = git_top_level(parent, plumbing_timeout) else {
        return AutoCommitOutcome::NoOp;
    };

    // `git rev-parse --show-toplevel` returns the **canonical** path
    // (symlinks resolved), but the caller may have passed us a path
    // that still goes through symlinks — e.g. macOS tempdirs live at
    // `/var/folders/...` which symlinks to `/private/var/...`.
    // Canonicalise the input path first so `strip_prefix` against the
    // canonical repo top succeeds.
    let canonical_path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return AutoCommitOutcome::NoOp,
    };
    let pathspec = match canonical_path.strip_prefix(&repo_top) {
        Ok(p) => p.to_path_buf(),
        Err(_) => return AutoCommitOutcome::NoOp,
    };
    let Some(pathspec_str) = pathspec.to_str() else {
        return AutoCommitOutcome::NoOp;
    };

    match run_git(&repo_top, &["add", "--", pathspec_str], plumbing_timeout) {
        GitStep::Ok => {}
        GitStep::TimedOut { secs } => {
            return timed_out(path, "add", secs);
        }
        GitStep::Failed => return AutoCommitOutcome::Failed,
    }

    if !has_staged_changes(&repo_top, pathspec_str, plumbing_timeout) {
        return AutoCommitOutcome::NoOp;
    }

    let amend = last_commit_is_unpushed_and_only(&repo_top, pathspec_str, plumbing_timeout);
    let args: &[&str] = if amend {
        &["commit", "--amend", "--no-edit", "--"]
    } else {
        &["commit", "-m", "Update bullseye.yaml", "--"]
    };
    let mut argv = args.to_vec();
    argv.push(pathspec_str);

    match run_git(&repo_top, &argv, commit_timeout) {
        GitStep::Ok => AutoCommitOutcome::Committed,
        GitStep::TimedOut { secs } => timed_out(path, "commit", secs),
        GitStep::Failed => {
            eprintln!(
                "bullseye: auto-commit of {} failed (left dirty for manual resolution)",
                path.display()
            );
            AutoCommitOutcome::Failed
        }
    }
}

/// Log a killed git step and package it for the caller to render.
fn timed_out(path: &Path, step: &'static str, secs: u64) -> AutoCommitOutcome {
    eprintln!(
        "bullseye: auto-commit of {}: `git {step}` timed out after {secs}s and was killed \
         (left dirty for manual resolution)",
        path.display()
    );
    AutoCommitOutcome::TimedOut { step, secs }
}

/// Outcome of one bounded git invocation.
enum GitStep {
    Ok,
    Failed,
    TimedOut { secs: u64 },
}

/// Resolve the top-level directory of the git repo containing `dir`,
/// or `None` if `dir` isn't inside a working tree (or git didn't answer
/// within `timeout`).
fn git_top_level(dir: &Path, timeout: Duration) -> Option<PathBuf> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"]);
    let out = bounded_output(&mut cmd, timeout).ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// True iff the index has differences from `HEAD` for `pathspec`. A
/// fresh repo with no `HEAD` reports the just-added file as a change,
/// which is what we want — auto-commit creates the initial commit.
fn has_staged_changes(repo_top: &Path, pathspec: &str, timeout: Duration) -> bool {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo_top)
        .args(["diff", "--cached", "--name-only", "--", pathspec]);
    match bounded_output(&mut cmd, timeout) {
        Ok(o) if o.status.success() => !o.stdout.trim_ascii().is_empty(),
        _ => false,
    }
}

/// True iff `HEAD` is not reachable from any remote-tracking ref AND
/// the set of files touched by `HEAD` (vs its parent, or vs the empty
/// tree for a root commit) is exactly `[pathspec]`.
///
/// Returns false on:
/// - no remote configured (treats no-remote as "unpushed" — correct,
///   since there's nothing to push to),
/// - no `HEAD` (empty repo — there's no last commit to amend),
/// - any git command failure.
fn last_commit_is_unpushed_and_only(repo_top: &Path, pathspec: &str, timeout: Duration) -> bool {
    // List the most recent unpushed commit on HEAD. Empty stdout →
    // HEAD is reachable from at least one remote ref → pushed.
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo_top)
        .args(["rev-list", "-1", "HEAD", "--not", "--remotes"]);
    let out = match bounded_output(&mut cmd, timeout) {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };
    if out.stdout.trim_ascii().is_empty() {
        return false;
    }

    // Files touched by HEAD. `git show --pretty=format: --name-only`
    // works for both root and non-root commits: for a root commit it
    // lists the entire tree (no parent to diff against), which is the
    // right interpretation of "what's in this commit" for our test.
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo_top)
        .args(["show", "--pretty=format:", "--name-only", "HEAD"]);
    let out = match bounded_output(&mut cmd, timeout) {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };
    let stdout = String::from_utf8(out.stdout).unwrap_or_default();
    let files: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    files.len() == 1 && files[0] == pathspec
}

/// Run `git -C <repo_top> <args>` swallowing stdout/stderr, bounded at
/// `timeout`. Distinguishes a killed step from a plain failure so the
/// caller can report "still running when we gave up" rather than the
/// misleading "git refused".
fn run_git(repo_top: &Path, args: &[&str], timeout: Duration) -> GitStep {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo_top).args(args);
    match bounded_output(&mut cmd, timeout) {
        Ok(o) if o.status.success() => GitStep::Ok,
        Ok(o) => {
            eprintln!(
                "bullseye git_commit: `git {}` failed in {}: {}",
                args.join(" "),
                repo_top.display(),
                String::from_utf8_lossy(&o.stderr).trim(),
            );
            GitStep::Failed
        }
        Err(BoundedError::TimedOut { secs }) => GitStep::TimedOut { secs },
        Err(e) => {
            eprintln!("bullseye git_commit: `git {}`: {e}", args.join(" "));
            GitStep::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Initialise a git repo in `dir` with a stable identity so commits
    /// don't depend on the developer's git config.
    fn git_init(dir: &Path) {
        run_git_verbose(dir, &["init", "-q", "-b", "master"]);
        run_git_verbose(dir, &["config", "user.email", "test@example.com"]);
        run_git_verbose(dir, &["config", "user.name", "Test"]);
        // Point hook lookup at an empty directory so the developer's
        // global pre-commit/etc. hooks don't run during tests. Using
        // a dedicated empty subdir is more portable than `/dev/null`,
        // which on some platforms isn't a valid directory path.
        let empty = dir.join(".git/empty-hooks");
        std::fs::create_dir_all(&empty).unwrap();
        run_git_verbose(dir, &["config", "core.hooksPath", empty.to_str().unwrap()]);
    }

    /// Like `run_git`, but panics on failure with the captured stderr
    /// so test scaffolding errors are diagnosable. Output is captured
    /// (not inherited) so passing tests stay quiet.
    fn run_git_verbose(repo_top: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo_top)
            .args(args)
            .output()
            .expect("git invocation failed");
        if !out.status.success() {
            panic!(
                "git {args:?} failed in {repo_top:?}:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            );
        }
    }

    fn write(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
    }

    fn head_message(repo: &Path) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["log", "-1", "--pretty=%s"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn head_files(repo: &Path) -> Vec<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["show", "--pretty=format:", "--name-only", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    fn commit_count(repo: &Path) -> usize {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-list", "--count", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap_or(0)
    }

    #[test]
    fn no_op_outside_git_repo() {
        // tempdir is a plain directory — no git repo. Auto-commit must
        // not panic and must not create a repo as a side effect.
        let tmp = TempDir::new().unwrap();
        let yaml = tmp.path().join("bullseye.yaml");
        write(&yaml, "schema_version: 3\ntargets: {}\n");
        auto_commit_yaml(&yaml);
        assert!(!tmp.path().join(".git").exists());
    }

    #[test]
    fn no_op_when_yaml_clean() {
        // Repo with bullseye.yaml already committed and not modified.
        // Auto-commit must observe "no staged changes" and create no
        // new commit.
        let tmp = TempDir::new().unwrap();
        git_init(tmp.path());
        let yaml = tmp.path().join("bullseye.yaml");
        write(&yaml, "schema_version: 3\ntargets: {}\n");
        run_git_verbose(tmp.path(), &["add", "bullseye.yaml"]);
        run_git_verbose(tmp.path(), &["commit", "-m", "feat: add targets"]);
        let before = commit_count(tmp.path());
        auto_commit_yaml(&yaml);
        assert_eq!(commit_count(tmp.path()), before);
        assert_eq!(head_message(tmp.path()), "feat: add targets");
    }

    #[test]
    fn creates_initial_commit_when_yaml_is_first_file() {
        // Empty repo, just-written bullseye.yaml. Auto-commit creates
        // the initial commit.
        let tmp = TempDir::new().unwrap();
        git_init(tmp.path());
        let yaml = tmp.path().join("bullseye.yaml");
        write(&yaml, "schema_version: 3\ntargets: {}\n");
        auto_commit_yaml(&yaml);
        assert_eq!(commit_count(tmp.path()), 1);
        assert_eq!(head_message(tmp.path()), "Update bullseye.yaml");
        assert_eq!(head_files(tmp.path()), vec!["bullseye.yaml"]);
    }

    #[test]
    fn amends_when_last_commit_is_unpushed_yaml_only() {
        // Repo where HEAD is a yaml-only commit with no remote (so it
        // counts as unpushed). A second mutation should amend rather
        // than create a new commit.
        let tmp = TempDir::new().unwrap();
        git_init(tmp.path());
        let yaml = tmp.path().join("bullseye.yaml");
        write(&yaml, "schema_version: 3\ntargets: {}\n");
        run_git_verbose(tmp.path(), &["add", "bullseye.yaml"]);
        run_git_verbose(tmp.path(), &["commit", "-m", "feat: add targets"]);
        let before = commit_count(tmp.path());

        // Mutate again.
        write(
            &yaml,
            "schema_version: 3\ntargets:\n  T1: {name: x, kind: work, status: identified, value: 0, cost: 0, acceptance: [a], context: '', discovered: 2026-04-28}\n",
        );
        auto_commit_yaml(&yaml);

        assert_eq!(commit_count(tmp.path()), before, "amend, not new commit");
        // Original message preserved by --amend --no-edit.
        assert_eq!(head_message(tmp.path()), "feat: add targets");
    }

    #[test]
    fn creates_new_commit_when_last_commit_touched_other_files() {
        // HEAD touches more than just bullseye.yaml — the rule says
        // start a fresh commit rather than amending.
        let tmp = TempDir::new().unwrap();
        git_init(tmp.path());
        let yaml = tmp.path().join("bullseye.yaml");
        let other = tmp.path().join("README.md");
        write(&yaml, "schema_version: 3\ntargets: {}\n");
        write(&other, "# project\n");
        run_git_verbose(tmp.path(), &["add", "bullseye.yaml", "README.md"]);
        run_git_verbose(tmp.path(), &["commit", "-m", "init"]);
        let before = commit_count(tmp.path());

        write(&yaml, "schema_version: 3\ntargets: {x: {}}\n");
        auto_commit_yaml(&yaml);

        assert_eq!(commit_count(tmp.path()), before + 1);
        assert_eq!(head_message(tmp.path()), "Update bullseye.yaml");
        assert_eq!(head_files(tmp.path()), vec!["bullseye.yaml"]);
    }

    #[test]
    fn ignores_unrelated_staged_changes() {
        // The user has another file staged. Auto-commit must commit
        // only bullseye.yaml; the other staged file stays in the index.
        let tmp = TempDir::new().unwrap();
        git_init(tmp.path());
        let yaml = tmp.path().join("bullseye.yaml");
        let other = tmp.path().join("README.md");
        write(&yaml, "schema_version: 3\ntargets: {}\n");
        write(&other, "# project\n");
        run_git_verbose(tmp.path(), &["add", "bullseye.yaml"]);
        run_git_verbose(tmp.path(), &["commit", "-m", "feat: add targets"]);

        // User stages README.md; bullseye then mutates yaml.
        write(&other, "# project\n\nmore docs\n");
        run_git_verbose(tmp.path(), &["add", "README.md"]);
        write(&yaml, "schema_version: 3\ntargets: {y: {}}\n");
        auto_commit_yaml(&yaml);

        // Latest commit (an amend, since previous was yaml-only +
        // unpushed) contains only bullseye.yaml.
        assert_eq!(head_files(tmp.path()), vec!["bullseye.yaml"]);

        // README.md is still staged but uncommitted.
        let staged = Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["diff", "--cached", "--name-only"])
            .output()
            .unwrap();
        let staged_str = String::from_utf8(staged.stdout).unwrap();
        assert!(
            staged_str.lines().any(|l| l == "README.md"),
            "README.md must still be staged after auto-commit; got: {staged_str:?}",
        );
    }

    /// Install a `pre-commit` hook that blocks forever, so `git commit`
    /// hangs the way it does in a repo whose hook waits on a lock, a
    /// network mount, or a signing prompt that never arrives.
    #[cfg(unix)]
    fn install_blocking_pre_commit_hook(repo: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let hooks = repo.join(".git/blocking-hooks");
        fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        fs::write(&hook, "#!/bin/sh\nexec sleep 3600\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        run_git_verbose(repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    }

    #[test]
    #[cfg(unix)]
    fn blocking_pre_commit_hook_times_out_instead_of_hanging() {
        // Regression test for 🎯T62. `bullseye_convergence` auto-commits
        // before it does anything else, so an unbounded `git commit` here
        // meant a blocking project hook hung the whole tool with no
        // response at all — the worst failure mode for an MCP server.
        // The commit must be abandoned promptly and reported.
        let tmp = TempDir::new().unwrap();
        git_init(tmp.path());
        let yaml = tmp.path().join("bullseye.yaml");
        write(&yaml, "schema_version: 3\ntargets: {}\n");
        run_git_verbose(tmp.path(), &["add", "bullseye.yaml"]);
        run_git_verbose(tmp.path(), &["commit", "-m", "init"]);

        install_blocking_pre_commit_hook(tmp.path());
        write(&yaml, "schema_version: 3\ntargets: {t: {}}\n");

        let start = std::time::Instant::now();
        let outcome = auto_commit_yaml_with_timeouts(
            &yaml,
            Duration::from_millis(500),
            Duration::from_secs(30),
        );
        let elapsed = start.elapsed();

        assert_eq!(
            outcome,
            AutoCommitOutcome::TimedOut {
                step: "commit",
                secs: 0,
            },
            "a blocked commit must be reported as timed out, not as a plain failure",
        );
        assert!(
            elapsed < Duration::from_secs(20),
            "must return shortly after the 500ms bound, not wait out the hook's 3600s sleep; \
             took {elapsed:?}",
        );
        // The hook's `sleep` must not survive as an orphan holding the
        // repo — killing the whole process group is the point.
        assert!(
            !commit_is_in_progress(tmp.path()),
            "the killed commit must not leave the repo mid-commit",
        );
    }

    /// True while git considers a commit to be underway (its lock file
    /// is still present).
    fn commit_is_in_progress(repo: &Path) -> bool {
        repo.join(".git/index.lock").exists()
    }

    #[test]
    fn creates_new_commit_when_last_commit_is_pushed() {
        // Simulate a "pushed" HEAD by setting up a bare remote and
        // pushing the yaml-only commit. The next mutation must NOT
        // amend a pushed commit; it must create a fresh one.
        let tmp = TempDir::new().unwrap();
        let bare = TempDir::new().unwrap();
        run_git_verbose(bare.path(), &["init", "--bare", "-q", "-b", "master"]);

        git_init(tmp.path());
        let yaml = tmp.path().join("bullseye.yaml");
        write(&yaml, "schema_version: 3\ntargets: {}\n");
        run_git_verbose(tmp.path(), &["add", "bullseye.yaml"]);
        run_git_verbose(tmp.path(), &["commit", "-m", "feat: add targets"]);
        run_git_verbose(
            tmp.path(),
            &["remote", "add", "origin", bare.path().to_str().unwrap()],
        );
        run_git_verbose(tmp.path(), &["push", "-u", "origin", "master"]);

        let before = commit_count(tmp.path());
        write(&yaml, "schema_version: 3\ntargets: {z: {}}\n");
        auto_commit_yaml(&yaml);

        assert_eq!(
            commit_count(tmp.path()),
            before + 1,
            "pushed commit must not be amended"
        );
        assert_eq!(head_message(tmp.path()), "Update bullseye.yaml");
    }
}
