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
    // Must be the structured VersionTooNew variant so callers can
    // discriminate it from Io/Parse errors. Also check the rendered
    // Display form carries enough detail for a human.
    match &err {
        store::LoadError::VersionTooNew {
            found, supported, ..
        } => {
            assert_eq!(*found, 999);
            assert!(*supported < 999);
        }
        other => panic!("expected VersionTooNew, got {other:?}"),
    }
    let rendered = err.to_string();
    assert!(rendered.contains("schema_version 999"), "got: {rendered}");
    assert!(rendered.contains("Upgrade bullseye"), "got: {rendered}");
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
    let scan = portfolio::discover_repos(&fixture, 3);
    // The fixture has a docs/targets.yaml at its root.
    assert_eq!(scan.repos.len(), 1);
    assert_eq!(scan.repos[0].active, 4);
    assert!(scan.repos[0].frontier > 0);
    assert_eq!(scan.repos[0].achieved, 1);
    // Fixture is clean — no warnings.
    assert!(scan.warnings.is_empty());
}

#[test]
fn portfolio_format_includes_frontier_targets() {
    use bullseye::portfolio;

    let fixture = fixture_path();
    let scan = portfolio::discover_repos(&fixture, 3);
    let out = portfolio::format_portfolio(&scan);
    assert!(out.contains("## Ready for work"));
    assert!(out.contains("🎯T1"));
}

#[test]
fn portfolio_reports_version_mismatch_as_warning() {
    use bullseye::portfolio::{self, RepoWarningKind};
    use std::io::Write;

    // A repo whose targets.yaml declares a newer schema_version than
    // this bullseye supports must appear as a warning in the scan
    // — NOT silently disappear from the repos list. This is the
    // whole reason the schema_version check exists: if portfolio
    // swallows the error, an outdated bullseye would hide the
    // "upgrade me" signal across every repo the user scans.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("org").join("future-repo");
    let docs = repo.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let path = docs.join("targets.yaml");
    write!(
        std::fs::File::create(&path).unwrap(),
        "schema_version: 999\ntargets:\n  T1:\n    name: From the future\n    \
         status: identified\n    value: 3\n    cost: 2\n    acceptance:\n      \
         - ok\n    discovered: 2026-04-01\n"
    )
    .unwrap();

    let scan = portfolio::discover_repos(tmp.path(), 5);
    assert!(
        scan.repos.is_empty(),
        "broken repo should not appear in repos list"
    );
    assert_eq!(scan.warnings.len(), 1, "expected one warning");
    assert_eq!(scan.warnings[0].kind, RepoWarningKind::VersionMismatch);
    assert!(scan.warnings[0].message.contains("999"));

    // And the formatted output surfaces it prominently.
    let out = portfolio::format_portfolio(&scan);
    assert!(out.contains("## ⚠ Warnings"));
    assert!(out.contains("Schema version mismatch"));
    assert!(out.contains("upgrade bullseye"));
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
    let out = graph::summary(&file, "test/docs/targets.yaml", None, false);

    // Header with counts.
    assert!(out.contains("Total: 5 target(s)"));
    assert!(out.contains("4 active"));
    assert!(out.contains("1 achieved"));

    // Has key sections. No WSJF ranking section any more — frontier
    // itself carries the focus ordering.
    assert!(out.contains("## Active targets by group"));
    assert!(out.contains("## Frontier"));
    assert!(!out.contains("## WSJF ranking"));
    assert!(!out.contains("WSJF"));
}

#[test]
fn summary_shows_frontier_targets() {
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);

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
    let out = graph::summary(&file, "test", None, false);

    // T5 depends on T1+T3 (not achieved), so it's blocked.
    assert!(out.contains("## Blocked targets"));
    assert!(out.contains("🎯T5"));
    assert!(out.contains("blocked by"));
}

#[test]
fn summary_frontier_ordered_by_value() {
    // No momentum → focus = value, so the frontier is sorted by
    // target value desc. The fixture's frontier is T1 (v=8), T3 (v=5),
    // T2 (v=3). T5 is a verify target blocked on T1+T3 so it's not
    // in the frontier; T4 is achieved.
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);

    let frontier_section = out
        .split("## Frontier")
        .nth(1)
        .expect("frontier section exists");
    let end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..end];

    let t1_pos = frontier_text.find("🎯T1").expect("T1 in frontier");
    let t3_pos = frontier_text.find("🎯T3").expect("T3 in frontier");
    let t2_pos = frontier_text.find("🎯T2").expect("T2 in frontier");

    assert!(
        t1_pos < t3_pos,
        "T1 (v=8) should rank above T3 (v=5); got: {frontier_text}"
    );
    assert!(
        t3_pos < t2_pos,
        "T3 (v=5) should rank above T2 (v=3); got: {frontier_text}"
    );
    // Without momentum, the plain baseline shape is used.
    assert!(frontier_text.contains("v=8"));
    assert!(!frontier_text.contains("focus"));
    assert!(!frontier_text.contains("momentum"));
}

// --- Convergence integration tests ---

fn write_project(tmp: &std::path::Path, makefile: &str, targets_yaml: &str) {
    use std::io::Write;
    let docs = tmp.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    write!(
        std::fs::File::create(docs.join("targets.yaml")).unwrap(),
        "{targets_yaml}"
    )
    .unwrap();
    write!(
        std::fs::File::create(tmp.join("Makefile")).unwrap(),
        "{makefile}"
    )
    .unwrap();
}

const SIMPLE_TARGETS_YAML: &str = r#"
schema_version: 1
targets:
  T1:
    name: Primary deliverable
    status: identified
    value: 8
    cost: 3
    acceptance:
      - Produces the primary artifact
      - Tests cover the happy path
    context: The highest-value thing in the project.
    discovered: 2026-04-01
  T2:
    name: Secondary polish
    status: identified
    value: 3
    cost: 2
    acceptance:
      - Rough edges smoothed
    discovered: 2026-04-01
"#;

#[test]
fn convergence_end_to_end_green_invariants_picks_top_frontier() {
    // Full integration: real temp project, real Makefile that exits 0,
    // real targets.yaml, real convergence pipeline. Verifies the whole
    // path from hook invocation to recommendation text.
    let tmp = tempfile::tempdir().unwrap();
    // `true` is a trivial program that exits 0 — standing invariants green.
    let makefile = "bullseye:\n\t@true\n";
    write_project(tmp.path(), makefile, SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("docs/targets.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    assert!(out.contains("# Convergence"));
    assert!(out.contains("## Invariants"));
    assert!(
        out.contains("Status: ✓ all green"),
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
    // No WSJF references anywhere in the convergence output.
    assert!(!out.to_lowercase().contains("wsjf"));
}

#[test]
fn convergence_end_to_end_red_invariants_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    // `false` exits 1 — standing invariants red.
    let makefile = "bullseye:\n\t@echo 'tests failing'; false\n";
    write_project(tmp.path(), makefile, SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("docs/targets.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    assert!(out.contains("Status: ✗ failed"));
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

    let path = tmp.path().join("docs/targets.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, true);

    assert!(out.contains("(skipped"));
    assert!(!out.contains("would have failed"));
    assert!(!out.contains("Status: ✗"));
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
fn convergence_missing_makefile_degrades_gracefully() {
    // A repo with targets.yaml but no Makefile. Convergence must
    // still run to completion — emit the target snapshot, mark
    // invariants as unknown with setup instructions embedded, and
    // still produce a frontier recommendation.
    let tmp = tempfile::tempdir().unwrap();
    let docs = tmp.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    use std::io::Write;
    write!(
        std::fs::File::create(docs.join("targets.yaml")).unwrap(),
        "{SIMPLE_TARGETS_YAML}"
    )
    .unwrap();
    // Note: NO Makefile.

    let path = docs.join("targets.yaml");
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

    let path = tmp.path().join("docs/targets.yaml");
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

    let yaml_path = path.join("docs/targets.yaml");
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
fn every_tool_emits_valid_json_schema() {
    // Regression test for the `bullseye_summary.momentum: BTreeMap`
    // incident: the rust-mcp-sdk JsonSchema derive silently fell
    // back to `type: "unknown"` for a field it couldn't schema-ify,
    // and the resulting tools/list response was rejected by the
    // Anthropic API as non-Draft-2020-12-compliant, blocking every
    // turn of every session that had bullseye registered. The bug
    // shipped as far as v0.9.0 before a user hit it.
    //
    // Assert that no tool's input schema contains any forbidden
    // patterns: `type: "unknown"` (the specific fallback), plus
    // empty/null types (also invalid).
    use bullseye::tools::TargetTools;

    let tools = TargetTools::tools();
    assert!(!tools.is_empty(), "expected non-empty tool list");

    for tool in &tools {
        let schema_json =
            serde_json::to_string(&tool.input_schema).expect("input_schema must serialize");

        // Forbidden: `type: "unknown"` anywhere in the schema.
        assert!(
            !schema_json.contains("\"type\":\"unknown\""),
            "tool `{}` emits a schema containing `\"type\":\"unknown\"`, which the \
             Anthropic API rejects: {schema_json}",
            tool.name,
        );
        // Forbidden: `type: null` or `type: ""` (both invalid).
        assert!(
            !schema_json.contains("\"type\":null") && !schema_json.contains("\"type\":\"\""),
            "tool `{}` emits a schema with a null or empty `type`: {schema_json}",
            tool.name,
        );
    }
}

#[test]
fn summary_momentum_reorders_frontier() {
    use std::collections::BTreeMap;

    // Baseline frontier ordering (value desc): T1 (v=8), T3 (v=5), T2 (v=3).
    // Apply momentum:
    //   T3 × 5.0 → focus 25.0 (jumps to top)
    //   T1 × 0.5 → focus  4.0 (drops below T2)
    //   T2 × 1.0 → focus  3.0 (default, no explicit entry)
    // Expected order desc: T3 (25), T1 (4), T2 (3).
    let file = load_fixture();
    let mut momentum = BTreeMap::new();
    momentum.insert("T3".to_string(), 5.0);
    momentum.insert("T1".to_string(), 0.5);
    let out = graph::summary(&file, "test", Some(&momentum), false);

    // No WSJF section at all.
    assert!(!out.contains("## WSJF ranking"));
    assert!(!out.contains("WSJF"));

    let frontier_section = out
        .split("## Frontier")
        .nth(1)
        .expect("frontier section exists");
    let end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..end];

    let t3_pos = frontier_text.find("🎯T3").expect("T3 in frontier");
    let t1_pos = frontier_text.find("🎯T1").expect("T1 in frontier");
    let t2_pos = frontier_text.find("🎯T2").expect("T2 in frontier");

    assert!(
        t3_pos < t1_pos,
        "T3 (focus 25.0) should rank above T1 (focus 4.0). Got:\n{frontier_text}"
    );
    assert!(
        t1_pos < t2_pos,
        "T1 (focus 4.0) should rank above T2 (focus 3.0). Got:\n{frontier_text}"
    );

    // Boosted and suppressed targets show both focus and momentum
    // annotations. Baseline entries (no explicit momentum entry) show
    // focus but elide the "× momentum" clause.
    assert!(frontier_text.contains("focus 25.0"));
    assert!(frontier_text.contains("focus 4.0"));
    assert!(frontier_text.contains("focus 3.0"));
    assert!(frontier_text.contains("momentum 5.00"));
    assert!(frontier_text.contains("momentum 0.50"));

    // T2 has no momentum entry — its line shows "(v=3)" without the
    // "× momentum" clause.
    let t2_line_end = frontier_text[t2_pos..]
        .find('\n')
        .map(|n| t2_pos + n)
        .unwrap_or(frontier_text.len());
    let t2_line = &frontier_text[t2_pos..t2_line_end];
    assert!(
        !t2_line.contains("× momentum"),
        "T2 has no momentum entry and should not show × momentum clause: {t2_line}"
    );
    assert!(
        t2_line.contains("focus 3.0"),
        "T2 line should still show focus when a momentum map is provided: {t2_line}"
    );
}

#[test]
fn summary_momentum_map_with_no_frontier_matches_is_baseline_order() {
    use std::collections::BTreeMap;

    // Only T5 (a verify target, blocked — NOT in frontier) has a
    // momentum entry. No frontier target is affected, so the
    // frontier order is identical to the no-momentum baseline.
    // But the annotation format switches to the "focus X (v=Y)" shape
    // because a momentum map was provided.
    let file = load_fixture();
    let mut momentum = BTreeMap::new();
    momentum.insert("T5".to_string(), 3.0);
    let out = graph::summary(&file, "test", Some(&momentum), false);

    let frontier_section = out
        .split("## Frontier")
        .nth(1)
        .expect("frontier section exists");
    let end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..end];

    // Order is baseline: T1 (v=8) > T3 (v=5) > T2 (v=3).
    let t1_pos = frontier_text.find("🎯T1").expect("T1 in frontier");
    let t3_pos = frontier_text.find("🎯T3").expect("T3 in frontier");
    let t2_pos = frontier_text.find("🎯T2").expect("T2 in frontier");
    assert!(t1_pos < t3_pos);
    assert!(t3_pos < t2_pos);

    // Annotation format is the has-momentum baseline shape: "focus X
    // (v=Y)". No "× momentum" clause because no frontier entry has a
    // non-default multiplier.
    assert!(frontier_text.contains("focus 8"));
    assert!(frontier_text.contains("focus 5"));
    assert!(frontier_text.contains("focus 3"));
    assert!(!frontier_text.contains("× momentum"));
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
    let out = graph::summary(&file, "test", None, false);
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

    let out = graph::summary(&file, "test", None, false);
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

    let out = graph::summary(&file, "test", None, false);
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
fn load_parse_error_is_structured() {
    use std::io::Write;
    // A targets.yaml that exists but is syntactically broken should
    // return LoadError::Parse — the typed variant lets callers like
    // bullseye_startup_context choose to degrade gracefully instead
    // of surfacing a raw tool-call error.
    let tmp = tempfile::tempdir().unwrap();
    let docs = tmp.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let path = docs.join("targets.yaml");

    // Deliberately malformed: unterminated list + stray colon.
    let broken_yaml = "targets:\n  T1:\n    name: [unterminated\n    status:::\n";
    write!(std::fs::File::create(&path).unwrap(), "{broken_yaml}").unwrap();

    let err = store::load(&path).unwrap_err();
    assert!(
        matches!(err, store::LoadError::Parse(_)),
        "expected Parse, got {err:?}"
    );
}

#[test]
fn startup_context_broken_file_is_graceful() {
    // The helper that formats the degraded response for a broken
    // targets.yaml must surface the error without looking like a
    // tool-call failure — session start should continue.
    let out = graph::startup_context_broken_file(
        "/tmp/fake/docs/targets.yaml",
        "failed to parse /tmp/fake/docs/targets.yaml: invalid YAML at line 4",
    );
    assert!(out.contains("# Startup context"));
    assert!(out.contains("/tmp/fake/docs/targets.yaml"));
    assert!(out.contains("could not be loaded"));
    assert!(out.contains("invalid YAML at line 4"));
    assert!(out.contains("Session start is continuing"));
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
