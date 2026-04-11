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

/// Check whether the project has a usable `bullseye` rule and, if so,
/// return the command to invoke it. Returns `Err(SetupReason)` if the
/// build file is missing or the rule isn't defined — the caller uses
/// [`setup_instructions`] to render the response.
pub fn detect_invariants_command(repo_root: &Path) -> Result<Vec<String>, SetupReason> {
    // Prefer mkfile when both are present: if the project went to the
    // trouble of using mk, that's the canonical build tool for it.
    let mkfile = repo_root.join("mkfile");
    if mkfile.is_file() {
        return if has_bullseye_rule(&mkfile) {
            Ok(vec!["mk".to_string(), "bullseye".to_string()])
        } else {
            Err(SetupReason::MissingRule {
                build_file: "mkfile".to_string(),
            })
        };
    }
    let makefile = repo_root.join("Makefile");
    if makefile.is_file() {
        return if has_bullseye_rule(&makefile) {
            Ok(vec!["make".to_string(), "bullseye".to_string()])
        } else {
            Err(SetupReason::MissingRule {
                build_file: "Makefile".to_string(),
            })
        };
    }
    Err(SetupReason::NoBuildFile)
}

fn has_bullseye_rule(build_file: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(build_file) else {
        return false;
    };
    // A rule line looks like `bullseye:` or `bullseye :` at column 0,
    // optionally followed by dependencies. This is a loose check that
    // accepts any rule name starting with `bullseye` followed by `:`.
    content.lines().any(|line| {
        let trimmed = line.trim_end();
        (trimmed == "bullseye:"
            || trimmed.starts_with("bullseye:")
            || trimmed.starts_with("bullseye :"))
            && !line.starts_with(' ')
            && !line.starts_with('\t')
    })
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

    // Priority 3 — highest-focus frontier target.
    let errors = graph::validate(file);
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

    // Compute focus scores for the frontier so we can find the top
    // candidate(s). Same formula as graph::summary — see that function
    // for the rationale.
    let all_targets = &file.targets;
    let mut scored: Vec<(&graph::FrontierTarget, f64)> = front
        .iter()
        .map(|ft| {
            let value = all_targets.get(&ft.id).map(|t| t.value).unwrap_or(0.0);
            let m = momentum
                .and_then(|mm| mm.get(&ft.id).copied())
                .unwrap_or(1.0);
            (ft, value * m)
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let top_focus = scored[0].1;
    let ties: Vec<&graph::FrontierTarget> = scored
        .iter()
        .take_while(|(_, s)| (s - top_focus).abs() < 1e-6)
        .map(|(ft, _)| *ft)
        .collect();

    if ties.len() == 1 {
        let top = ties[0];
        out.push_str(&format!(
            "**Execute now**: Work on 🎯{} {}\n\n\
             See the ## Frontier section above for this target's acceptance criteria and \
             context.\n",
            top.id, top.name,
        ));
    } else {
        let ids: Vec<String> = ties.iter().map(|t| format!("🎯{}", t.id)).collect();
        out.push_str(&format!(
            "**Execute now**: Work in parallel on {} frontier targets sharing the top focus \
             score — {}.\n\n\
             Each target's acceptance criteria and context are in the ## Frontier section \
             above. Fan out via parallel Agent calls, one per target, per the Teams \
             directive in CLAUDE.md.\n",
            ties.len(),
            ids.join(", "),
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
        assert!(is_fix_commit("Crash when targets.yaml is empty"));
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
    fn detect_invariants_errors_on_makefile_without_rule() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Makefile"), "all:\n\t@echo hello\n").unwrap();
        let result = detect_invariants_command(tmp.path());
        match result {
            Err(SetupReason::MissingRule { build_file }) => assert_eq!(build_file, "Makefile"),
            other => panic!("expected MissingRule, got {other:?}"),
        }
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
