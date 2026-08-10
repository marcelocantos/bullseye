// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! End-to-end convergence evaluation for a project.
//!
//! Bullseye's primary role is target management; convergence is the
//! question *"given the current target state and the current health of
//! the project, what's the next most-valuable thing to work on?"*. The
//! old `/cv` skill answered this by orchestrating many individual tool
//! calls: git for standing invariants, `bullseye_summary` for targets,
//! `bullseye_get` per frontier target for details, `mnemo_recent_activity`
//! for momentum, commit log parsing for unreleased fixes, and finally
//! LLM-driven prose to stitch it all together.
//!
//! `bullseye_convergence` collapses all of that into a single tool call.
//! It:
//!
//!   1. Runs the project's own invariants script (`make bullseye` or
//!      `mk bullseye`) and captures the output.
//!   2. Inspects local git for commits since the last tag and flags the
//!      ones with fix markers.
//!   3. Emits the same summary `bullseye_summary` produces, with full
//!      frontier details inline (so the caller doesn't need a
//!      `bullseye_get` loop).
//!   4. Computes a deterministic recommendation — "execute now" vs
//!      "blocked" — that the calling skill can act on directly without
//!      needing any LLM reasoning at the skill layer.
//!
//! The project must have a `bullseye` rule in its Makefile or mkfile.
//! Missing rule → instructive setup response, not a tool-call error.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::git_commit;
use crate::graph;
use crate::schema::TargetsFile;

/// Result of a convergence run, rendered to a single string response
/// ready to ship to an MCP caller.
pub fn convergence(
    file: &TargetsFile,
    file_path: &Path,
    repo_root: &Path,
    momentum: Option<&BTreeMap<String, f64>>,
    skip_invariants: bool,
) -> String {
    let mut out = String::new();

    // Header: one File: line, one Total: line, both attributable to
    // the convergence response rather than the embedded summary. The
    // inner summary's own "# Summary / File: / Total:" header is
    // trimmed below so we don't duplicate.
    let active_count = file.active().len();
    let achieved_count = file.achieved().len();
    out.push_str(&format!(
        "# Convergence\n\
         File: {}\n\
         Total: {} target(s) — {} active, {} achieved\n\n",
        file_path.display(),
        file.targets.len(),
        active_count,
        achieved_count,
    ));

    // --- 0. Auto-commit a dirty bullseye.yaml (🎯T22) ---
    //
    // If the file was edited externally (or any earlier mutation in
    // this process didn't reach the post-save hook), fold the change
    // into git before invariants run. Otherwise, the project's
    // `make bullseye` dirty-tree check would always block convergence
    // on a freshly-mutated yaml. Best-effort: a no-git or hook-rejected
    // case leaves the file dirty and the invariants block fires as
    // before.
    git_commit::auto_commit_yaml(file_path);

    // --- 1. Invariants (project-supplied hook) ---
    //
    // Three paths:
    //   - skip_invariants=true → render "(skipped)" marker, no subprocess
    //   - hook present → run it, render stdout + status
    //   - hook missing → render setup instructions inline (see note below)
    //
    // Crucially, a missing hook does NOT abort convergence. The agent
    // still gets the full target snapshot and a frontier-based next
    // action, just with invariants marked as "unknown — please set up
    // the hook". This preserves the degraded-but-useful contract for
    // session-start / speculative callers.
    let invariants_result = if skip_invariants {
        out.push_str("## Invariants\n\n(skipped — skip_invariants=true)\n\n");
        InvariantsResult::Skipped
    } else {
        match detect_invariants_command(repo_root) {
            Ok(argv) => run_invariants_command(&argv, repo_root, &mut out),
            Err(reason) => {
                render_setup_warning(&mut out, &reason);
                InvariantsResult::HookMissing
            }
        }
    };

    // --- 2. Unreleased fixes (git-based) ---
    // Partition into surface-touching vs surface-external when the
    // project declares `release_surface` (🎯T42). Absent declaration
    // → all fix commits are ship-relevant (legacy behaviour).
    let all_unreleased = detect_unreleased_fixes(repo_root);
    let (ship_relevant, surface_external) =
        partition_by_release_surface(repo_root, &all_unreleased, &file.release_surface);
    render_unreleased(&mut out, &ship_relevant, &surface_external);
    let release_freeze = detect_release_freeze(repo_root);
    let release_policy = detect_release_policy(repo_root);

    // --- 3. Summary body (reuse graph::summary with frontier detail on) ---
    //
    // graph::summary emits its own "# Summary / File: / Total:"
    // header, but we already printed the equivalent at the top of
    // this response. Strip everything before the first `## ` heading
    // so the embedded sections flow cleanly into the convergence
    // output.
    let summary_out = graph::summary(file, &file_path.display().to_string(), momentum, true);
    let body_start = summary_out.find("## ").unwrap_or(0);
    out.push_str(&summary_out[body_start..]);

    // --- 3.5 Stylistic warnings (non-blocking) ---
    //
    // Validation warnings — currently just non-conforming target IDs —
    // are surfaced for visibility but never gate the next-action
    // recommendation. The user can still operate on the offending
    // target via bullseye_put / bullseye_retire / bullseye_set_aside;
    // this section just makes sure the warning isn't silently swept
    // under the carpet.
    let warnings = graph::validate_warnings(file);
    if !warnings.is_empty() {
        out.push_str("## Validation warnings\n\n");
        for w in &warnings {
            out.push_str(&format!("- {w}\n"));
        }
        out.push('\n');
    }

    // --- 4. Next action ---
    render_next_action(
        &mut out,
        &invariants_result,
        &ship_relevant,
        release_freeze.as_deref(),
        release_policy,
        file,
        file_path,
        momentum,
    );

    out
}

/// Render the Invariants section for a project that has no usable
/// `bullseye` rule in its build file. Embedded in the full convergence
/// output, not a standalone response — the caller still gets target
/// data and a frontier recommendation, this is just the Invariants
/// slot explaining that standing checks couldn't be verified.
fn render_setup_warning(out: &mut String, reason: &SetupReason) {
    let example = MAKE_BULLSEYE_EXAMPLE;
    out.push_str("## Invariants\n\n");
    match reason {
        SetupReason::NoBuildFile => {
            out.push_str(
                "⚠ **Standing-invariants hook not configured.** \
                 bullseye_convergence looks for a `bullseye` target in \
                 `Makefile` or `mkfile`; neither was found under this \
                 project. Invariants are marked **unknown** below — the \
                 frontier recommendation still fires, but you're \
                 running without a safety net.\n\n\
                 **Fix**: create a `Makefile` with a `bullseye:` rule \
                 that runs your project's standing-invariant checks. \
                 Example for a Rust project:\n\n",
            );
        }
        SetupReason::MissingRule { build_file } => {
            out.push_str(&format!(
                "⚠ **Standing-invariants hook not configured.** \
                 bullseye_convergence found `{build_file}` but it has no \
                 `bullseye` target. Invariants are marked **unknown** \
                 below — the frontier recommendation still fires, but \
                 you're running without a safety net.\n\n\
                 **Fix**: add a `bullseye:` rule to `{build_file}` that \
                 runs your project's standing-invariant checks. Example \
                 for a Rust project:\n\n",
            ));
        }
    }
    out.push_str(&format!("```make\n{example}\n```\n\n"));
    out.push_str(
        "The rule's exit code is the signal bullseye needs: 0 means \
         all invariants green, non-zero means at least one violation. \
         Stdout is relayed verbatim to the agent; format it however \
         you like.\n\n\
         Status: ⚠ unknown (hook not configured)\n\n",
    );
}

/// Why the project can't run the standing-invariants hook.
#[derive(Debug)]
pub enum SetupReason {
    NoBuildFile,
    MissingRule { build_file: String },
}

/// Return the command to invoke the project's standing-invariants
/// hook, based purely on which build file is present. Does NOT parse
/// the build file to pre-verify that a `bullseye` rule exists — that
/// check happens downstream, by observing the build tool's own output
/// when the command actually runs. Trying to second-guess Make's rule
/// table by hand is brittle (`.PHONY: bullseye` is not a rule, tab vs
/// space matters, `include` directives are ignored, etc.). The build
/// tool is the authority on what rules it has; we just run it and
/// cascade from the result.
///
/// Returns `Err(SetupReason::NoBuildFile)` only when neither a
/// `Makefile` nor an `mkfile` exists at the repo root. A build file
/// that lacks a `bullseye` rule still gets `Ok(argv)` — the
/// "no such rule" case is detected in [`run_invariants_command`] by
/// pattern-matching the build tool's stderr.
pub fn detect_invariants_command(repo_root: &Path) -> Result<Vec<String>, SetupReason> {
    // Prefer mkfile when both are present: if the project went to the
    // trouble of using mk, that's the canonical build tool for it.
    if repo_root.join("mkfile").is_file() {
        return Ok(vec!["mk".to_string(), "bullseye".to_string()]);
    }
    if repo_root.join("Makefile").is_file() {
        return Ok(vec!["make".to_string(), "bullseye".to_string()]);
    }
    Err(SetupReason::NoBuildFile)
}

/// Detect the "rule not found" signature in a build tool's stderr.
/// Used as a fallback classification inside [`run_invariants_command`]
/// when the hook exits non-zero — we need to distinguish "project has
/// no `bullseye` rule" (hook not configured; non-blocking) from "the
/// `bullseye` rule ran and actual invariants failed" (blocking).
///
/// The signatures checked cover the three build tools bullseye
/// supports: GNU make, BSD make, and marcelocantos/mk. All three emit
/// English error messages; we match a few stable substrings rather
/// than exact strings so translations or minor phrasing drift don't
/// break the cascade. A false positive here (classifying an actual
/// build failure as "hook missing") is theoretically possible but
/// unlikely — the phrases we match don't appear in normal rule output.
fn stderr_indicates_missing_rule(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    // GNU make: "make: *** No rule to make target 'bullseye'.  Stop."
    lower.contains("no rule to make target")
        // BSD make: "make: don't know how to make bullseye. Stop"
        || lower.contains("don't know how to make")
        // marcelocantos/mk: `mk: no rule to build "bullseye"`
        // (verified against mk 2026-04-12 at
        // github.com/marcelocantos/mk, graph.go:566).
        || lower.contains("no rule to build")
}

/// Outcome of running the `make bullseye` / `mk bullseye` hook.
#[derive(Debug)]
pub enum InvariantsResult {
    /// Hook exited 0. Standing invariants are green.
    Green,
    /// Hook exited non-zero. Violations present; block on them.
    Violated { exit_code: i32 },
    /// Hook was skipped at the caller's request.
    Skipped,
    /// No hook configured — project doesn't have a `bullseye` rule
    /// in Makefile/mkfile. Treated like `Skipped` for next-action
    /// purposes (the frontier recommendation still fires), but the
    /// Invariants section carries a prominent warning with setup
    /// instructions.
    HookMissing,
    /// Hook couldn't even be launched (subprocess error).
    SpawnFailed { error: String },
    /// Hook ran past [`INVARIANTS_HOOK_TIMEOUT`] and was killed. Treated
    /// like `HookMissing` for next-action purposes (the frontier
    /// recommendation still fires), but the Invariants section carries a
    /// prominent "timed out — invariants unknown" warning. Guards against
    /// a project hook that never returns (a test blocking on input,
    /// network, or DB auth) hanging the tool — and the agent — forever.
    /// See 🎯T38.
    TimedOut { secs: u64 },
}

/// Maximum wall-clock time the standing-invariants hook may run before
/// bullseye gives up, kills it, and treats invariants as unknown. A
/// project's `bullseye` rule is meant to be a quick gate; anything past
/// two minutes is almost certainly stuck (a test blocking on
/// input/network/DB), and hanging the agent indefinitely is never the
/// right answer. See 🎯T38.
const INVARIANTS_HOOK_TIMEOUT: Duration = Duration::from_secs(120);

fn run_invariants_command(argv: &[String], repo_root: &Path, out: &mut String) -> InvariantsResult {
    run_invariants_command_with_timeout(argv, repo_root, out, INVARIANTS_HOOK_TIMEOUT)
}

/// SIGKILL the process group led by `pid`. The hook is spawned with
/// `process_group(0)`, so `pid` is also the group id and `kill -KILL
/// -<pid>` reaps the whole tree (e.g. a `go test` child stuck on a DB
/// connection, not just the `make` parent). Shelling out to `kill` keeps
/// this dependency-free — bullseye already shells to git/gh. Best-effort
/// and Unix-only; elsewhere the timeout still returns and the orphan is
/// left to the OS.
fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .status();
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

/// Run the invariants hook, bounding it at `timeout`. On expiry the
/// hook's whole process group is killed and a
/// [`InvariantsResult::TimedOut`] is returned instead of blocking
/// forever. Split out from [`run_invariants_command`] so tests can drive
/// a short timeout. See 🎯T38.
fn run_invariants_command_with_timeout(
    argv: &[String],
    repo_root: &Path,
    out: &mut String,
    timeout: Duration,
) -> InvariantsResult {
    use std::sync::mpsc;

    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .current_dir(repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Own process group so the whole tree can be signalled on timeout —
    // killing `make` alone would orphan the child that's actually stuck.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            out.push_str(&format!(
                "## Invariants\n\n⚠ failed to run `{}`: {e}\n\n",
                argv.join(" "),
            ));
            return InvariantsResult::SpawnFailed {
                error: e.to_string(),
            };
        }
    };

    // With process_group(0) the child leads a new group whose pgid equals
    // its pid; capture it before the Child moves into the waiter thread.
    let pid = child.id();

    // wait_with_output drains both pipes and waits for exit; run it on a
    // thread so the main thread can enforce the timeout via recv_timeout.
    let (tx, rx) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    let output = match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            out.push_str(&format!(
                "## Invariants\n\n⚠ failed to run `{}`: {e}\n\n",
                argv.join(" "),
            ));
            return InvariantsResult::SpawnFailed {
                error: e.to_string(),
            };
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            kill_process_group(pid);
            // On Unix the kill lets the waiter's wait_with_output return
            // promptly; detach rather than join so a non-Unix best-effort
            // kill can't reintroduce a hang.
            drop(waiter);
            let secs = timeout.as_secs();
            out.push_str(&format!(
                "## Invariants\n\n⚠ invariants hook `{}` timed out after {secs}s and was \
                 killed — invariants are **unknown** for this run.\n\n\
                 Status: ⚠ unknown (hook timed out)\n\n",
                argv.join(" "),
            ));
            return InvariantsResult::TimedOut { secs };
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            out.push_str("## Invariants\n\n⚠ invariants hook runner exited unexpectedly.\n\n");
            return InvariantsResult::SpawnFailed {
                error: "invariants waiter thread disconnected".to_string(),
            };
        }
    };

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Cascade: if the build tool reported "no such rule", classify as
    // HookMissing and render the setup warning instead of the normal
    // invariants block. This is the authoritative check — the build
    // tool knows its own rule table, and we trust its output rather
    // than pre-parsing the Makefile ourselves.
    if exit_code != 0 && stderr_indicates_missing_rule(&stderr) {
        let build_file = if argv[0] == "mk" {
            "mkfile"
        } else {
            "Makefile"
        };
        render_setup_warning(
            out,
            &SetupReason::MissingRule {
                build_file: build_file.to_string(),
            },
        );
        return InvariantsResult::HookMissing;
    }

    out.push_str("## Invariants\n\n");
    out.push_str(&format!("```\n$ {}\n", argv.join(" ")));
    out.push_str(&colorize_status_glyphs(stdout.trim_end()));
    if !stdout.is_empty() && !stderr.is_empty() {
        out.push('\n');
    }
    if !stderr.is_empty() {
        out.push_str(&colorize_status_glyphs(stderr.trim_end()));
    }
    out.push_str("\n```\n\n");

    if exit_code == 0 {
        out.push_str("Status: ✅ all green\n\n");
        InvariantsResult::Green
    } else {
        out.push_str(&format!("Status: ❌ failed (exit {exit_code})\n\n"));
        InvariantsResult::Violated { exit_code }
    }
}

/// Replaces the ✓/✗ glyphs with their already-coloured emoji
/// counterparts (✅/❌) so pass/fail status pops visually in any
/// markdown renderer — ANSI escapes don't survive code fences in
/// most terminal renderers, but emoji do.
fn colorize_status_glyphs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '✓' => out.push('✅'),
            '✗' => out.push('❌'),
            _ => out.push(ch),
        }
    }
    out
}

/// A single unreleased fix commit.
#[derive(Debug, Clone)]
pub struct UnreleasedFix {
    pub hash: String,
    pub subject: String,
}

/// How unreleased fixes should influence next-action (🎯T46).
///
/// Loaded from an external profile template (`profile:` in AGENTS.md /
/// CLAUDE.md → YAML under `$BULLSEYE_PROFILES_DIR` or `~/.claude/gates/`).
/// Bullseye never branches on product-flavor names in code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnreleasedFixesPolicy {
    /// Recommend `/release` when ship-relevant unreleased fixes exist.
    #[default]
    RecommendShip,
    /// List fixes but recommend frontier work instead of `/release`.
    Informational,
}

/// Resolved release policy from a profile template.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReleasePolicy {
    pub unreleased_fixes: UnreleasedFixesPolicy,
    /// Optional delivery channel for wording (`tag`, `store`, `package`, `none`).
    pub channel: Option<String>,
    /// Profile name that produced this policy (for Next-action notes).
    pub profile: Option<String>,
}

/// Query local git for commits since the latest tag whose subject
/// matches a fix marker. Returns an empty vec when the project has no
/// tags, is up to date, or `git` isn't available — convergence should
/// degrade quietly, not blow up.
pub fn detect_unreleased_fixes(repo_root: &Path) -> Vec<UnreleasedFix> {
    // Latest tag. If there are no tags yet, everything is technically
    // "unreleased", but we don't want to flag a pre-release project
    // as having unreleased fixes — there's nothing to ship against.
    let tag_out = Command::new("git")
        .args(["describe", "--tags", "--abbrev=0"])
        .current_dir(repo_root)
        .output();
    let Ok(tag_out) = tag_out else {
        return Vec::new();
    };
    if !tag_out.status.success() {
        return Vec::new();
    }
    let tag = String::from_utf8_lossy(&tag_out.stdout).trim().to_string();
    if tag.is_empty() {
        return Vec::new();
    }

    let log_out = Command::new("git")
        .args(["log", "--oneline", "--no-merges", &format!("{tag}..HEAD")])
        .current_dir(repo_root)
        .output();
    let Ok(log_out) = log_out else {
        return Vec::new();
    };
    if !log_out.status.success() {
        return Vec::new();
    }
    let log = String::from_utf8_lossy(&log_out.stdout);
    log.lines()
        .filter_map(parse_oneline)
        .filter(|f| is_fix_commit(&f.subject))
        .collect()
}

/// Detect a project-level release freeze directive in agent instruction files.
///
/// `repo_root` may be a subdirectory inside the git repository (for example,
/// HMS runs /cv from `hms2/` while the canonical `AGENTS.md` lives one level
/// higher), so inspect both `repo_root` and the git top-level when available.
/// Missing files and unreadable git metadata degrade to "no freeze" so
/// convergence remains useful in partially configured repos.
pub fn detect_release_freeze(repo_root: &Path) -> Option<String> {
    let mut roots = vec![repo_root.to_path_buf()];
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(repo_root)
        .output()
        && output.status.success()
    {
        let git_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !git_root.is_empty() {
            let git_root = Path::new(&git_root).to_path_buf();
            if !roots.iter().any(|r| r == &git_root) {
                roots.push(git_root);
            }
        }
    }

    for root in roots {
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let path = root.join(name);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(reason) = parse_release_freeze(&text) {
                return Some(format!("{name}: {reason}"));
            }
        }
    }
    None
}

fn parse_release_freeze(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("release_freeze:") else {
            continue;
        };
        let reason = rest.trim().trim_matches('"').trim_matches('\'').trim();
        if reason.is_empty() {
            return Some("release freeze is declared".to_string());
        }
        return Some(reason.to_string());
    }
    None
}

fn parse_oneline(line: &str) -> Option<UnreleasedFix> {
    let (hash, subject) = line.split_once(' ')?;
    Some(UnreleasedFix {
        hash: hash.to_string(),
        subject: subject.to_string(),
    })
}

/// Does the commit subject look like a bug fix?
pub fn is_fix_commit(subject: &str) -> bool {
    let lower = subject.to_lowercase();
    const MARKERS: &[&str] = &[
        "fix:",
        "fix(",
        "fix ",
        "fixes ",
        "fixed ",
        "bugfix",
        "hotfix",
        "revert",
        "regression",
        "crash",
        "incorrect",
        "broken",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

/// Split unreleased fix commits into those that touch the declared
/// shipped surface vs those that do not (🎯T42).
///
/// When `release_surface` is empty, every fix is ship-relevant (legacy).
/// Matching is prefix-based against paths from `git show --name-only --pretty=format:`.
pub fn partition_by_release_surface(
    repo_root: &Path,
    fixes: &[UnreleasedFix],
    release_surface: &[String],
) -> (Vec<UnreleasedFix>, Vec<UnreleasedFix>) {
    if release_surface.is_empty() {
        return (fixes.to_vec(), Vec::new());
    }
    let prefixes: Vec<String> = release_surface
        .iter()
        .map(|s| s.trim().trim_start_matches("./").to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if prefixes.is_empty() {
        return (fixes.to_vec(), Vec::new());
    }

    let mut ship = Vec::new();
    let mut external = Vec::new();
    for f in fixes {
        if commit_touches_surface(repo_root, &f.hash, &prefixes) {
            ship.push(f.clone());
        } else {
            external.push(f.clone());
        }
    }
    (ship, external)
}

/// True when any path in the commit matches a release-surface prefix.
pub fn commit_touches_surface(repo_root: &Path, hash: &str, prefixes: &[String]) -> bool {
    let out = Command::new("git")
        .args(["show", "--name-only", "--pretty=format:", hash])
        .current_dir(repo_root)
        .output();
    let Ok(out) = out else {
        // Fail open: if we can't inspect the commit, treat as surface-touching
        // so we don't silently drop a real fix recommendation.
        return true;
    };
    if !out.status.success() {
        return true;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let path = line.trim().trim_start_matches("./");
        if path.is_empty() {
            continue;
        }
        for prefix in prefixes {
            let p = prefix.trim_end_matches('/');
            if path == p || path.starts_with(&format!("{p}/")) || path.starts_with(prefix.as_str())
            {
                return true;
            }
        }
    }
    false
}

fn render_unreleased(
    out: &mut String,
    ship_relevant: &[UnreleasedFix],
    surface_external: &[UnreleasedFix],
) {
    out.push_str("## Unreleased fixes\n\n");
    if ship_relevant.is_empty() && surface_external.is_empty() {
        out.push_str("(none — everything on master is released)\n\n");
        return;
    }
    for f in ship_relevant {
        out.push_str(&format!("- `{}` {}\n", f.hash, f.subject));
    }
    if !surface_external.is_empty() {
        out.push_str("\nNot user-visible (outside declared `release_surface`):\n");
        for f in surface_external {
            out.push_str(&format!(
                "- `{}` {} — not user-visible\n",
                f.hash, f.subject
            ));
        }
    }
    out.push('\n');
}

/// Resolve `profile:` from AGENTS.md / CLAUDE.md and load release policy
/// from the matching profile template YAML (🎯T46).
///
/// Search path for templates:
/// 1. `$BULLSEYE_PROFILES_DIR/<profile>.yaml` (if set)
/// 2. `~/.claude/gates/<profile>.yaml`
///
/// Missing profile / missing file / missing `release:` block → default
/// [`UnreleasedFixesPolicy::RecommendShip`]. Never hard-codes flavor names.
pub fn detect_release_policy(repo_root: &Path) -> ReleasePolicy {
    let profile = detect_profile_name(repo_root);
    let Some(profile) = profile else {
        return ReleasePolicy::default();
    };
    let path = profile_template_path(&profile);
    let Some(path) = path else {
        return ReleasePolicy {
            profile: Some(profile),
            ..ReleasePolicy::default()
        };
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ReleasePolicy {
            profile: Some(profile),
            ..ReleasePolicy::default()
        };
    };
    parse_release_policy_yaml(&text, &profile)
}

/// Profile name from the first `profile: <name>` line in AGENTS.md/CLAUDE.md
/// under repo_root or git top-level.
pub fn detect_profile_name(repo_root: &Path) -> Option<String> {
    let mut roots = vec![repo_root.to_path_buf()];
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(repo_root)
        .output()
        && output.status.success()
    {
        let git_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !git_root.is_empty() {
            let git_root = Path::new(&git_root).to_path_buf();
            if !roots.iter().any(|r| r == &git_root) {
                roots.push(git_root);
            }
        }
    }
    for root in roots {
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let path = root.join(name);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(p) = parse_profile_line(&text) {
                return Some(p);
            }
        }
    }
    None
}

fn parse_profile_line(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("profile:") else {
            continue;
        };
        let name = rest.trim().trim_matches('"').trim_matches('\'').trim();
        if name.is_empty() {
            continue;
        }
        // Ignore YAML-like nested keys; profile names are simple tokens.
        if name.contains(':') {
            continue;
        }
        return Some(name.to_string());
    }
    None
}

fn profile_template_path(profile: &str) -> Option<std::path::PathBuf> {
    // Sanitise: only allow simple profile names (no path traversal).
    if profile.is_empty()
        || profile.contains('/')
        || profile.contains('\\')
        || profile.contains("..")
    {
        return None;
    }
    let file = format!("{profile}.yaml");
    if let Ok(dir) = std::env::var("BULLSEYE_PROFILES_DIR") {
        let p = Path::new(&dir).join(&file);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = Path::new(&home).join(".claude/gates").join(&file);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Parse the `release:` block from a profile template.
///
/// Minimal YAML subset (no full parser dependency): looks for lines under
/// a top-level `release:` key:
/// ```yaml
/// release:
///   unreleased_fixes: informational   # or recommend_ship
///   channel: store                    # optional
/// ```
pub fn parse_release_policy_yaml(text: &str, profile: &str) -> ReleasePolicy {
    let mut in_release = false;
    let mut policy = ReleasePolicy {
        profile: Some(profile.to_string()),
        ..ReleasePolicy::default()
    };
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }
        // Leaving the release block: a non-indented key that is not `release:`.
        if in_release
            && !line.starts_with(' ')
            && !line.starts_with('\t')
            && !trimmed.starts_with("release:")
        {
            in_release = false;
        }
        if trimmed.starts_with("release:") {
            in_release = true;
            continue;
        }
        if !in_release {
            continue;
        }
        let content = trimmed.trim_start();
        if let Some(rest) = content.strip_prefix("unreleased_fixes:") {
            let v = rest.trim().trim_matches('"').trim_matches('\'').trim();
            policy.unreleased_fixes = match v {
                "informational" => UnreleasedFixesPolicy::Informational,
                "recommend_ship" | "" => UnreleasedFixesPolicy::RecommendShip,
                _ => UnreleasedFixesPolicy::RecommendShip,
            };
        } else if let Some(rest) = content.strip_prefix("channel:") {
            let v = rest.trim().trim_matches('"').trim_matches('\'').trim();
            if !v.is_empty() {
                policy.channel = Some(v.to_string());
            }
        }
    }
    policy
}

#[allow(clippy::too_many_arguments)]
fn render_next_action(
    out: &mut String,
    invariants: &InvariantsResult,
    unreleased: &[UnreleasedFix],
    release_freeze: Option<&str>,
    release_policy: ReleasePolicy,
    file: &TargetsFile,
    file_path: &Path,
    momentum: Option<&BTreeMap<String, f64>>,
) {
    out.push_str("## Next action\n\n");

    // Priority 1 — invariants violated (blocking).
    if let InvariantsResult::Violated { exit_code } = invariants {
        out.push_str(&format!(
            "**Blocked**: invariants failing (exit {exit_code}). See the ## Invariants \
             section above for detail. Fix the violations before proceeding with target work.\n",
        ));
        return;
    }
    // SpawnFailed is a real runtime error (permission denied, binary
    // not found, etc.) — different from HookMissing, which is an
    // intentional "project hasn't configured the hook yet" state.
    // Spawn failures block; missing hooks degrade.
    if let InvariantsResult::SpawnFailed { error } = invariants {
        out.push_str(&format!(
            "**Blocked**: could not run the invariants hook: {error}. Fix the build or \
             environment issue that prevents `make bullseye` / `mk bullseye` from running.\n",
        ));
        return;
    }

    // HookMissing flows through to the frontier recommendation just
    // like Skipped does, with an added note at the end so the agent
    // understands the recommendation is unguarded.
    let hook_missing_note = matches!(invariants, InvariantsResult::HookMissing);
    let hook_timed_out = matches!(invariants, InvariantsResult::TimedOut { .. });

    // Priority 2 — unreleased fixes take precedence over new target work,
    // unless the project declares a release freeze (always wins) or the
    // profile template sets unreleased_fixes: informational (🎯T46).
    let release_frozen = !unreleased.is_empty() && release_freeze.is_some();
    let informational = release_policy.unreleased_fixes == UnreleasedFixesPolicy::Informational;
    if !unreleased.is_empty() && !release_frozen && !informational {
        let n = unreleased.len();
        let s = if n == 1 { "" } else { "s" };
        out.push_str(&format!(
            "**Execute now**: Run `/release` to ship {n} unreleased fix{s}.\n\n\
             Users on the installed version are still hitting the bug{s}. Shipping existing \
             fixes takes precedence over starting new target work. See the ## Unreleased fixes \
             section above for the commit list.\n",
        ));
        return;
    }

    // Priority 3 — repo-level frontier ordering.
    //
    // Unblocking fanout is the primary sort signal, with target ID
    // as the deterministic tiebreak. See `graph::rank_frontier` for
    // the full rule. `momentum` is intentionally not consumed here —
    // it's a portfolio-scope input, not a repo-level signal.
    let _ = momentum;
    // 🎯T64: validation errors degrade the recommendation instead of
    // withholding it. The invalid targets are skipped and named; if
    // everything healthy is still blocked, that is reported below on
    // its own terms.
    let tolerant = graph::frontier_tolerant(file);
    if !tolerant.is_clean() {
        out.push_str(&format!(
            "**Note**: {} target(s) failed validation and were skipped (see above). \
             The recommendation below covers the rest of the graph.\n\n",
            tolerant.issues.len(),
        ));
    }
    let front = tolerant.targets;
    if front.is_empty() {
        out.push_str(
            "**Blocked**: no unblocked frontier targets. All active targets are blocked by \
             unmet dependencies. Investigate what's holding the frontier back, or retire \
             achieved targets to unblock downstream work.\n",
        );
        return;
    }

    let ranked = graph::rank_frontier(file, &front);

    // Collect top-tier ties — frontier targets sharing the same
    // unblocking fanout as the top candidate. These are all equally-
    // good choices from repo-level ordering's point of view and can
    // be fanned out to parallel agents.
    let top = &ranked[0];
    let top_fanout = top.fanout;
    let ties: Vec<&graph::RankedFrontier<'_>> = ranked
        .iter()
        .take_while(|rf| rf.fanout == top_fanout)
        .collect();

    if ties.len() == 1 {
        let t = ties[0];
        out.push_str(&format!(
            "**Execute now**: Work on 🎯{} {}\n\n\
             Unblocking fanout: {}. See the ## Frontier section above for this target's \
             acceptance criteria and context.\n",
            t.target.id, t.target.name, t.fanout,
        ));
    } else {
        let ids: Vec<String> = ties.iter().map(|t| format!("🎯{}", t.target.id)).collect();
        out.push_str(&format!(
            "**Execute now**: Work in parallel on {} frontier targets sharing the top \
             repo-level rank — {}.\n\n\
             All tied on unblocking fanout = {}. Each target's acceptance criteria and \
             context are in the ## Frontier section above. Fan out via parallel Agent \
             calls, one per target, per the Teams directive in CLAUDE.md.\n",
            ties.len(),
            ids.join(", "),
            top_fanout,
        ));
    }

    if hook_missing_note {
        out.push_str(
            "\n⚠ Note: standing invariants are **unknown** for this run — the project \
             has no `bullseye` rule in its Makefile/mkfile. See the ## Invariants section \
             for setup instructions. Proceed with caution until the hook is in place.\n",
        );
    }
    if hook_timed_out {
        out.push_str(
            "\n⚠ Note: the standing-invariants hook **timed out** and was killed — \
             invariants are **unknown** for this run. Find why `make bullseye` / `mk \
             bullseye` doesn't return (often a test blocking on input, network, or DB \
             auth), or pass skip_invariants to bypass it. Proceed with caution.\n",
        );
    }
    if let Some(reason) = release_freeze.filter(|_| !unreleased.is_empty()) {
        out.push_str(&format!(
            "\n⚠ Note: {} unreleased fix commit(s) are present, but `/release` is \
             suppressed because the project declares a release freeze ({reason}).\n",
            unreleased.len(),
        ));
    }
    if informational && !unreleased.is_empty() && !release_frozen {
        let profile = release_policy
            .profile
            .as_deref()
            .unwrap_or("(profile template)");
        let channel = release_policy
            .channel
            .as_deref()
            .map(|c| format!(" (channel: {c})"))
            .unwrap_or_default();
        out.push_str(&format!(
            "\n⚠ Note: {} unreleased fix commit(s) are listed above, but `/release` is \
             not recommended because profile `{profile}` sets \
             `unreleased_fixes: informational`{channel}. Continue frontier work instead.\n",
            unreleased.len(),
        ));
    }

    // Suppress unused-parameter warning for file_path in release builds.
    let _ = file_path;
}

/// Example `make bullseye` rule shown in the setup instructions.
const MAKE_BULLSEYE_EXAMPLE: &str = r#"bullseye:
	@cargo fmt --check && echo "✓ fmt"
	@cargo clippy --quiet -- -D warnings && echo "✓ clippy"
	@cargo test --quiet 2>&1 | grep "test result" && echo "✓ tests"
	@test -z "$$(git status --porcelain)" && echo "✓ clean" || \
	 (echo "✗ dirty tree"; git status --short; exit 1)"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_commit_markers() {
        // Positive cases — each one should match at least one marker.
        assert!(is_fix_commit("fix: typo in docs"));
        assert!(is_fix_commit("fix(parser): handle newline"));
        assert!(is_fix_commit("Fix audit log commit hash for v0.9.0"));
        assert!(is_fix_commit("Bump to v0.10.0 and fix broken schema"));
        assert!(is_fix_commit("hotfix for release workflow"));
        assert!(is_fix_commit("bugfix: off-by-one in parser"));
        assert!(is_fix_commit("Revert \"Add broken feature\""));
        assert!(is_fix_commit("Address regression in T5 verification"));
        assert!(is_fix_commit("Crash when bullseye.yaml is empty"));
        assert!(is_fix_commit("Handle incorrect merge resolution"));
        assert!(is_fix_commit("Repair broken migration path"));

        // Negative cases — no fix marker.
        assert!(!is_fix_commit("Add new feature"));
        assert!(!is_fix_commit("Refactor graph module"));
        assert!(!is_fix_commit("Update documentation"));
        assert!(!is_fix_commit("Bump version to v0.11.0"));
    }

    #[test]
    fn detect_invariants_errors_without_build_file() {
        let tmp = tempfile::tempdir().unwrap();
        let result = detect_invariants_command(tmp.path());
        assert!(matches!(result, Err(SetupReason::NoBuildFile)));
    }

    #[test]
    fn detect_invariants_returns_ok_for_any_makefile() {
        // New design: detect_invariants_command does NOT pre-parse the
        // Makefile to verify a `bullseye` rule exists. The rule check
        // happens downstream, by observing the build tool's own output
        // (see stderr_indicates_missing_rule). Here, as long as a
        // Makefile exists, we return the argv to run it.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Makefile"), "all:\n\t@echo hello\n").unwrap();
        let cmd = detect_invariants_command(tmp.path()).unwrap();
        assert_eq!(cmd, vec!["make".to_string(), "bullseye".to_string()]);
    }

    #[test]
    fn detect_invariants_prefers_mkfile_over_makefile() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Makefile"), "bullseye:\n\t@echo make\n").unwrap();
        std::fs::write(tmp.path().join("mkfile"), "bullseye:V:\n\t@echo mk\n").unwrap();
        let cmd = detect_invariants_command(tmp.path()).unwrap();
        assert_eq!(cmd[0], "mk");
    }

    #[test]
    fn detect_invariants_accepts_makefile_with_bullseye_rule() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Makefile"),
            "bullseye:\n\t@echo ok\n\nother:\n\t@true\n",
        )
        .unwrap();
        let cmd = detect_invariants_command(tmp.path()).unwrap();
        assert_eq!(cmd, vec!["make".to_string(), "bullseye".to_string()]);
    }

    #[test]
    fn stderr_missing_rule_matches_gnu_make_output() {
        // GNU make emits this phrasing; the cascade detection must
        // match it so run_invariants_command can classify the failure
        // as HookMissing rather than Violated.
        let stderr = "make: *** No rule to make target `bullseye'.  Stop.\n";
        assert!(stderr_indicates_missing_rule(stderr));
    }

    #[test]
    fn stderr_missing_rule_matches_bsd_make_output() {
        let stderr = "make: don't know how to make bullseye. Stop\n";
        assert!(stderr_indicates_missing_rule(stderr));
    }

    #[test]
    fn stderr_missing_rule_matches_mk_output() {
        // marcelocantos/mk emits this phrasing from graph.go:566 when
        // asked to build an unknown target. Verified against a live
        // `mk bullseye` run on an mkfile with no `bullseye` rule.
        let stderr = "mk: no rule to build \"bullseye\"\n";
        assert!(stderr_indicates_missing_rule(stderr));
    }

    #[test]
    fn stderr_missing_rule_ignores_real_failures() {
        // An ordinary compile failure or test failure must NOT be
        // mistaken for a missing rule — that would mask genuine
        // invariant violations as "hook not configured".
        let stderr = "error: cannot find module\nbuild failed with exit code 1\n";
        assert!(!stderr_indicates_missing_rule(stderr));
        let stderr = "test result: FAILED. 3 passed; 2 failed\n";
        assert!(!stderr_indicates_missing_rule(stderr));
    }

    #[test]
    fn render_setup_warning_no_build_file_mentions_both_options() {
        let mut out = String::new();
        render_setup_warning(&mut out, &SetupReason::NoBuildFile);
        assert!(out.contains("## Invariants"));
        assert!(out.contains("Makefile"));
        assert!(out.contains("mkfile"));
        assert!(out.contains("bullseye:"));
        // Informational, not a crash — degrades gracefully so
        // frontier recommendation still fires downstream.
        assert!(out.contains("unknown"));
        assert!(out.contains("Status: ⚠ unknown"));
    }

    #[test]
    fn render_setup_warning_missing_rule_includes_build_file_name() {
        let mut out = String::new();
        render_setup_warning(
            &mut out,
            &SetupReason::MissingRule {
                build_file: "Makefile".to_string(),
            },
        );
        assert!(out.contains("Makefile"));
        assert!(out.contains("`bullseye` target"));
        assert!(out.contains("Status: ⚠ unknown"));
    }

    #[test]
    fn invariants_hook_times_out_instead_of_hanging() {
        use std::time::Instant;
        // The esfera2-hang regression guard (🎯T38): a hook that would
        // run far past the timeout must be killed and return promptly,
        // not block the tool (and the agent) forever.
        let tmp = tempfile::tempdir().unwrap();
        let mut out = String::new();
        let start = Instant::now();
        let result = run_invariants_command_with_timeout(
            &["sleep".to_string(), "30".to_string()],
            tmp.path(),
            &mut out,
            Duration::from_millis(500),
        );
        let elapsed = start.elapsed();
        assert!(
            matches!(result, InvariantsResult::TimedOut { .. }),
            "expected TimedOut, got {result:?}"
        );
        assert!(
            out.contains("timed out"),
            "output must explain the timeout; got: {out}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "must return shortly after the 500ms timeout, not wait out the 30s sleep; took {elapsed:?}"
        );
    }

    #[test]
    fn invariants_hook_fast_command_completes() {
        // A hook that exits immediately must still run to completion and
        // report Green — the timeout machinery must not perturb the
        // normal case.
        let tmp = tempfile::tempdir().unwrap();
        let mut out = String::new();
        let result = run_invariants_command_with_timeout(
            &["true".to_string()],
            tmp.path(),
            &mut out,
            Duration::from_secs(10),
        );
        assert!(
            matches!(result, InvariantsResult::Green),
            "expected Green, got {result:?}"
        );
    }

    #[test]
    fn parse_release_policy_informational() {
        let yaml = r#"
# game profile
aspires: 2
release:
  unreleased_fixes: informational
  channel: store
other:
  key: value
"#;
        let p = parse_release_policy_yaml(yaml, "game");
        assert_eq!(p.unreleased_fixes, UnreleasedFixesPolicy::Informational);
        assert_eq!(p.channel.as_deref(), Some("store"));
        assert_eq!(p.profile.as_deref(), Some("game"));
    }

    #[test]
    fn parse_release_policy_default_recommend_ship() {
        let p = parse_release_policy_yaml("aspires: 1\n", "cli");
        assert_eq!(p.unreleased_fixes, UnreleasedFixesPolicy::RecommendShip);
        assert!(p.channel.is_none());
    }

    #[test]
    fn parse_release_policy_recommend_ship_explicit() {
        let yaml = "release:\n  unreleased_fixes: recommend_ship\n";
        let p = parse_release_policy_yaml(yaml, "cli");
        assert_eq!(p.unreleased_fixes, UnreleasedFixesPolicy::RecommendShip);
    }

    #[test]
    fn parse_profile_line_simple() {
        assert_eq!(
            parse_profile_line("profile: game\n"),
            Some("game".to_string())
        );
        assert_eq!(
            parse_profile_line("  profile: \"cli\"  \n"),
            Some("cli".to_string())
        );
        assert_eq!(parse_profile_line("no profile here\n"), None);
    }

    #[test]
    fn partition_empty_surface_keeps_all_ship_relevant() {
        let tmp = tempfile::tempdir().unwrap();
        let fixes = vec![UnreleasedFix {
            hash: "abc".into(),
            subject: "fix: x".into(),
        }];
        let (ship, ext) = partition_by_release_surface(tmp.path(), &fixes, &[]);
        assert_eq!(ship.len(), 1);
        assert!(ext.is_empty());
    }

    #[test]
    fn no_hardcoded_game_in_release_policy_path() {
        // 🎯T46: policy code must not branch on product flavor strings.
        let src = include_str!("convergence.rs");
        // Strip string literals roughly; just ensure we don't match on "game" as a code branch.
        assert!(
            !src.contains("== \"game\"") && !src.contains("== \"Game\""),
            "release-policy path must not hard-code product flavors"
        );
    }
}
