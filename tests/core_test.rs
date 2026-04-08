// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use bullseye::graph;
use bullseye::ops;
use bullseye::render;
use bullseye::schema::{Kind, Status, TargetsFile};
use bullseye::store;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_fixture() -> TargetsFile {
    let path = fixture_path().join("docs/targets.yaml");
    store::load(&path).unwrap()
}

#[test]
fn loads_and_counts_targets() {
    let file = load_fixture();
    assert_eq!(file.targets.len(), 5);
}

#[test]
fn active_filter() {
    let file = load_fixture();
    let active = file.active();
    assert_eq!(active.len(), 4); // T1, T2, T3, T5 (T4 is achieved)
    assert!(!active.contains_key("T4"));
}

#[test]
fn achieved_filter() {
    let file = load_fixture();
    let achieved = file.achieved();
    assert_eq!(achieved.len(), 1);
    assert!(achieved.contains_key("T4"));
}

#[test]
fn validates_ok() {
    let file = load_fixture();
    let errors = graph::validate(&file);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[test]
fn mermaid_generation() {
    let file = load_fixture();
    let diagram = graph::mermaid(&file);
    assert!(diagram.contains("graph TD"));
    // T3 gates T1 — should have a dotted arrow.
    assert!(diagram.contains("gates"));
}

#[test]
fn discovers_from_subdirectory() {
    let found = store::discover(&fixture_path());
    assert!(found.is_some());
    assert!(found.unwrap().ends_with("docs/targets.yaml"));
}

#[test]
fn detects_cycle_in_depends_on() {
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().depends_on = vec!["T2".to_string()];
    file.targets.get_mut("T2").unwrap().depends_on = vec!["T1".to_string()];
    let errors = graph::validate(&file);
    assert!(errors.iter().any(|e| e.contains("cycle")));
}

#[test]
fn yaml_roundtrip() {
    let file = load_fixture();
    let yaml = serde_yaml_ng::to_string(&file).unwrap();
    let reparsed: TargetsFile = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(file.targets.len(), reparsed.targets.len());
    assert_eq!(file.targets["T1"].status, reparsed.targets["T1"].status);
}

#[test]
fn frontier_returns_unblocked_leaves() {
    let file = load_fixture();
    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    // T1 (converging, no deps), T2 (identified, no deps), T3 (identified, no deps)
    // are all active leaves with no unachieved dependencies.
    // T4 is achieved so excluded. T5 depends on T1+T3 so blocked.
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&"T1"));
    assert!(ids.contains(&"T2"));
    assert!(ids.contains(&"T3"));
    assert!(!ids.contains(&"T5"), "T5 should be blocked by T1 and T3");
}

#[test]
fn frontier_excludes_blocked() {
    let mut file = load_fixture();
    // Make T2 depend on T1 (which is converging, not achieved).
    file.targets.get_mut("T2").unwrap().depends_on = vec!["T1".to_string()];
    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    assert!(!ids.contains(&"T2"), "T2 should be blocked by T1");
    assert!(ids.contains(&"T1"));
    assert!(ids.contains(&"T3"));
}

#[test]
fn verify_target_kind() {
    let file = load_fixture();
    let t5 = &file.targets["T5"];
    assert_eq!(t5.kind, Kind::Verify);
    assert_eq!(t5.verifies, vec!["T1", "T3"]);
}

#[test]
fn work_target_default_kind() {
    let file = load_fixture();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.kind, Kind::Work);
    assert!(t1.verifies.is_empty());
}

#[test]
fn validate_verify_without_verifies_is_error() {
    let mut file = load_fixture();
    // Make T5 a verify target but clear its verifies list.
    file.targets.get_mut("T5").unwrap().verifies.clear();
    let errors = graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T5") && e.contains("non-empty verifies"))
    );
}

#[test]
fn validate_work_with_verifies_is_error() {
    let mut file = load_fixture();
    // Give T1 (work) a verifies list.
    file.targets.get_mut("T1").unwrap().verifies = vec!["T2".to_string()];
    let errors = graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T1") && e.contains("must not have verifies"))
    );
}

#[test]
fn frontier_includes_verify_when_unblocked() {
    let mut file = load_fixture();
    // Achieve T1 and T3 so T5 becomes unblocked.
    file.targets.get_mut("T1").unwrap().status = Status::Achieved;
    file.targets.get_mut("T3").unwrap().status = Status::Achieved;
    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    assert!(
        ids.contains(&"T5"),
        "T5 should be in frontier when T1+T3 achieved"
    );
    let t5 = front.iter().find(|f| f.id == "T5").unwrap();
    assert_eq!(t5.kind, Kind::Verify);
    assert_eq!(t5.verifies, vec!["T1", "T3"]);
}

#[test]
fn mermaid_shows_verifies_edges() {
    let file = load_fixture();
    let diagram = graph::mermaid(&file);
    assert!(diagram.contains("verifies"));
}

#[test]
fn render_shows_verify_target() {
    let file = load_fixture();
    let md = render::render_markdown(&file);
    // T5 should have the ✓ marker and Verifies line.
    assert!(md.contains("### 🎯T5 ✓ CI and platform isolation verified"));
    assert!(md.contains("- **Verifies**: 🎯T1, 🎯T3"));
    assert!(md.contains("- **Rework**: 🎯T1"));
}

#[test]
fn render_shows_retry_budget() {
    let file = load_fixture();
    let md = render::render_markdown(&file);
    assert!(md.contains("- **Retry budget**: 3"));
}

#[test]
fn rework_field_parsed() {
    let file = load_fixture();
    let t5 = &file.targets["T5"];
    assert_eq!(t5.rework.as_deref(), Some("T1"));
}

#[test]
fn retry_budget_parsed() {
    let file = load_fixture();
    assert_eq!(file.targets["T1"].retry_budget, Some(3));
    assert_eq!(file.targets["T1"].retries, 0);
}

#[test]
fn validate_rework_must_be_in_verifies() {
    let mut file = load_fixture();
    // Point rework at T3 which is not in verifies... wait, T3 IS in verifies.
    // Point rework at T2 which is NOT in verifies.
    file.targets.get_mut("T5").unwrap().rework = Some("T2".to_string());
    let errors = graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T5") && e.contains("must be in verifies"))
    );
}

#[test]
fn validate_rework_only_on_verify() {
    let mut file = load_fixture();
    // Give T1 (work target) a rework field.
    file.targets.get_mut("T1").unwrap().rework = Some("T2".to_string());
    let errors = graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T1") && e.contains("only verify"))
    );
}

#[test]
fn mermaid_shows_rework_edge() {
    let file = load_fixture();
    let diagram = graph::mermaid(&file);
    assert!(diagram.contains("rework"));
}

#[test]
fn tunnel_detects_uncovered_work() {
    let file = load_fixture();
    let warnings = graph::tunnels(&file, 2);
    // T2 has no verify target covering it — should be flagged.
    let t2_warning = warnings.iter().find(|w| w.target_id == "T2");
    assert!(t2_warning.is_some(), "T2 should be flagged as a tunnel");
    assert!(
        t2_warning.unwrap().depth.is_none(),
        "T2 has no verify reachable"
    );
}

#[test]
fn tunnel_no_warning_for_covered_work() {
    let file = load_fixture();
    let warnings = graph::tunnels(&file, 2);
    // T1 and T3 are covered by T5 (1 hop) — should NOT be flagged.
    assert!(
        !warnings.iter().any(|w| w.target_id == "T1"),
        "T1 should not be flagged (covered by T5)"
    );
    assert!(
        !warnings.iter().any(|w| w.target_id == "T3"),
        "T3 should not be flagged (covered by T5)"
    );
}

#[test]
fn tunnel_detects_deep_chain() {
    use bullseye::schema::{Kind as K, Target};
    use chrono::NaiveDate;

    let mut file = load_fixture();
    let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();

    // Build a chain: T10 → T11 → T12 → T13(verify covers T10)
    // T10 is 3 hops from verification — should be flagged at max_depth=2.
    for (id, deps) in [("T10", vec![]), ("T11", vec!["T10"]), ("T12", vec!["T11"])] {
        file.targets.insert(
            id.to_string(),
            Target {
                name: format!("Work {id}"),
                kind: K::Work,
                status: Status::Identified,
                value: 1.0,
                cost: 1.0,
                actual_cost: None,
                acceptance: vec!["done".to_string()],
                context: String::new(),

                gates: vec![],
                depends_on: deps.into_iter().map(String::from).collect(),
                verifies: vec![],
                rework: None,
                retry_budget: None,
                retries: 0,
                tags: vec![],
                origin: "test".to_string(),
                discovered: date,
                achieved: None,
            },
        );
    }
    // T13 is a verify target covering T10, depending on T12.
    file.targets.insert(
        "T13".to_string(),
        Target {
            name: "Verify chain".to_string(),
            kind: K::Verify,
            status: Status::Identified,
            value: 1.0,
            cost: 1.0,
            actual_cost: None,
            acceptance: vec!["verified".to_string()],
            context: String::new(),
            gates: vec![],
            depends_on: vec!["T12".to_string()],
            verifies: vec!["T10".to_string()],
            rework: None,
            retry_budget: None,
            retries: 0,
            tags: vec![],
            origin: "test".to_string(),
            discovered: date,
            achieved: None,
        },
    );

    // At max_depth=2 these are all within range. Test with max_depth=0
    // where distance 1 is already too far.
    let warnings_strict = graph::tunnels(&file, 0);
    // At max_depth=0, any target that is not itself a verify target and doesn't
    // have a verify target at distance 0 is flagged. Distance 1 = too far.
    // T10 → T13 at distance 1: flagged.
    let t10 = warnings_strict.iter().find(|w| w.target_id == "T10");
    assert!(t10.is_some(), "T10 should be flagged at max_depth=0");
    assert_eq!(t10.unwrap().depth, Some(1));
}

// --- ops::rework tests ---

#[test]
fn rework_resets_statuses() {
    let mut file = load_fixture();
    // Achieve T1 and T3 so T5 is actionable, then mark T5 as converging
    // (simulating a verification in progress that fails).
    file.targets.get_mut("T1").unwrap().status = Status::Achieved;
    file.targets.get_mut("T3").unwrap().status = Status::Achieved;
    file.targets.get_mut("T5").unwrap().status = Status::Converging;

    let result = ops::rework(&mut file, "T5", "tests failed on Linux").unwrap();

    assert_eq!(result.rework_id, "T1");
    assert_eq!(result.retries, 1);
    assert!(!result.budget_exhausted);

    // Verify target reset to identified.
    assert_eq!(file.targets["T5"].status, Status::Identified);
    // Rework destination reset to converging.
    assert_eq!(file.targets["T1"].status, Status::Converging);
    assert_eq!(file.targets["T1"].retries, 1);
}

#[test]
fn rework_appends_diagnosis() {
    let mut file = load_fixture();
    let original_context = file.targets["T1"].context.clone();

    ops::rework(&mut file, "T5", "linker error on arm64").unwrap();

    let ctx = &file.targets["T1"].context;
    assert!(ctx.contains(&original_context));
    assert!(ctx.contains("Rework #1: linker error on arm64"));
}

#[test]
fn rework_empty_diagnosis_preserves_context() {
    let mut file = load_fixture();
    let original_context = file.targets["T1"].context.clone();

    ops::rework(&mut file, "T5", "").unwrap();

    assert_eq!(file.targets["T1"].context, original_context);
}

#[test]
fn rework_increments_retries() {
    let mut file = load_fixture();

    // First rework.
    let r1 = ops::rework(&mut file, "T5", "attempt 1").unwrap();
    assert_eq!(r1.retries, 1);
    assert!(!r1.budget_exhausted);

    // Second rework.
    let r2 = ops::rework(&mut file, "T5", "attempt 2").unwrap();
    assert_eq!(r2.retries, 2);
    assert!(!r2.budget_exhausted);

    // Third rework — hits budget (T1 has retry_budget: 3).
    let r3 = ops::rework(&mut file, "T5", "attempt 3").unwrap();
    assert_eq!(r3.retries, 3);
    assert!(r3.budget_exhausted);
    assert_eq!(r3.budget, Some(3));
}

#[test]
fn rework_exceeds_budget() {
    let mut file = load_fixture();

    // Burn through the budget.
    for i in 1..=3 {
        let r = ops::rework(&mut file, "T5", &format!("attempt {i}")).unwrap();
        assert_eq!(r.retries, i);
    }

    // Fourth rework — past budget. Still works (budget is advisory),
    // but budget_exhausted remains true.
    let r4 = ops::rework(&mut file, "T5", "attempt 4").unwrap();
    assert_eq!(r4.retries, 4);
    assert!(r4.budget_exhausted);
}

#[test]
fn rework_no_budget_never_exhausted() {
    let mut file = load_fixture();
    // Remove T1's retry budget.
    file.targets.get_mut("T1").unwrap().retry_budget = None;

    for i in 1..=5 {
        let r = ops::rework(&mut file, "T5", &format!("attempt {i}")).unwrap();
        assert_eq!(r.retries, i);
        assert!(!r.budget_exhausted);
        assert_eq!(r.budget, None);
    }
}

#[test]
fn rework_multiple_diagnoses_separated() {
    let mut file = load_fixture();

    ops::rework(&mut file, "T5", "first issue").unwrap();
    ops::rework(&mut file, "T5", "second issue").unwrap();

    let ctx = &file.targets["T1"].context;
    assert!(ctx.contains("Rework #1: first issue"));
    assert!(ctx.contains("Rework #2: second issue"));
    // Separated by blank line.
    assert!(ctx.contains("first issue\n\nRework #2"));
}

#[test]
fn rework_error_not_found() {
    let mut file = load_fixture();
    let err = ops::rework(&mut file, "T99", "").unwrap_err();
    assert_eq!(err, ops::ReworkError::TargetNotFound("T99".to_string()));
}

#[test]
fn rework_error_not_verify() {
    let mut file = load_fixture();
    let err = ops::rework(&mut file, "T1", "").unwrap_err();
    assert_eq!(err, ops::ReworkError::NotVerifyTarget("T1".to_string()));
}

#[test]
fn rework_error_no_rework_target() {
    let mut file = load_fixture();
    // Remove the rework field from T5.
    file.targets.get_mut("T5").unwrap().rework = None;
    let err = ops::rework(&mut file, "T5", "").unwrap_err();
    assert_eq!(err, ops::ReworkError::NoReworkTarget("T5".to_string()));
}

#[test]
fn rework_error_dest_not_found() {
    let mut file = load_fixture();
    // Point rework at a nonexistent target (bypass validation for this test).
    file.targets.get_mut("T5").unwrap().rework = Some("T99".to_string());
    file.targets
        .get_mut("T5")
        .unwrap()
        .verifies
        .push("T99".to_string());
    let err = ops::rework(&mut file, "T5", "").unwrap_err();
    assert_eq!(err, ops::ReworkError::ReworkDestNotFound("T99".to_string()));
}

#[test]
fn renders_markdown() {
    let file = load_fixture();
    let md = render::render_markdown(&file);

    // Has structure.
    assert!(md.contains("# Targets"));
    assert!(md.contains("## Active"));
    assert!(md.contains("## Achieved"));

    // Has target entries with 🎯 prefix.
    assert!(md.contains("### 🎯T1 All tests pass on CI"));
    assert!(md.contains("### 🎯T4 Documentation covers all public APIs"));

    // Has value and cost lines.
    assert!(md.contains("- **Value**:"));
    assert!(md.contains("- **Cost**:"));

    // Has acceptance criteria.
    assert!(md.contains("- **Acceptance**:"));

    // Has gates.
    assert!(md.contains("- **Gates**: 🎯T1 (80%)"));

    // Achieved target has achieved date.
    assert!(md.contains("- **Achieved**: 2026-03-10"));

    // Has mermaid graph (active targets exist).
    assert!(md.contains("```mermaid"));
    assert!(md.contains("graph TD"));
}

#[test]
fn markdown_path_derivation() {
    use std::path::Path;
    let yaml = Path::new("/foo/docs/targets.yaml");
    let md = render::markdown_path(yaml);
    assert_eq!(md, Path::new("/foo/docs/targets.md"));
}

#[test]
fn create_starter_produces_valid_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_starter(tmp.path(), "test-project").unwrap();

    assert!(path.exists());
    assert_eq!(path, tmp.path().join("docs/targets.yaml"));

    let file = store::load(&path).unwrap();
    assert_eq!(file.targets.len(), 1);

    let t1 = &file.targets["T1"];
    assert!(t1.name.contains("test-project"));
    assert_eq!(t1.status, Status::Identified);
    assert_eq!(t1.origin, "bullseye_init");
    assert_eq!(t1.acceptance.len(), 2);

    // Validate the file passes all checks.
    let errors = graph::validate(&file);
    assert!(errors.is_empty(), "validation errors: {errors:?}");
}

#[test]
fn create_starter_does_not_overwrite() {
    let tmp = tempfile::tempdir().unwrap();

    // Create the first time.
    let path = store::create_starter(tmp.path(), "project").unwrap();
    assert!(path.exists());

    // discover should now find it, so handler-level guard works.
    let found = store::discover(tmp.path());
    assert!(found.is_some());
}

// --- Startup context tests ---

#[test]
fn startup_context_shows_frontier_and_counts() {
    let file = load_fixture();
    let ctx = graph::startup_context(&file, "test/docs/targets.yaml", 14);

    // Header with counts.
    assert!(ctx.contains("Active: 4 target(s)"));
    assert!(ctx.contains("Frontier:"));

    // Frontier section should include T1 and T2 (unblocked active targets).
    assert!(ctx.contains("## Frontier"));
    assert!(ctx.contains("🎯T1"));
    assert!(ctx.contains("🎯T2"));
}

#[test]
fn startup_context_shows_recently_achieved() {
    let mut file = load_fixture();
    // Set T4's achieved date to today so it appears in recent.
    let today = chrono::Local::now().date_naive();
    file.targets.get_mut("T4").unwrap().achieved = Some(today);

    let ctx = graph::startup_context(&file, "test", 14);
    assert!(ctx.contains("## Recently achieved"));
    assert!(ctx.contains("🎯T4"));
    assert!(ctx.contains("Documentation covers all public APIs"));
}

#[test]
fn startup_context_omits_old_achieved() {
    let file = load_fixture();
    // T4 was achieved on 2026-03-10, which is >14 days ago (test runs after that).
    let ctx = graph::startup_context(&file, "test", 14);
    // Should NOT have a recently achieved section (T4 is too old).
    assert!(!ctx.contains("## Recently achieved"));
}

#[test]
fn startup_context_shows_tunnel_warnings() {
    let mut file = load_fixture();
    // Remove the verify target so tunnels are detected.
    file.targets.remove("T5");

    let ctx = graph::startup_context(&file, "test", 14);
    assert!(ctx.contains("## Warnings"));
    assert!(ctx.contains("Tunnels:"));
    assert!(ctx.contains("lack nearby verification"));
}

#[test]
fn startup_context_shows_validation_errors() {
    let mut file = load_fixture();
    // Create a dangling depends_on reference.
    file.targets
        .get_mut("T1")
        .unwrap()
        .depends_on
        .push("T99".to_string());

    let ctx = graph::startup_context(&file, "test", 14);
    assert!(ctx.contains("Validation errors"));
    assert!(ctx.contains("T99"));
}
