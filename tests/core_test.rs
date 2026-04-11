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
    // T5 depends on T1 and T3 — should have "needs" edges.
    assert!(diagram.contains("needs"));
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
fn load_accepts_legacy_file_without_schema_version() {
    use bullseye::schema::CURRENT_SCHEMA_VERSION;
    use std::io::Write;
    // A targets.yaml written before schema_version was introduced must
    // still load cleanly. The loader treats the missing field as the
    // current (v1) schema and fills it in so the next save stamps it.
    let tmp = tempfile::tempdir().unwrap();
    let docs = tmp.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let path = docs.join("targets.yaml");

    let legacy_yaml = r#"
targets:
  T1:
    name: Legacy target
    status: identified
    value: 3
    cost: 2
    acceptance:
      - it works
    discovered: 2026-04-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{legacy_yaml}").unwrap();

    let file = store::load(&path).unwrap();
    assert_eq!(file.schema_version, Some(CURRENT_SCHEMA_VERSION));
    assert_eq!(file.targets.len(), 1);
}

#[test]
fn load_rejects_newer_schema_version_with_upgrade_prompt() {
    use std::io::Write;
    // A targets.yaml declaring a schema_version higher than this
    // binary supports must fail fast with a clear upgrade message,
    // not silently drop or misinterpret unknown fields.
    let tmp = tempfile::tempdir().unwrap();
    let docs = tmp.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let path = docs.join("targets.yaml");

    let future_yaml = r#"
schema_version: 999
targets:
  T1:
    name: From the future
    status: identified
    value: 3
    cost: 2
    acceptance:
      - it works
    discovered: 2026-04-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{future_yaml}").unwrap();

    let err = store::load(&path).unwrap_err();
    assert!(err.contains("schema_version 999"), "got: {err}");
    assert!(err.contains("Upgrade bullseye"), "got: {err}");
}

#[test]
fn save_stamps_current_schema_version() {
    use bullseye::schema::CURRENT_SCHEMA_VERSION;
    use std::io::Write;
    // Loading a legacy file and re-saving must produce a file with
    // the current schema_version on disk, so legacy files self-upgrade
    // on first contact with a v0.9.0+ bullseye.
    let tmp = tempfile::tempdir().unwrap();
    let docs = tmp.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let path = docs.join("targets.yaml");

    let legacy_yaml = r#"
targets:
  T1:
    name: Legacy target
    status: identified
    value: 3
    cost: 2
    acceptance:
      - it works
    discovered: 2026-04-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{legacy_yaml}").unwrap();

    let file = store::load(&path).unwrap();
    store::save(&path, &file).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains(&format!("schema_version: {CURRENT_SCHEMA_VERSION}")),
        "expected schema_version stamp; got:\n{after}"
    );
}

#[test]
fn load_migrates_legacy_gates_to_depends_on() {
    use std::io::Write;
    // Write a legacy YAML with the old `gates` field and verify that
    // `T2.gates = [T1]` folds into `T2.depends_on += [T1]` — i.e., the
    // owning target absorbs its gates as blockers ("T2 is gated by T1").
    let tmp = tempfile::tempdir().unwrap();
    let docs = tmp.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let path = docs.join("targets.yaml");

    let legacy_yaml = r#"
targets:
  T1:
    name: Upstream prerequisite
    status: identified
    value: 3
    cost: 2
    acceptance:
      - prerequisite is ready
    discovered: 2026-04-01
  T2:
    name: Downstream work
    status: identified
    value: 3
    cost: 2
    acceptance:
      - it works
    gates:
      - target: T1
        criticality: 0.8
    discovered: 2026-04-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{legacy_yaml}").unwrap();

    let file = store::load(&path).unwrap();
    // T2 should now depend on T1, because T2 was gated by T1.
    assert_eq!(file.targets["T2"].depends_on, vec!["T1"]);
    assert!(file.targets["T1"].depends_on.is_empty());
    // Both targets should have empty gates after migration.
    assert!(file.targets["T1"].gates.is_empty());
    assert!(file.targets["T2"].gates.is_empty());
    // And the frontier reflects the new blocking edge.
    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    assert!(ids.contains(&"T1"), "T1 is unblocked");
    assert!(!ids.contains(&"T2"), "T2 is blocked by T1");
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

// --- Portfolio tests ---

#[test]
fn portfolio_discovers_fixture() {
    use bullseye::portfolio;

    let fixture = fixture_path();
    let repos = portfolio::discover_repos(&fixture, 3);
    // The fixture has a docs/targets.yaml at its root.
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].active, 4);
    assert!(repos[0].frontier > 0);
    assert_eq!(repos[0].achieved, 1);
}

#[test]
fn portfolio_format_includes_frontier_targets() {
    use bullseye::portfolio;

    let fixture = fixture_path();
    let repos = portfolio::discover_repos(&fixture, 3);
    let out = portfolio::format_portfolio(&repos);
    assert!(out.contains("## Ready for work"));
    assert!(out.contains("🎯T1"));
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

// --- Summary tests ---

#[test]
fn summary_shows_totals_and_sections() {
    let file = load_fixture();
    let out = graph::summary(&file, "test/docs/targets.yaml", 5, None);

    // Header with counts.
    assert!(out.contains("Total: 5 target(s)"));
    assert!(out.contains("4 active"));
    assert!(out.contains("1 achieved"));

    // Has key sections.
    assert!(out.contains("## Active targets by group"));
    assert!(out.contains("## Frontier"));
    assert!(out.contains("## WSJF ranking"));
}

#[test]
fn summary_shows_frontier_targets() {
    let file = load_fixture();
    let out = graph::summary(&file, "test", 5, None);

    // Frontier should include T1, T2, T3 (unblocked).
    let frontier_section = out.split("## Frontier").nth(1).unwrap();
    let frontier_end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..frontier_end];
    assert!(frontier_text.contains("🎯T1"));
    assert!(frontier_text.contains("🎯T2"));
    assert!(frontier_text.contains("🎯T3"));
}

#[test]
fn summary_shows_blocked_targets() {
    let file = load_fixture();
    let out = graph::summary(&file, "test", 5, None);

    // T5 depends on T1+T3 (not achieved), so it's blocked.
    assert!(out.contains("## Blocked targets"));
    assert!(out.contains("🎯T5"));
    assert!(out.contains("blocked by"));
}

#[test]
fn summary_wsjf_ranking_ordered() {
    let file = load_fixture();
    let out = graph::summary(&file, "test", 5, None);

    // T5: v=3, c=1, WSJF=3.0
    // T1: v=8, c=3, WSJF=2.67
    // T3: v=5, c=5, WSJF=1.0 (but also has gate on T1)
    // T2: v=3, c=2, WSJF=1.5
    // Order should be T5(3.0), T1(2.67), T2(1.5), T3(1.0)
    let wsjf_section = out.split("## WSJF ranking").nth(1).unwrap();
    let t5_pos = wsjf_section.find("🎯T5").unwrap();
    let t1_pos = wsjf_section.find("🎯T1").unwrap();
    let t2_pos = wsjf_section.find("🎯T2").unwrap();
    let t3_pos = wsjf_section.find("🎯T3").unwrap();
    assert!(t5_pos < t1_pos, "T5 (WSJF 3.0) should rank above T1 (2.67)");
    assert!(t1_pos < t2_pos, "T1 (WSJF 2.67) should rank above T2 (1.5)");
    assert!(t2_pos < t3_pos, "T2 (WSJF 1.5) should rank above T3 (1.0)");
}

#[test]
fn summary_top_n_limits_wsjf() {
    let file = load_fixture();
    let out = graph::summary(&file, "test", 2, None);

    // Should only show top 2.
    assert!(out.contains("## WSJF ranking (top 2)"));
    let wsjf_section = out.split("## WSJF ranking").nth(1).unwrap();
    // Should have T5 and T1 but not T2 or T3.
    assert!(wsjf_section.contains("🎯T5"));
    assert!(wsjf_section.contains("🎯T1"));
    assert!(!wsjf_section.contains("🎯T2"));
    assert!(!wsjf_section.contains("🎯T3"));
}

#[test]
fn summary_momentum_reorders_ranking() {
    use std::collections::BTreeMap;

    // Baseline: T5(3.0) > T1(2.67) > T2(1.5) > T3(1.0).
    // Give T3 a 5x momentum boost → adjusted 5.0, should jump to the top.
    // Give T1 a 0.5x suppression → adjusted 1.33, should drop below T2(1.5).
    // Expected order: T3(5.0), T5(3.0), T2(1.5), T1(1.33).
    let file = load_fixture();
    let mut momentum = BTreeMap::new();
    momentum.insert("T3".to_string(), 5.0);
    momentum.insert("T1".to_string(), 0.5);
    let out = graph::summary(&file, "test", 5, Some(&momentum));

    // Section heading is labelled momentum-adjusted so consumers can
    // distinguish from the baseline rendering.
    assert!(out.contains("## WSJF ranking, momentum-adjusted"));

    let wsjf_section = out.split("## WSJF ranking").nth(1).unwrap();
    let t3_pos = wsjf_section.find("🎯T3").expect("T3 in ranking");
    let t5_pos = wsjf_section.find("🎯T5").expect("T5 in ranking");
    let t2_pos = wsjf_section.find("🎯T2").expect("T2 in ranking");
    let t1_pos = wsjf_section.find("🎯T1").expect("T1 in ranking");

    assert!(
        t3_pos < t5_pos,
        "T3 (boosted to 5.0) should rank above T5 (3.0). Got: {wsjf_section}"
    );
    assert!(t5_pos < t2_pos, "T5 (3.0) should rank above T2 (1.5)");
    assert!(
        t2_pos < t1_pos,
        "T2 (1.5) should rank above T1 (suppressed to 1.33). Got: {wsjf_section}"
    );

    // Adjusted entries should show the momentum annotation; T2 (no
    // entry in the map) should render with the plain WSJF form.
    assert!(wsjf_section.contains("momentum 5.00"));
    assert!(wsjf_section.contains("momentum 0.50"));
    // T2 has no momentum entry — default 1.0 → plain form, no
    // "momentum 1.00" annotation in its line.
    let t2_line_end = wsjf_section[t2_pos..]
        .find('\n')
        .map(|n| t2_pos + n)
        .unwrap_or(wsjf_section.len());
    let t2_line = &wsjf_section[t2_pos..t2_line_end];
    assert!(
        !t2_line.contains("momentum"),
        "T2 has no momentum entry and should render without the annotation: {t2_line}"
    );
}

#[test]
fn summary_momentum_missing_entries_default_to_one() {
    use std::collections::BTreeMap;

    // Only T5 has a momentum entry, and it's exactly 1.0. The result
    // must be identical to the baseline ordering — a no-op multiplier
    // on one target, default 1.0 on the rest.
    let file = load_fixture();
    let mut momentum = BTreeMap::new();
    momentum.insert("T5".to_string(), 1.0);
    let out = graph::summary(&file, "test", 5, Some(&momentum));

    let wsjf_section = out.split("## WSJF ranking").nth(1).unwrap();
    let t5_pos = wsjf_section.find("🎯T5").unwrap();
    let t1_pos = wsjf_section.find("🎯T1").unwrap();
    let t2_pos = wsjf_section.find("🎯T2").unwrap();
    let t3_pos = wsjf_section.find("🎯T3").unwrap();
    // Identical baseline order: T5 > T1 > T2 > T3.
    assert!(t5_pos < t1_pos);
    assert!(t1_pos < t2_pos);
    assert!(t2_pos < t3_pos);
}

#[test]
fn summary_without_momentum_matches_baseline_heading() {
    // When `momentum` is None, the heading stays as the legacy
    // "## WSJF ranking (top N)" form — unchanged from pre-v0.9.0
    // callers that didn't pass the new argument.
    let file = load_fixture();
    let out = graph::summary(&file, "test", 5, None);
    assert!(out.contains("## WSJF ranking (top "));
    assert!(!out.contains("momentum-adjusted"));
}

#[test]
fn summary_stale_parent_all_children_achieved() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    let mut file = load_fixture();
    let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();

    // Add sub-targets T1.1 and T1.2, both achieved.
    for sub in ["T1.1", "T1.2"] {
        file.targets.insert(
            sub.to_string(),
            Target {
                name: format!("Sub {sub}"),
                kind: Kind::Work,
                status: Status::Achieved,
                value: 2.0,
                cost: 1.0,
                actual_cost: None,
                acceptance: vec!["done".to_string()],
                context: String::new(),
                gates: vec![],
                depends_on: vec![],
                verifies: vec![],
                rework: None,
                retry_budget: None,
                retries: 0,
                tags: vec![],
                origin: "test".to_string(),
                discovered: date,
                achieved: Some(date),
            },
        );
    }

    // T1 is converging but both children are achieved — stale.
    let out = graph::summary(&file, "test", 5, None);
    assert!(out.contains("## Stale targets"));
    assert!(out.contains("all sub-targets achieved"));
}

#[test]
fn summary_shows_grouped_children() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    let mut file = load_fixture();
    let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();

    // Add sub-target T1.1 (active).
    file.targets.insert(
        "T1.1".to_string(),
        Target {
            name: "Sub-target of T1".to_string(),
            kind: Kind::Work,
            status: Status::Identified,
            value: 2.0,
            cost: 1.0,
            actual_cost: None,
            acceptance: vec!["done".to_string()],
            context: String::new(),
            gates: vec![],
            depends_on: vec![],
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

    let out = graph::summary(&file, "test", 5, None);
    // T1 should show a rollup count.
    assert!(out.contains("achieved)"));
    // T1.1 should appear indented under T1.
    assert!(out.contains("🎯T1.1"));
}

#[test]
fn summary_with_validation_errors_skips_frontier() {
    let mut file = load_fixture();
    // Create a dangling depends_on reference.
    file.targets
        .get_mut("T1")
        .unwrap()
        .depends_on
        .push("T99".to_string());

    let out = graph::summary(&file, "test", 5, None);
    assert!(out.contains("## Validation errors"));
    assert!(out.contains("T99"));
    // Should NOT have frontier or blocked sections.
    assert!(!out.contains("## Frontier"));
    assert!(!out.contains("## Blocked"));
}

#[test]
fn startup_context_no_file_is_graceful() {
    // A repo with no targets.yaml must not make startup_context fail
    // outright — the session-start hook that typically invokes it runs
    // before the agent knows whether the repo uses bullseye. Return a
    // friendly "not using bullseye yet" message instead.
    let tmp = tempfile::tempdir().unwrap();
    // Sanity check: discover returns None on a fresh empty dir.
    assert!(store::discover(tmp.path()).is_none());

    let out = graph::startup_context_no_file(&tmp.path().display().to_string());
    assert!(out.contains("# Startup context"));
    assert!(out.contains("no targets.yaml found"));
    assert!(out.contains("bullseye_init"));
    // Must not look like an error string — agents should be able to
    // keep going.
    assert!(!out.to_lowercase().contains("error"));
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
