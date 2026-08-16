// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Bounded subprocess execution (🎯T62).
//!
//! Every subprocess bullseye launches is external code whose runtime it
//! does not control: a project's `make bullseye` rule, a `git commit`
//! that fires the repo's `pre-commit` hook, a `git` read against a
//! network filesystem or a wedged fsmonitor daemon. `Command::output()`
//! waits forever, and a tool call that never returns is the worst
//! possible failure mode for an MCP server — the caller sees no result,
//! no error, and no reason, just a request that never answers.
//!
//! [`bounded_output`] is `Command::output()` with a wall-clock bound. On
//! expiry the child's whole process group is killed and the caller gets
//! [`BoundedError::TimedOut`] to report, rather than an indefinite hang.
//!
//! The bound is a parameter rather than a constant so callers can pick a
//! limit that matches the step (a hook that runs tests deserves minutes;
//! a `git rev-parse` does not) and so tests can drive a short one instead
//! of waiting out a production timeout.

use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Wall-clock bound for a read-only git query. Nothing here runs hooks
/// or touches the network, so seconds are already pathological — but a
/// repo on a stalled network mount, behind a wedged fsmonitor daemon, or
/// waiting on an `index.lock` held by another process can still block
/// forever, and bullseye reads git on nearly every code path.
pub const GIT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a bounded subprocess produced no [`Output`].
#[derive(Debug)]
pub enum BoundedError {
    /// The process could not be launched at all.
    Spawn(io::Error),
    /// The process ran past its bound and its process group was killed.
    TimedOut {
        /// The bound that was exceeded, in seconds.
        secs: u64,
    },
    /// The waiter thread vanished without reporting — should not happen,
    /// but it is reported rather than waited on forever.
    Disconnected,
}

impl std::fmt::Display for BoundedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundedError::Spawn(e) => write!(f, "failed to spawn: {e}"),
            BoundedError::TimedOut { secs } => {
                write!(f, "timed out after {secs}s and was killed")
            }
            BoundedError::Disconnected => write!(f, "subprocess runner exited unexpectedly"),
        }
    }
}

/// How long a process group gets to unwind after `SIGTERM` before it is
/// `SIGKILL`ed. Anything with a cleanup handler needs only milliseconds;
/// this is slack, not a wait.
const TERMINATION_GRACE: Duration = Duration::from_secs(2);

/// Signal the process group led by `pid`. Children are spawned with
/// `process_group(0)`, so `pid` is also the group id and `kill -<sig>
/// -<pid>` reaches the whole tree — a `go test` stuck on a DB connection
/// under `make`, or the `sleep` inside a `pre-commit` hook under `git
/// commit`, not just the direct child. Shelling out to `kill` keeps this
/// dependency-free; bullseye already shells to git. Best-effort and
/// Unix-only — elsewhere the bound still returns and the orphan is left
/// to the OS.
fn signal_process_group(pid: u32, sig: &str) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg(format!("-{sig}"))
            .arg(format!("-{pid}"))
            .status();
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, sig);
    }
}

/// Run `cmd` to completion, capturing stdout and stderr, giving up after
/// `timeout`.
///
/// stdout/stderr are piped (never inherited) so the child cannot write
/// into an MCP server's stdio transport, and stdin is null so a child
/// that reads input fails fast instead of blocking on a console that
/// isn't there.
pub fn bounded_output(cmd: &mut Command, timeout: Duration) -> Result<Output, BoundedError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Own process group so the whole tree can be signalled on expiry —
    // killing `git commit` alone would orphan the hook that is actually
    // stuck.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let child = cmd.spawn().map_err(BoundedError::Spawn)?;

    // With process_group(0) the child leads a new group whose pgid equals
    // its pid; capture it before the Child moves into the waiter thread.
    let pid = child.id();

    // wait_with_output drains both pipes and waits for exit; run it on a
    // thread so this thread can enforce the bound via recv_timeout.
    let (tx, rx) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(BoundedError::Spawn(e)),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // SIGTERM before SIGKILL. git, make and most hooks install
            // cleanup handlers, and a `git commit` that is killed outright
            // never runs its own — it leaves `.git/index.lock` behind,
            // wedging every later git call in the repo. Trading two
            // seconds for a repo the user can still use is worth it; a
            // process that ignores TERM still gets KILLed.
            signal_process_group(pid, "TERM");
            if rx.recv_timeout(TERMINATION_GRACE).is_err() {
                signal_process_group(pid, "KILL");
                // Let the waiter observe the death so callers that
                // inspect leftover files (git's index.lock) see a
                // settled tree rather than a process that has not
                // yet been reaped.
                let _ = rx.recv_timeout(Duration::from_millis(200));
            }
            // On Unix the signal lets the waiter's wait_with_output return
            // promptly; detach rather than join so a non-Unix best-effort
            // kill can't reintroduce the very hang this guards against.
            drop(waiter);
            Err(BoundedError::TimedOut {
                secs: timeout.as_secs(),
            })
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(BoundedError::Disconnected),
    }
}

/// Run `git -C <dir> <args>` bounded at `timeout`, returning stdout when
/// git exits 0 and `None` for every other outcome — not a repo, no git
/// binary, non-zero exit, or a git that ran past the bound and was
/// killed.
///
/// Callers of this helper all read git to *enrich* a response (unreleased
/// fixes, superproject detection, ID history) and already degrade to a
/// safe default when git can't answer, so a bound folds naturally into
/// their existing `None` branch. The kill is logged so a section that
/// silently went empty is still attributable to a wedged git rather than
/// to an empty repo.
pub fn git_query(dir: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    match bounded_output(&mut cmd, timeout) {
        Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(_) => None,
        Err(e @ BoundedError::TimedOut { .. }) => {
            eprintln!(
                "bullseye: `git {}` in {} {e}",
                args.join(" "),
                dir.display(),
            );
            None
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn fast_command_returns_its_output() {
        let mut cmd = Command::new("echo");
        cmd.arg("hello");
        let out = bounded_output(&mut cmd, Duration::from_secs(30)).expect("echo must succeed");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }

    #[test]
    fn blocking_command_is_killed_at_the_bound() {
        // A command that would run far past the bound must be killed and
        // reported promptly, not waited out.
        let mut cmd = Command::new("sleep");
        cmd.arg("3600");
        let start = Instant::now();
        let err = bounded_output(&mut cmd, Duration::from_millis(300))
            .expect_err("sleep 3600 must not complete within 300ms");
        let elapsed = start.elapsed();
        assert!(
            matches!(err, BoundedError::TimedOut { secs: 0 }),
            "expected TimedOut, got: {err:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "must return shortly after the 300ms bound, not wait out the 3600s sleep; took {elapsed:?}"
        );
    }

    #[test]
    fn spawn_failure_is_reported_not_panicked() {
        let mut cmd = Command::new("bullseye-no-such-binary-exists");
        let err = bounded_output(&mut cmd, Duration::from_secs(5))
            .expect_err("missing binary must not succeed");
        assert!(
            matches!(err, BoundedError::Spawn(_)),
            "expected Spawn, got: {err:?}"
        );
    }
}
