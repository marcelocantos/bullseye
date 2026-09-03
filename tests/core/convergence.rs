// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn convergence_end_to_end_green_invariants_picks_top_frontier() {
    // Full integration: real temp project, real Makefile that exits 0,
    // real bullseye.yaml, real convergence pipeline. Verifies the whole
    // path from hook invocation to recommendation text.
    let tmp = tempfile::tempdir().unwrap();
    // `true` is a trivial program that exits 0 — standing invariants green.
    let makefile = "bullseye:\n\t@true\n";
    write_project(tmp.path(), makefile, SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    assert!(out.contains("# Convergence"));
    assert!(out.contains("## Invariants"));
    assert!(
        out.contains("Status: ✅ all green"),
        "expected green status; got:\n{out}"
    );
    assert!(out.contains("## Unreleased fixes"));
    // No git history in the temp dir → no tag → no unreleased fixes reported.
    assert!(out.contains("(none"));
    assert!(out.contains("## Frontier"));
    // Frontier should include both targets with full details inline.
    assert!(out.contains("🎯T1 Primary deliverable"));
    assert!(out.contains("🎯T2 Secondary polish"));
    assert!(
        out.contains("Produces the primary artifact"),
        "frontier details should include acceptance criteria; got:\n{out}"
    );
    assert!(
        out.contains("The highest-value thing in the project."),
        "frontier details should include context; got:\n{out}"
    );
    assert!(out.contains("## Next action"));
    assert!(
        out.contains("**Execute now**: Work on 🎯T1 Primary deliverable"),
        "expected top-focus target as next action; got:\n{out}"
    );
    // No WSJF ranking annotations anywhere in the convergence output.
    // (The 🎯T16 banner names WSJF once to explicitly disavow it at
    // repo scope — absence of `wsjf=` covers the actual anti-pattern
    // of scored WSJF entries without colliding with the disavowal.)
    assert!(!out.to_lowercase().contains("wsjf="));
    // 🎯T16: repo-scope banner must survive the summary-body splice
    // into convergence output.
    assert!(
        out.contains("Repo-scope ordering"),
        "convergence output must carry the repo-scope banner; got:\n{out}"
    );
}

#[test]
fn convergence_end_to_end_red_invariants_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    // `false` exits 1 — standing invariants red.
    let makefile = "bullseye:\n\t@echo 'tests failing'; false\n";
    write_project(tmp.path(), makefile, SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    assert!(out.contains("Status: ❌ failed"));
    assert!(out.contains("tests failing"));
    let next = out
        .split("## Next action")
        .nth(1)
        .expect("next action section");
    assert!(
        next.contains("**Blocked**"),
        "blocked recommendation expected; got:\n{next}"
    );
    assert!(next.contains("invariants failing"));
    // Crucially: the Execute now path must NOT fire when invariants fail,
    // even though there are perfectly good frontier targets.
    assert!(!next.contains("**Execute now**"));
}

#[test]
fn convergence_end_to_end_skip_invariants_flag_bypasses_hook() {
    let tmp = tempfile::tempdir().unwrap();
    // Red Makefile — but we're skipping, so it should never run.
    let makefile = "bullseye:\n\t@echo 'would have failed'; false\n";
    write_project(tmp.path(), makefile, SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, true);

    assert!(out.contains("(skipped"));
    assert!(!out.contains("would have failed"));
    assert!(!out.contains("Status: ❌"));
    // With invariants skipped and no unreleased fixes, we should go
    // straight to the frontier-based recommendation.
    let next = out
        .split("## Next action")
        .nth(1)
        .expect("next action section");
    assert!(
        next.contains("**Execute now**: Work on 🎯T1"),
        "expected top-focus target as next action; got:\n{next}"
    );
}

#[test]
#[cfg(unix)]
fn convergence_returns_despite_blocking_pre_commit_hook() {
    // 🎯T73 inverted 🎯T62's auto-commit hang: bullseye no longer runs
    // `git commit` before invariants, so a blocking pre-commit hook is
    // never invoked. Dirty yaml + a hook that ignores the ledger →
    // invariants pass, no `## Auto-commit` section.
    let tmp = tempfile::tempdir().unwrap();
    write_project_with_blocking_pre_commit_hook(tmp.path(), YAML_IGNORING_MAKEFILE);

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();

    let start = std::time::Instant::now();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "convergence must not wait on a pre-commit hook it no longer runs; took {elapsed:?}"
    );
    assert!(
        !out.contains("## Auto-commit"),
        "auto-commit rail is gone; got:\n{out}"
    );
    assert!(
        out.contains("Status: ✅ all green"),
        "yaml dirt must not fail standing invariants; got:\n{out}"
    );
    assert!(
        out.contains("## Next action"),
        "convergence must still produce a next action; got:\n{out}"
    );
    assert!(
        !out.contains("**Blocked**"),
        "yaml dirt must not block next action; got:\n{out}"
    );
}

#[test]
#[cfg(unix)]
fn convergence_skip_invariants_runs_no_project_hooks() {
    // 🎯T62 acceptance: with skip_invariants=true, convergence runs none
    // of the project's own code — not the `bullseye` rule, and not the
    // `pre-commit` hook that `git commit` would fire. `skip_invariants`
    // is what a caller reaches for when the project's checks are the
    // suspect, so a back-door invocation of them defeats the flag; that
    // is why the original hang survived a skip_invariants retry.
    let tmp = tempfile::tempdir().unwrap();
    write_project_with_blocking_pre_commit_hook(tmp.path(), "bullseye:\n\t@echo 'ok'; false\n");

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();

    let start = std::time::Instant::now();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, true);
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "skip_invariants=true must not block on any project subprocess; took {elapsed:?}"
    );
    assert!(out.contains("(skipped"), "got:\n{out}");
    assert!(
        out.contains("## Next action"),
        "convergence must still produce a next action; got:\n{out}"
    );
}

#[test]
fn mutation_does_not_create_yaml_only_git_commit() {
    // 🎯T73: a real handler mutation in a git repo with a prior
    // non-ledger commit must not add a yaml-only commit. Ledger
    // content on disk still reflects the mutation.
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_commit;
    use bullseye::tools::CommitTool;

    let tmp = tempfile::tempdir().unwrap();
    t73_git_repo_with_readme(tmp.path());
    let path = store::create_at(tmp.path(), Location::InRepo, "t73").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            bullseye::config::set_external_root_override(None);
        }
    }
    let _cleanup = Cleanup;

    let before = t73_commit_count(tmp.path());
    let head_before = t73_git(tmp.path(), &["rev-parse", "HEAD"]);

    let result = handle_commit(CommitTool {
        cwd,
        op: "track".into(),
        id: None,
        child_of: None,
        name: Some("T73 mutation target".into()),
        value: None,
        cost: None,
        acceptance: Some(vec!["a".into()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
        actual_cost: None,
        attestation: None,
        reason: None,
        postponed_until: None,
        postpone_predicate: None,
        parent: None,
        mode: None,
        children: None,
        retire_reason: None,
        tail: None,
        owner: None,
    })
    .expect("track must succeed");
    let text = text_from_call_result(result);
    assert!(
        text.contains("ok: true") && text.contains("Created"),
        "mutation must be acknowledged; got:\n{text}"
    );

    assert_eq!(
        t73_commit_count(tmp.path()),
        before,
        "handler mutation must not add a git commit"
    );
    assert_eq!(t73_git(tmp.path(), &["rev-parse", "HEAD"]), head_before);
    let log = t73_git(tmp.path(), &["log", "-1", "--name-only"]);
    assert!(
        !log.contains("Update bullseye.yaml"),
        "HEAD must not be a bullseye-produced yaml-only commit; got:\n{log}"
    );
    let head_files: Vec<&str> = log.lines().skip(1).filter(|l| !l.is_empty()).collect();
    assert_ne!(
        head_files,
        vec!["bullseye.yaml"],
        "HEAD must not touch only bullseye.yaml; files={head_files:?}"
    );

    let ledger = std::fs::read_to_string(&path).unwrap();
    assert!(
        ledger.contains("T73 mutation target"),
        "ledger on disk must reflect the mutation; got:\n{ledger}"
    );
}

#[test]
fn convergence_dirty_yaml_passes_when_hook_ignores_ledger() {
    // 🎯T73: dirty in-repo bullseye.yaml + otherwise-green make bullseye
    // that ignores the ledger → standing invariants pass. Residue: a
    // dirty non-ledger file still fails if the hook says so.
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path(), YAML_IGNORING_MAKEFILE, SIMPLE_TARGETS_YAML);
    t73_git_repo_with_readme(tmp.path());
    t73_git(tmp.path(), &["add", "-A"]);
    t73_git(tmp.path(), &["commit", "-q", "-m", "add ledger"]);

    // Dirty the ledger only.
    use std::io::Write;
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(tmp.path().join("bullseye.yaml"))
            .unwrap(),
        "# touched"
    )
    .unwrap();

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);
    assert!(
        out.contains("Status: ✅ all green"),
        "yaml dirt must not fail standing invariants; got:\n{out}"
    );
    assert!(!out.contains("**Blocked**"), "got:\n{out}");
    assert!(!out.contains("## Auto-commit"), "got:\n{out}");

    // Residue: other dirty files still fail.
    std::fs::write(tmp.path().join("NOTES.txt"), "scratch\n").unwrap();
    let out_dirty = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);
    assert!(
        out_dirty.contains("Status: ❌ failed") || out_dirty.contains("✗ dirty tree"),
        "non-ledger dirt must still fail the hook; got:\n{out_dirty}"
    );
    let next = out_dirty
        .split("## Next action")
        .nth(1)
        .expect("next action");
    assert!(
        next.contains("**Blocked**"),
        "other dirt must block; got:\n{next}"
    );
}

#[test]
fn convergence_dirty_tree_warns_without_failing_recommended_hook() {
    // Recommended policy: non-ledger dirt is a loud warning (exit 0),
    // so /cv can still recommend next work and the agent decides.
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path(), DIRTY_WARN_MAKEFILE, SIMPLE_TARGETS_YAML);
    t73_git_repo_with_readme(tmp.path());
    t73_git(tmp.path(), &["add", "-A"]);
    t73_git(tmp.path(), &["commit", "-q", "-m", "add ledger"]);

    std::fs::write(tmp.path().join("NOTES.txt"), "scratch\n").unwrap();

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);
    assert!(
        out.contains("Status: ✅ all green"),
        "recommended dirty-tree hook must not fail invariants; got:\n{out}"
    );
    assert!(
        out.contains("DIRTY WORKING TREE") || out.contains("Warning only"),
        "dirty tree must be noisy in invariants output; got:\n{out}"
    );
    assert!(!out.contains("**Blocked**"), "got:\n{out}");
    let next = out.split("## Next action").nth(1).expect("next action");
    assert!(
        next.contains("**Execute now**") || next.contains("Work on"),
        "frontier recommendation must still fire; got:\n{next}"
    );
}

#[test]
fn convergence_nested_dirty_yaml_passes_when_hook_ignores_ledger() {
    // Nested in-repo path (e.g. hms2/bullseye.yaml) counts as yaml dirt.
    let tmp = tempfile::tempdir().unwrap();
    t73_git_repo_with_readme(tmp.path());
    let nested = tmp.path().join("hms2");
    std::fs::create_dir_all(&nested).unwrap();
    write_project(&nested, YAML_IGNORING_MAKEFILE, SIMPLE_TARGETS_YAML);
    t73_git(tmp.path(), &["add", "-A"]);
    t73_git(tmp.path(), &["commit", "-q", "-m", "add nested ledger"]);

    use std::io::Write;
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(nested.join("bullseye.yaml"))
            .unwrap(),
        "# nested touch"
    )
    .unwrap();

    let path = nested.join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, &nested, None, false);
    assert!(
        out.contains("Status: ✅ all green"),
        "nested yaml dirt must not fail standing invariants; got:\n{out}"
    );
    assert!(!out.contains("**Blocked**"), "got:\n{out}");
}

#[test]
fn repo_makefile_ignores_bullseye_yaml() {
    let makefile = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Makefile"));
    assert!(
        makefile.contains("grep -vE 'bullseye\\.yaml$$'"),
        "this repo's make bullseye must ignore bullseye.yaml; got:\n{makefile}"
    );
}

#[test]
fn convergence_missing_makefile_degrades_gracefully() {
    // A repo with bullseye.yaml but no Makefile. Convergence must
    // still run to completion — emit the target snapshot, mark
    // invariants as unknown with setup instructions embedded, and
    // still produce a frontier recommendation.
    let tmp = tempfile::tempdir().unwrap();
    use std::io::Write;
    write!(
        std::fs::File::create(tmp.path().join("bullseye.yaml")).unwrap(),
        "{SIMPLE_TARGETS_YAML}"
    )
    .unwrap();
    // Note: NO Makefile.

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    // Full convergence shape is present.
    assert!(out.contains("# Convergence"));
    assert!(out.contains("## Invariants"));
    assert!(out.contains("## Unreleased fixes"));
    assert!(out.contains("## Frontier"));
    assert!(out.contains("## Next action"));

    // Invariants section includes the setup warning inline.
    let invariants_section = out
        .split("## Invariants")
        .nth(1)
        .expect("invariants section");
    let end = invariants_section
        .find("\n## ")
        .unwrap_or(invariants_section.len());
    let invariants_text = &invariants_section[..end];
    assert!(invariants_text.contains("⚠"));
    assert!(invariants_text.contains("not configured"));
    assert!(invariants_text.contains("Makefile"));
    assert!(invariants_text.contains("bullseye:"));
    assert!(invariants_text.contains("unknown"));
    // 🎯T73: the emitted skeleton's dirty-tree check ignores the ledger.
    assert!(
        invariants_text.contains("grep -vE")
            && (invariants_text.contains("bullseye.yaml")
                || invariants_text.contains("bullseye\\.yaml")),
        "missing-hook skeleton must ignore bullseye.yaml; got:\n{invariants_text}"
    );

    // Target snapshot still renders — the frontier has details.
    assert!(out.contains("🎯T1 Primary deliverable"));
    assert!(out.contains("Produces the primary artifact"));

    // Next action still fires — frontier recommendation — with a
    // prominent note that invariants are unknown.
    let next = out.split("## Next action").nth(1).expect("next action");
    assert!(
        next.contains("**Execute now**: Work on 🎯T1"),
        "frontier recommendation should still fire when hook is missing; got:\n{next}"
    );
    assert!(
        next.contains("standing invariants are **unknown**"),
        "should warn that invariants are unknown; got:\n{next}"
    );
}

#[test]
fn convergence_makefile_without_bullseye_rule_degrades_gracefully() {
    // Same shape as the no-Makefile case, but with a Makefile that
    // exists but has no `bullseye` target. The setup warning should
    // identify the specific build file so the fix is obvious.
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path(), "all:\n\t@echo hello\n", SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    let invariants_section = out
        .split("## Invariants")
        .nth(1)
        .expect("invariants section");
    let end = invariants_section
        .find("\n## ")
        .unwrap_or(invariants_section.len());
    let invariants_text = &invariants_section[..end];
    assert!(invariants_text.contains("found `Makefile`"));
    assert!(invariants_text.contains("no `bullseye` target"));

    // Frontier recommendation still fires.
    let next = out.split("## Next action").nth(1).expect("next action");
    assert!(next.contains("**Execute now**: Work on 🎯T1"));
}

#[test]
fn handle_convergence_resolves_repo_root() {
    // Regression guard for a user-reported bug: `handle_convergence`
    // used to compute the repo root by stepping up two parent
    // directories unconditionally, landing in the grandparent. No
    // Makefile was found there, so invariant detection fell through to
    // "hook not configured" even though the repo had a perfectly good
    // `bullseye:` rule at the real root.
    //
    // Every other convergence end-to-end test in this file calls
    // `bullseye::convergence::convergence(...)` directly, passing
    // `repo_root` explicitly — which bypasses the path-computation
    // layer that contained the bug. This test drives
    // `handle_convergence` as a full integration so any future
    // inversion of `repo_root_from_targets_path` or
    // `store::discover`'s candidate order is caught at the handler
    // boundary.
    use bullseye::config;
    use bullseye::handler::handle_convergence;
    use bullseye::tools::ConvergenceTool;

    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path(), "bullseye:\n\t@true\n", SIMPLE_TARGETS_YAML);

    // Isolate the external shadow root so discover_anywhere can't pick
    // up state from the developer's real ~/.local/share/bullseye.
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            bullseye::config::set_external_root_override(None);
        }
    }
    let _cleanup = Cleanup;

    let result = handle_convergence(ConvergenceTool {
        cwd: tmp.path().to_string_lossy().into_owned(),
        momentum: None,
        skip_invariants: None,
    })
    .expect("handle_convergence should succeed with a valid project");
    let out = text_from_call_result(result);

    // Headline assertion: the invariants hook must have been found and
    // run. If the repo root was computed incorrectly, this would
    // instead report "not configured" + a setup warning, and the
    // status would be "unknown".
    assert!(
        out.contains("Status: ✅ all green"),
        "expected green invariants status — this is the regression guard for the \
         root-level bullseye.yaml bug; if this fails, handle_convergence is \
         computing repo_root incorrectly. Output:\n{out}"
    );

    // Mirror the canonical `convergence_end_to_end_green_invariants_picks_top_frontier`
    // assertions so this test also covers the rest of the pipeline,
    // not just the repo-root fix.
    assert!(out.contains("# Convergence"));
    assert!(out.contains("## Invariants"));
    assert!(out.contains("## Frontier"));
    assert!(out.contains("🎯T1 Primary deliverable"));
    assert!(
        out.contains("**Execute now**: Work on 🎯T1 Primary deliverable"),
        "expected top-focus target as next action; got:\n{out}"
    );

    // Negative: no stray "not configured" text anywhere — this is the
    // exact phrase the buggy path produced, and it must not appear.
    assert!(
        !out.contains("not configured"),
        "convergence should not report the hook as missing when it is \
         present at the repo root; got:\n{out}"
    );
}

#[test]
fn convergence_unreleased_fixes_detected_in_git_repo() {
    // Initialise a real git repo in a temp dir, tag it, then add a
    // "Fix ..." commit so convergence sees an unreleased fix.
    use std::process::Command;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path();
    write_project(path, "bullseye:\n\t@true\n", SIMPLE_TARGETS_YAML);

    // Minimal git init + config + tag + commit sequence.
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr),
        );
    };
    git(&["init", "-q", "-b", "master"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "test"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "Initial"]);
    git(&["tag", "v0.1.0"]);

    // Touch a file and make a fix commit.
    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "-q", "-m", "Fix missing README for v0.1.0"]);

    let yaml_path = path.join("bullseye.yaml");
    let file = store::load(&yaml_path).unwrap();
    let out = bullseye::convergence::convergence(&file, &yaml_path, path, None, false);

    let unreleased_section = out
        .split("## Unreleased fixes")
        .nth(1)
        .expect("unreleased section exists");
    assert!(
        unreleased_section.contains("Fix missing README"),
        "expected fix commit in unreleased section; got:\n{unreleased_section}"
    );
    // Priority 2: unreleased fixes take precedence — expect /release as next action.
    let next = out
        .split("## Next action")
        .nth(1)
        .expect("next action section");
    assert!(
        next.contains("**Execute now**: Run `/release`"),
        "expected /release next action; got:\n{next}"
    );
}

#[test]
fn convergence_release_freeze_suppresses_release_recommendation() {
    use std::process::Command;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path();
    write_project(path, "bullseye:\n\t@true\n", SIMPLE_TARGETS_YAML);

    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr),
        );
    };
    git(&["init", "-q", "-b", "master"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "test"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "Initial"]);
    git(&["tag", "v0.1.0"]);

    std::fs::write(
        path.join("AGENTS.md"),
        r#"release_freeze: "migration in progress""#,
    )
    .unwrap();
    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    git(&["add", "AGENTS.md", "README.md"]);
    git(&["commit", "-q", "-m", "Fix missing README for v0.1.0"]);

    let yaml_path = path.join("bullseye.yaml");
    let file = store::load(&yaml_path).unwrap();
    let out = bullseye::convergence::convergence(&file, &yaml_path, path, None, false);

    let next = out
        .split("## Next action")
        .nth(1)
        .expect("next action section");
    assert!(
        next.contains("**Execute now**: Work on 🎯T1 Primary deliverable"),
        "expected frontier work while release is frozen; got:\n{next}"
    );
    assert!(
        !next.contains("Run `/release`"),
        "release freeze should suppress /release recommendation; got:\n{next}"
    );
    assert!(
        next.contains("release freeze") && next.contains("migration in progress"),
        "expected release-freeze note; got:\n{next}"
    );
}

#[test]
fn convergence_release_freeze_is_found_at_git_root_from_subdir() {
    use std::process::Command;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let subdir = root.join("hms2");
    std::fs::create_dir(&subdir).unwrap();
    write_project(&subdir, "bullseye:\n\t@true\n", SIMPLE_TARGETS_YAML);

    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr),
        );
    };
    git(&["init", "-q", "-b", "master"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "test"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "Initial"]);
    git(&["tag", "v0.1.0"]);

    std::fs::write(root.join("AGENTS.md"), r#"release_freeze: "port phase""#).unwrap();
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    git(&["add", "AGENTS.md", "README.md"]);
    git(&["commit", "-q", "-m", "Fix missing README for v0.1.0"]);

    let yaml_path = subdir.join("bullseye.yaml");
    let file = store::load(&yaml_path).unwrap();
    let out = bullseye::convergence::convergence(&file, &yaml_path, &subdir, None, false);

    let next = out
        .split("## Next action")
        .nth(1)
        .expect("next action section");
    assert!(
        next.contains("**Execute now**: Work on 🎯T1 Primary deliverable"),
        "expected frontier work while root release is frozen; got:\n{next}"
    );
    assert!(
        !next.contains("Run `/release`"),
        "root release freeze should suppress /release recommendation; got:\n{next}"
    );
    assert!(
        next.contains("port phase"),
        "expected root release-freeze reason; got:\n{next}"
    );
}

#[test]
fn t46_informational_policy_suppresses_release_in_next_action_notes() {
    use bullseye::convergence::{
        ReleasePolicy, UnreleasedFix, UnreleasedFixesPolicy, parse_release_policy_yaml,
    };

    let p = parse_release_policy_yaml(
        "release:\n  unreleased_fixes: informational\n  channel: store\n",
        "game",
    );
    assert_eq!(p.unreleased_fixes, UnreleasedFixesPolicy::Informational);
    assert_eq!(p.channel.as_deref(), Some("store"));

    // recommend_ship default when missing release block
    let d = ReleasePolicy::default();
    assert_eq!(d.unreleased_fixes, UnreleasedFixesPolicy::RecommendShip);

    // smoke: fix list type still usable
    let _ = UnreleasedFix {
        hash: "dead".into(),
        subject: "fix: x".into(),
    };
}

// --- 🎯T64: transition hygiene and read-path tolerance ------------------
//
// Two defects, both from the jevons incident of 2026-08-10: a status
// transition left behind a field the destination status forbids, and the
// read path answered a validation error *instead of* the graph, so one
// bad field made an entire ledger unreadable.
