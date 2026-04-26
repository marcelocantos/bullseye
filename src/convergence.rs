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
use std::process::Command;

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
    let unreleased = detect_unreleased_fixes(repo_root);
    render_unreleased(&mut out, &unreleased);

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
        &unreleased,
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
}

fn run_invariants_command(argv: &[String], repo_root: &Path, out: &mut String) -> InvariantsResult {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(repo_root);
    let output = match cmd.output() {
        Ok(o) => o,
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
    out.push_str(stdout.trim_end());
    if !stdout.is_empty() && !stderr.is_empty() {
        out.push('\n');
    }
    if !stderr.is_empty() {
        out.push_str(stderr.trim_end());
    }
    out.push_str("\n```\n\n");

    if exit_code == 0 {
        out.push_str("Status: ✓ all green\n\n");
        InvariantsResult::Green
    } else {
        out.push_str(&format!("Status: ✗ failed (exit {exit_code})\n\n"));
        InvariantsResult::Violated { exit_code }
    }
}

/// A single unreleased fix commit.
#[derive(Debug, Clone)]
pub struct UnreleasedFix {
    pub hash: String,
    pub subject: String,
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

fn render_unreleased(out: &mut String, fixes: &[UnreleasedFix]) {
    out.push_str("## Unreleased fixes\n\n");
    if fixes.is_empty() {
        out.push_str("(none — everything on master is released)\n\n");
        return;
    }
    for f in fixes {
        out.push_str(&format!("- `{}` {}\n", f.hash, f.subject));
    }
    out.push('\n');
}

fn render_next_action(
    out: &mut String,
    invariants: &InvariantsResult,
    unreleased: &[UnreleasedFix],
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

    // Priority 2 — unreleased fixes take precedence over new target work.
    if !unreleased.is_empty() {
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

    // Priority 3 — repo-level frontier ordering (🎯T7).
    //
    // Distance-to-nearest-checkpoint is the primary sort signal,
    // with unblocking fanout as tiebreaker and ID as final tiebreak.
    // See `graph::rank_frontier` for the full rule. `momentum` is
    // intentionally not consumed here — it's a portfolio-scope
    // input, not a repo-level signal. See 🎯T7 in `bullseye.yaml`
    // and §9 of `docs/mcp-triad.md`.
    let _ = momentum;
    let errors = graph::validate_blocking(file);
    if !errors.is_empty() {
        out.push_str(
            "**Blocked**: targets file has validation errors (see above). Fix the graph \
             before proceeding.\n",
        );
        return;
    }
    let front = graph::frontier(file);
    if front.is_empty() {
        out.push_str(
            "**Blocked**: no unblocked frontier targets. All active targets are blocked by \
             unmet dependencies. Investigate what's holding the frontier back, or retire \
             achieved targets to unblock downstream work.\n",
        );
        return;
    }

    let ranked = graph::rank_frontier(file, &front);

    // Tunnel reshape guard: if the top-ranked frontier candidate has
    // no checkpoint reachable at all, selecting it would extend a
    // tunnel — a chain of non-checkpoint targets with no
    // human-visible signal at the end. Recommend reshaping the graph
    // (adding an intermediate verify target or promoting an existing
    // work target to `showcase: true`) rather than blundering
    // forward. Uses the `**Blocked**:` prefix so the `/cv` skill's
    // existing auto-execute branch correctly pauses instead of
    // dispatching to an agent. See acceptance #5 on 🎯T7.
    let top = &ranked[0];
    if top.distance.is_none() {
        let tun_len = ranked.iter().take_while(|rf| rf.distance.is_none()).count();
        out.push_str(&format!(
            "**Blocked**: top frontier target 🎯{id} \"{name}\" would extend a tunnel with \
             no checkpoint reachable downstream. {tun_len} of {total} frontier \
             target(s) have no checkpoint reachable at all.\n\n\
             Reshape the graph before proceeding — either add an intermediate verify target \
             or mark an existing downstream work target as `showcase: true` so the \
             decision-maker gets a checkpoint within a few hops. See the ## Frontier section \
             above for the full target list and the 🎯T7 rationale in \
             `docs/mcp-triad.md` §9.\n",
            id = top.target.id,
            name = top.target.name,
            total = ranked.len(),
        ));
        if hook_missing_note {
            out.push_str(
                "\n⚠ Note: standing invariants are **unknown** for this run — the project \
                 has no `bullseye` rule in its Makefile/mkfile. See the ## Invariants section \
                 for setup instructions.\n",
            );
        }
        return;
    }

    // Collect top-tier ties — frontier targets sharing the exact same
    // (distance, fanout) pair as the top candidate. These are all
    // equally-good choices from repo-level ordering's point of view
    // and can be fanned out to parallel agents.
    let top_key = (top.distance, top.fanout);
    let ties: Vec<&graph::RankedFrontier<'_>> = ranked
        .iter()
        .take_while(|rf| (rf.distance, rf.fanout) == top_key)
        .collect();

    if ties.len() == 1 {
        let t = ties[0];
        out.push_str(&format!(
            "**Execute now**: Work on 🎯{} {}\n\n\
             Distance to nearest checkpoint: {}. Unblocking fanout: {}. \
             See the ## Frontier section above for this target's acceptance criteria and \
             context.\n",
            t.target.id,
            t.target.name,
            describe_distance(t.distance),
            t.fanout,
        ));
    } else {
        let ids: Vec<String> = ties.iter().map(|t| format!("🎯{}", t.target.id)).collect();
        out.push_str(&format!(
            "**Execute now**: Work in parallel on {} frontier targets sharing the top \
             repo-level rank — {}.\n\n\
             All tied on distance-to-checkpoint = {} and unblocking fanout = {}. Each \
             target's acceptance criteria and context are in the ## Frontier section \
             above. Fan out via parallel Agent calls, one per target, per the Teams \
             directive in CLAUDE.md.\n",
            ties.len(),
            ids.join(", "),
            describe_distance(top_key.0),
            top_key.1,
        ));
    }

    if hook_missing_note {
        out.push_str(
            "\n⚠ Note: standing invariants are **unknown** for this run — the project \
             has no `bullseye` rule in its Makefile/mkfile. See the ## Invariants section \
             for setup instructions. Proceed with caution until the hook is in place.\n",
        );
    }

    // Suppress unused-parameter warning for file_path in release builds.
    let _ = file_path;
}

/// Human-readable distance for the next-action text. Collapses the
/// `Some(0)` case to "checkpoint" so the copy reads naturally when
/// the top candidate is itself a checkpoint.
fn describe_distance(distance: Option<usize>) -> String {
    match distance {
        Some(0) => "checkpoint (0)".to_string(),
        Some(n) => n.to_string(),
        None => "unreachable".to_string(),
    }
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
}
