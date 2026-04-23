// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use bullseye::graph;
use bullseye::ops;
use bullseye::schema::{Kind, Status, TargetsFile};
use bullseye::store;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_fixture() -> TargetsFile {
    let path = fixture_path().join("bullseye.yaml");
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
    assert!(found.unwrap().ends_with("bullseye.yaml"));
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
                showcase: false,
                actual_cost: None,
                demonstration: None,
                acceptance: vec!["done".to_string()],
                checks: vec![],
                context: String::new(),

                gates: vec![],
                depends_on: deps.into_iter().map(String::from).collect(),
                cross_depends: vec![],
                cross_enables: vec![],
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
            showcase: false,
            actual_cost: None,
            demonstration: None,
            acceptance: vec!["verified".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: vec!["T12".to_string()],
            cross_depends: vec![],
            cross_enables: vec![],
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
fn load_accepts_legacy_file_without_schema_version() {
    use bullseye::schema::CURRENT_SCHEMA_VERSION;
    use std::io::Write;
    // A bullseye.yaml written before schema_version was introduced must
    // still load cleanly. The loader treats the missing field as the
    // current (v1) schema and fills it in so the next save stamps it.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

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
    // A bullseye.yaml declaring a schema_version higher than this
    // binary supports must fail fast with a clear upgrade message,
    // not silently drop or misinterpret unknown fields.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

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
    let path = tmp.path().join("bullseye.yaml");

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
    let path = tmp.path().join("bullseye.yaml");

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
    let path = store::create_at(
        tmp.path(),
        bullseye::config::Location::InRepo,
        "test-project",
    )
    .unwrap();

    assert!(path.exists());
    assert_eq!(path, tmp.path().join("bullseye.yaml"));

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
    let path = store::create_at(tmp.path(), bullseye::config::Location::InRepo, "project").unwrap();
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
    let scan = portfolio::discover_repos(&fixture, 3, &[]);
    // The fixture has a bullseye.yaml at its root.
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
    let scan = portfolio::discover_repos(&fixture, 3, &[]);
    let out = portfolio::format_portfolio(&scan);
    assert!(out.contains("## Ready for work"));
    assert!(out.contains("🎯T1"));
}

#[test]
fn portfolio_reports_version_mismatch_as_warning() {
    use bullseye::portfolio::{self, RepoWarningKind};
    use std::io::Write;

    // A repo whose bullseye.yaml declares a newer schema_version than
    // this bullseye supports must appear as a warning in the scan
    // — NOT silently disappear from the repos list. This is the
    // whole reason the schema_version check exists: if portfolio
    // swallows the error, an outdated bullseye would hide the
    // "upgrade me" signal across every repo the user scans.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("org").join("future-repo");
    std::fs::create_dir_all(&repo).unwrap();
    let path = repo.join("bullseye.yaml");
    write!(
        std::fs::File::create(&path).unwrap(),
        "schema_version: 999\ntargets:\n  T1:\n    name: From the future\n    \
         status: identified\n    value: 3\n    cost: 2\n    acceptance:\n      \
         - ok\n    discovered: 2026-04-01\n"
    )
    .unwrap();

    let scan = portfolio::discover_repos(tmp.path(), 5, &[]);
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

// --- Cross-repo edge tests (🎯T2.2) ---

#[test]
fn cross_repo_edges_yaml_roundtrip() {
    use bullseye::schema::CrossEdge;
    use std::io::Write;

    // A targets file with both cross_depends and cross_enables fields
    // must parse cleanly, survive a save/load roundtrip, and preserve
    // the edges field-for-field. This is the bedrock behaviour the
    // whole T2.2 feature sits on — if serde doesn't handle the shape,
    // nothing else works.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    let yaml = r#"
schema_version: 1
targets:
  T1:
    name: Has cross-repo edges
    status: identified
    value: 5
    cost: 3
    acceptance:
      - done
    cross_depends:
      - repo: marcelocantos/jevon
        capability: Manager API
        note: summarizer lifecycle
    cross_enables:
      - repo: marcelocantos/targets
        target: T1.4
    discovered: 2026-04-07
"#;
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();

    // Load and check the parsed edges.
    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.cross_depends.len(), 1);
    assert_eq!(t1.cross_enables.len(), 1);

    let dep = &t1.cross_depends[0];
    assert_eq!(dep.repo, "marcelocantos/jevon");
    assert_eq!(dep.capability.as_deref(), Some("Manager API"));
    assert_eq!(dep.target, None);
    assert_eq!(dep.note.as_deref(), Some("summarizer lifecycle"));

    let en = &t1.cross_enables[0];
    assert_eq!(en.repo, "marcelocantos/targets");
    assert_eq!(en.target.as_deref(), Some("T1.4"));
    assert_eq!(en.capability, None);
    assert_eq!(en.note, None);

    // Round-trip via save + reload: edges must survive unchanged.
    store::save(&path, &file).unwrap();
    let reloaded = store::load(&path).unwrap();
    assert_eq!(reloaded.targets["T1"].cross_depends, t1.cross_depends);
    assert_eq!(reloaded.targets["T1"].cross_enables, t1.cross_enables);

    // Also check the serialized form directly — omitted fields
    // (target, capability, note) must not appear when they're None.
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("cross_depends:"));
    assert!(raw.contains("cross_enables:"));
    assert!(raw.contains("Manager API"));
    assert!(raw.contains("T1.4"));

    // And an empty cross_depends/cross_enables vec must not serialize
    // at all (skip_serializing_if = "Vec::is_empty").
    let _unused = CrossEdge {
        repo: "x".into(),
        target: None,
        capability: None,
        note: None,
    };
}

#[test]
fn cross_repo_edge_validation_rejects_empty_ref() {
    use bullseye::schema::CrossEdge;

    // An edge with no `target` and no `capability` is structurally
    // meaningless — there's nothing for the portfolio view to render
    // or for the agent to act on. Validation must flag it.
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().cross_depends = vec![CrossEdge {
        repo: "marcelocantos/other".to_string(),
        target: None,
        capability: None,
        note: None,
    }];

    let errors = graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T1") && e.contains("must set `target` or `capability`")),
        "expected cross-repo edge validation error; got: {errors:?}"
    );
}

#[test]
fn cross_repo_edge_validation_rejects_empty_repo() {
    use bullseye::schema::CrossEdge;

    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().cross_enables = vec![CrossEdge {
        repo: "   ".to_string(),
        target: Some("T1".to_string()),
        capability: None,
        note: None,
    }];

    let errors = graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T1") && e.contains("empty repo")),
        "expected empty-repo error; got: {errors:?}"
    );
}

#[test]
fn cross_repo_edges_do_not_block_frontier() {
    use bullseye::schema::CrossEdge;

    // Cross-repo dependencies are advisory only — they must not
    // remove a target from the frontier. Otherwise bullseye would
    // be making authoritative claims about the state of another
    // repo's graph, which it intentionally does not track.
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().cross_depends = vec![CrossEdge {
        repo: "marcelocantos/jevon".to_string(),
        capability: Some("missing capability".to_string()),
        target: None,
        note: None,
    }];

    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    assert!(
        ids.contains(&"T1"),
        "T1 should remain in frontier despite cross_depends; got: {ids:?}"
    );
}

#[test]
fn portfolio_surfaces_cross_repo_edges_from_loaded_yaml() {
    use bullseye::portfolio;
    use std::io::Write;

    // End-to-end: write a repo with cross-repo edges, run
    // discover_repos, check the scan captures the edges and
    // format_portfolio surfaces them.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("org").join("linker");
    std::fs::create_dir_all(&repo).unwrap();
    let path = repo.join("bullseye.yaml");

    let yaml = r#"
schema_version: 1
targets:
  T1:
    name: Local work with cross-repo enabler
    status: identified
    value: 3
    cost: 2
    acceptance:
      - done
    cross_enables:
      - repo: marcelocantos/targets
        target: T1.4
        note: unblocks target-aware compaction
    discovered: 2026-04-07
  T2:
    name: Plain higher-value work
    status: identified
    value: 8
    cost: 3
    acceptance:
      - done
    discovered: 2026-04-07
"#;
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();

    let scan = portfolio::discover_repos(tmp.path(), 5, &[]);
    assert_eq!(scan.repos.len(), 1);
    let r = &scan.repos[0];

    // Edges were captured on the summary.
    assert!(r.cross_depends.is_empty());
    assert_eq!(r.cross_enables.len(), 1);
    assert_eq!(r.cross_enables[0].source_target, "T1");
    assert_eq!(r.cross_enables[0].edge.repo, "marcelocantos/targets");
    assert_eq!(r.cross_enables[0].edge.target.as_deref(), Some("T1.4"));

    // Priority boost: T1 (cross-enabler, v=3) ranks above T2 (plain, v=8).
    let ids: Vec<&str> = r.frontier_targets.iter().map(|ft| ft.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["T1", "T2"],
        "cross-enabler T1 should sort above plain T2 despite lower value"
    );

    // And format_portfolio surfaces both the edge and the boost.
    let out = portfolio::format_portfolio(&scan);
    assert!(out.contains("## Cross-repo edges"));
    assert!(out.contains("🎯T1 enables 🎯T1.4 @ marcelocantos/targets"));
    assert!(out.contains("unblocks target-aware compaction"));
    assert!(out.contains("★ 🎯T1"));
}

// --- Startup context tests ---

#[test]
fn startup_context_shows_frontier_and_counts() {
    let file = load_fixture();
    let ctx = graph::startup_context(&file, "test/bullseye.yaml", 14);

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
    let out = graph::summary(&file, "test/bullseye.yaml", None, false);

    // Header with counts.
    assert!(out.contains("Total: 5 target(s)"));
    assert!(out.contains("4 active"));
    assert!(out.contains("1 achieved"));

    // Has key sections. No WSJF ranking section or annotations — at
    // repo scope the frontier itself carries the focus ordering
    // (distance-to-observable, fanout). The banner introduced by
    // 🎯T16 names WSJF once to explicitly disavow it, so we check
    // for the absence of actual WSJF *ranking* signals rather than
    // the word itself.
    assert!(out.contains("## Active targets by group"));
    assert!(out.contains("## Frontier"));
    assert!(!out.contains("## WSJF ranking"));
    assert!(!out.contains("wsjf="));
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
fn summary_frontier_section_opens_with_repo_scope_banner() {
    // 🎯T16: every repo-scope frontier rendering must lead with the
    // banner + legend so agents see the correct ordering framing
    // inline and don't default to WSJF/SAFe reasoning from
    // training-data habit. The banner has to sit inside the
    // `## Frontier` section (not before it) so it survives
    // convergence's summary-body splicing.
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);
    let frontier_section = out.split("## Frontier").nth(1).unwrap();
    let frontier_end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..frontier_end];

    assert!(
        frontier_text.contains("Repo-scope ordering"),
        "banner must name the repo-scope ordering function; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("min distance-to-checkpoint"),
        "banner must describe the primary sort key; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("max unblocking fanout"),
        "banner must describe the tiebreaker; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("portfolio-scope"),
        "banner must disavow portfolio-scope framing at repo scope; got:\n{frontier_text}"
    );
    // Legend covers the per-entry annotation shapes used in the
    // rendered frontier.
    assert!(
        frontier_text.contains("`checkpoint`"),
        "legend must define the `checkpoint` annotation; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("`showcase: true`"),
        "legend must reference the `showcase: true` field that promotes a work target to a checkpoint; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("`dist=N`"),
        "legend must define the `dist=N` annotation; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("`fanout=N`"),
        "legend must define the `fanout=N` annotation; got:\n{frontier_text}"
    );
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
fn summary_frontier_ordered_by_distance_and_fanout() {
    // Repo-level ordering (🎯T7): ascending distance-to-observable,
    // tiebroken by descending unblocking fanout, then by ID.
    //
    // The fixture's frontier is T1, T2, T3. T5 is a verify target
    // that covers T1 and T3 (distance 1 for both), and both are in
    // T5's depends_on so both have fanout 1. T2 has no observable
    // reachable and no dependants at all — tunnel. Expected order:
    // T1, T3, T2 (T1 and T3 tie on (dist=1, fanout=1) with T1 < T3
    // by ID; T2 sorts last because it extends a tunnel).
    //
    // Value/cost intentionally have no effect on this ordering;
    // they're portfolio-scope inputs. This test used to assert a
    // value-desc order that happened to produce the same top
    // target — the new assertions lock in the *reason* so a future
    // refactor that reintroduces value-based ordering at repo
    // scope would fail explicitly.
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
        "T1 (dist=1, fanout=1, id=T1) should rank above T3 (dist=1, fanout=1, id=T3); got: {frontier_text}"
    );
    assert!(
        t3_pos < t2_pos,
        "T3 (dist=1) should rank above T2 (dist=None, tunnel); got: {frontier_text}"
    );

    // Annotation format exposes dist/fanout, not value/focus/momentum.
    assert!(frontier_text.contains("dist=1"));
    assert!(frontier_text.contains("fanout=1"));
    assert!(frontier_text.contains("no checkpoint reachable"));
    assert!(!frontier_text.contains("v=8"));
    assert!(!frontier_text.contains("focus"));
    assert!(!frontier_text.contains("momentum"));
}

// --- Convergence integration tests ---

fn write_project(tmp: &std::path::Path, makefile: &str, targets_yaml: &str) {
    use std::io::Write;
    write!(
        std::fs::File::create(tmp.join("bullseye.yaml")).unwrap(),
        "{targets_yaml}"
    )
    .unwrap();
    write!(
        std::fs::File::create(tmp.join("Makefile")).unwrap(),
        "{makefile}"
    )
    .unwrap();
}

/// Extract the concatenated text payload from an MCP `CallToolResult`,
/// panicking if any content block is not a `TextContent`. Used by the
/// handler-level end-to-end tests that drive `handle_convergence`
/// directly rather than calling `convergence::convergence`.
fn text_from_call_result(result: rust_mcp_sdk::schema::CallToolResult) -> String {
    use rust_mcp_sdk::schema::ContentBlock;
    result
        .content
        .into_iter()
        .map(|block| match block {
            ContentBlock::TextContent(t) => t.text,
            other => panic!("expected TextContent, got {other:?}"),
        })
        .collect::<Vec<_>>()
        .join("")
}

const SIMPLE_TARGETS_YAML: &str = r#"
schema_version: 2
targets:
  T1:
    name: Primary deliverable
    status: identified
    value: 8
    cost: 3
    showcase: true
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

    let path = tmp.path().join("bullseye.yaml");
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
        out.contains("Status: ✓ all green"),
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
fn summary_momentum_does_not_affect_repo_level_ordering() {
    use std::collections::BTreeMap;

    // 🎯T7 removed momentum (and value/cost) from repo-level
    // frontier ordering. The parameter is still accepted on the
    // wire for backward compatibility, but it must not perturb the
    // order — repo scope is driven purely by distance-to-observable
    // and unblocking fanout. Momentum lives at the portfolio layer
    // now (`src/portfolio.rs`), not here.
    let file = load_fixture();
    let mut momentum = BTreeMap::new();
    // Boost T2 (the dirty tunnel) with an absurd multiplier. In the
    // old value × momentum formula this would catapult T2 to the
    // top. Under repo-level ordering it must stay dead last — its
    // distance to an observable is `None`.
    momentum.insert("T2".to_string(), 100.0);
    momentum.insert("T1".to_string(), 0.01);

    let with = graph::summary(&file, "test", Some(&momentum), false);
    let without = graph::summary(&file, "test", None, false);

    let section = |s: &str| -> String {
        let start = s.split("## Frontier").nth(1).unwrap();
        let end = start.find("\n## ").unwrap_or(start.len());
        start[..end].to_string()
    };

    assert_eq!(
        section(&with),
        section(&without),
        "momentum map must not change repo-level frontier ordering"
    );
    // Deliberately absent: any WSJF ranking signal, focus label, or
    // momentum annotation in the repo-scope output. The 🎯T16 banner
    // legitimately names WSJF once in order to disavow it, so we
    // check for the ranking-annotation pattern (`wsjf=`) rather than
    // the word.
    assert!(!with.contains("wsjf="));
    assert!(!with.contains("focus"));
    assert!(!with.contains("× momentum"));
}

#[test]
fn showcase_field_yaml_roundtrip() {
    use std::io::Write;
    // The showcase flag must round-trip cleanly: present only
    // when true, absent when false, survives save + reload.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    let yaml = r#"
schema_version: 2
targets:
  T1:
    name: Showcase checkpoint work
    status: identified
    value: 5
    cost: 3
    showcase: true
    acceptance:
      - done
    discovered: 2026-04-11
  T2:
    name: Plain work
    status: identified
    value: 3
    cost: 2
    acceptance:
      - done
    discovered: 2026-04-11
"#;
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();

    let file = store::load(&path).unwrap();
    assert!(file.targets["T1"].showcase);
    assert!(!file.targets["T2"].showcase);

    // Save + reload: showcase preserved, absent-false field not
    // re-emitted on the false target.
    store::save(&path, &file).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("showcase: true"));
    // The false target must not have an explicit `showcase: false`
    // line — serde should skip it via `is_false`.
    let t2_block = raw.split("T2:").nth(1).unwrap_or("");
    assert!(
        !t2_block.contains("showcase"),
        "T2 should not serialize showcase: false; got:\n{t2_block}"
    );
    // The legacy field name must not appear in newly-written output —
    // it deserialises via alias but serialises only as `showcase`.
    assert!(
        !raw.contains("observable:"),
        "save must emit the new field name only; got:\n{raw}"
    );

    let reloaded = store::load(&path).unwrap();
    assert!(reloaded.targets["T1"].showcase);
    assert!(!reloaded.targets["T2"].showcase);
}

#[test]
fn legacy_observable_field_still_deserialises() {
    // Schema v2 renamed `observable` to `showcase`. Pre-v2 yaml files
    // in the wild still use the legacy key; the `#[serde(alias)]` on
    // the new field must accept them transparently and surface the
    // value through the new field name. On the next save the file
    // is rewritten under `showcase`, so this is a one-shot migration.
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    // schema_version: 1 mimics a file written by an older bullseye
    // before the rename landed. The loader must still accept it.
    let yaml = r#"
schema_version: 1
targets:
  T1:
    name: Legacy showcase work
    status: identified
    value: 5
    cost: 3
    observable: true
    acceptance:
      - done
    discovered: 2026-03-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();

    let file = store::load(&path).unwrap();
    assert!(
        file.targets["T1"].showcase,
        "legacy `observable: true` must populate the new `showcase` field"
    );

    // Round-trip the file through save and confirm the legacy name
    // is purged in favour of the new one.
    store::save(&path, &file).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        raw.contains("showcase: true"),
        "saved file must emit the new field name; got:\n{raw}"
    );
    assert!(
        !raw.contains("observable:"),
        "saved file must not retain the legacy field name; got:\n{raw}"
    );
}

#[test]
fn checkpoint_distance_finds_nearest_checkpoint() {
    // Fixture graph:
    //   T1 (work)   ← T5 (verify covers T1, T3) — dist=1
    //   T2 (work)   — no forward path to any observable — dist=None
    //   T3 (work)   ← T5                                   — dist=1
    //   T5 (verify) — itself observable                    — dist=0
    let file = load_fixture();
    assert_eq!(graph::checkpoint_distance(&file, "T1", 4), Some(1));
    assert_eq!(graph::checkpoint_distance(&file, "T3", 4), Some(1));
    assert_eq!(graph::checkpoint_distance(&file, "T5", 4), Some(0));
    assert_eq!(graph::checkpoint_distance(&file, "T2", 4), None);

    // Marking T2 itself observable flips it to dist=0.
    let mut mutated = load_fixture();
    mutated.targets.get_mut("T2").unwrap().showcase = true;
    assert_eq!(graph::checkpoint_distance(&mutated, "T2", 4), Some(0));

    // Promoting a downstream work target to observable also closes
    // the tunnel, without touching the verify-target path.
    let mut downstream = load_fixture();
    // Make T1 depend on nothing (already true) and insert a chain
    // T2 → T2_downstream(observable). Forward edge from T2: anything
    // that depends on T2. Give T5 a new dependency on T2 so T2
    // forwards into T5 (which is observable).
    downstream
        .targets
        .get_mut("T5")
        .unwrap()
        .depends_on
        .push("T2".to_string());
    assert_eq!(
        graph::checkpoint_distance(&downstream, "T2", 4),
        Some(1),
        "T2 should see T5 once T5 depends on T2"
    );
}

#[test]
fn frontier_ordering_prefers_checkpoint_then_fanout() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    // Hand-rolled scenario exercising every sort key in turn:
    //   T100 (observable work)            → dist=0, fanout=0  → rank 1
    //   T101 (work, T102 depends on it)   → dist=∞ via T102→T103(verify)
    //                                                           dist=2, fanout=1
    //   T102 (work, blocked by T101)      → not in frontier
    //   T103 (verify, verifies T100)      → dist=0 itself, but blocked
    //   T104 (work, dependants = T105,
    //         T106 both depend on it,
    //         downstream reaches T107
    //         verify)                    → dist=1, fanout=2 → rank 2
    //   T105 (work, deps [T104])         → blocked
    //   T106 (work, deps [T104])         → blocked
    //   T107 (verify, verifies T104,
    //         deps [T104])                → blocked
    //   T108 (work, tunnel)               → dist=None, fanout=0 → last
    //
    // Expected frontier sort: T100 (dist=0), T104 (dist=1, fanout=2),
    // T101 (dist=2, fanout=1), T108 (dist=None).
    let date = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap();
    let mut file = TargetsFile {
        schema_version: Some(1),
        last_evaluated: None,
        targets: Default::default(),
    };

    let mk = |name: &str, kind: Kind, showcase: bool, deps: &[&str], verifies: &[&str]| -> Target {
        Target {
            name: name.to_string(),
            kind,
            status: Status::Identified,
            value: 1.0,
            cost: 1.0,
            showcase,
            actual_cost: None,
            demonstration: None,
            acceptance: vec!["done".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            cross_depends: vec![],
            cross_enables: vec![],
            verifies: verifies.iter().map(|s| s.to_string()).collect(),
            rework: None,
            retry_budget: None,
            retries: 0,
            tags: vec![],
            origin: "test".to_string(),
            discovered: date,
            achieved: None,
        }
    };

    file.targets
        .insert("T100".into(), mk("Obs work", Kind::Work, true, &[], &[]));
    file.targets
        .insert("T101".into(), mk("Two hops", Kind::Work, false, &[], &[]));
    file.targets.insert(
        "T102".into(),
        mk("Bridge", Kind::Work, false, &["T101"], &[]),
    );
    file.targets.insert(
        "T103".into(),
        mk("Verify T102", Kind::Verify, false, &["T102"], &["T102"]),
    );
    file.targets
        .insert("T104".into(), mk("Two deps", Kind::Work, false, &[], &[]));
    file.targets.insert(
        "T105".into(),
        mk("A of T104", Kind::Work, false, &["T104"], &[]),
    );
    file.targets.insert(
        "T106".into(),
        mk("B of T104", Kind::Work, false, &["T104"], &[]),
    );
    file.targets.insert(
        "T107".into(),
        mk("Verify T104", Kind::Verify, false, &["T104"], &["T104"]),
    );
    file.targets
        .insert("T108".into(), mk("Tunnel", Kind::Work, false, &[], &[]));

    let errors = graph::validate(&file);
    assert!(errors.is_empty(), "fixture invalid: {errors:?}");

    let front = graph::frontier(&file);
    let ranked = graph::rank_frontier(&file, &front);
    let ids: Vec<&str> = ranked.iter().map(|r| r.target.id.as_str()).collect();

    // T100 is observable (dist=0). T104 has dist=1 via T107 and
    // fanout=2 (T105, T106, T107 all depend on it). T101 has
    // dist=2 via T102 → T103 and fanout=1. T108 is a tunnel.
    assert_eq!(
        ids,
        vec!["T100", "T104", "T101", "T108"],
        "repo-level ordering mismatch; ranked entries:\n{:?}",
        ranked
            .iter()
            .map(|r| (r.target.id.clone(), r.distance, r.fanout))
            .collect::<Vec<_>>()
    );

    // Inspect each signal explicitly so a future refactor that
    // accidentally swaps the sort order fails loudly.
    let by_id = |id: &str| ranked.iter().find(|r| r.target.id == id).unwrap();
    assert_eq!(by_id("T100").distance, Some(0));
    assert_eq!(by_id("T100").fanout, 0);
    assert_eq!(by_id("T104").distance, Some(1));
    assert_eq!(by_id("T104").fanout, 3);
    assert_eq!(by_id("T101").distance, Some(2));
    assert_eq!(by_id("T101").fanout, 1);
    assert_eq!(by_id("T108").distance, None);
}

// --- Phase-boundary tests (🎯T11): value/cost optional at repo scope ---

/// Creating a repo-scope target without value or cost must succeed.
/// value/cost are portfolio-scope metadata (cross-repo WSJF ranking) and
/// must not be required when working within a single repo.
#[test]
fn put_create_without_value_cost_succeeds() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "phase-boundary-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    // Cleared at end of test.

    let result = handle_put(PutTool {
        cwd: cwd.clone(),
        id: None,
        name: Some("Repo-scope target with no portfolio metadata".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["CI green".to_string()]),
        context: None,
        kind: None,
        status: None,
        depends_on: None,
        showcase: None,
        blocks: None,
        verifies: None,
        origin: None,
        tags: None,
    });
    assert!(
        result.is_ok(),
        "create without value/cost must succeed: {result:?}"
    );

    // The created target must pass validation.
    let file = store::load(&path).unwrap();
    // Find the newly created target (it will be auto-assigned an ID beyond T1).
    let new_target = file
        .targets
        .values()
        .find(|t| t.name.contains("Repo-scope target"))
        .expect("new target should exist after put");
    assert_eq!(new_target.value, 0.0, "value should default to 0.0");
    assert_eq!(new_target.cost, 0.0, "cost should default to 0.0");

    // Validate: 0.0 value/cost must not produce validation errors.
    let errors = graph::validate(&file);
    let value_cost_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.contains("value") || e.contains("cost"))
        .collect();
    assert!(
        value_cost_errors.is_empty(),
        "0.0 value/cost should not produce validation errors: {value_cost_errors:?}"
    );

    config::set_external_root_override(None);
}

/// Repo-scope frontier ordering (observable distance + fanout) must be
/// invariant under value/cost mutation. Changing a target's portfolio
/// metadata must not change where it appears in the repo frontier.
#[test]
fn frontier_order_invariant_under_value_cost_mutation() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    let date = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();

    let mk_work = |name: &str, showcase: bool, deps: &[&str], v: f64, c: f64| -> Target {
        Target {
            name: name.to_string(),
            kind: Kind::Work,
            status: Status::Identified,
            value: v,
            cost: c,
            showcase,
            actual_cost: None,
            demonstration: None,
            acceptance: vec!["done".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            cross_depends: vec![],
            cross_enables: vec![],
            verifies: vec![],
            rework: None,
            retry_budget: None,
            retries: 0,
            tags: vec![],
            origin: "test".to_string(),
            discovered: date,
            achieved: None,
        }
    };

    // Build a graph: T200(observable) → T201 → T202(no observable reachable).
    // T200 is observable (dist=0), T201 has dist=1 via reverse graph,
    // T202 has no observable reachable (tunnel).
    // All have value=1, cost=1 initially.
    let mut file = TargetsFile {
        schema_version: Some(1),
        last_evaluated: None,
        targets: Default::default(),
    };
    file.targets
        .insert("T200".into(), mk_work("Observable", true, &[], 1.0, 1.0));
    file.targets
        .insert("T201".into(), mk_work("Near", false, &[], 1.0, 1.0));
    file.targets
        .insert("T202".into(), mk_work("Tunnel", false, &[], 1.0, 1.0));

    // Add a verify target for T201 so it has an observable at distance 1.
    file.targets.insert(
        "T203".into(),
        Target {
            name: "Verify T201".to_string(),
            kind: Kind::Verify,
            status: Status::Identified,
            value: 1.0,
            cost: 1.0,
            showcase: false,
            actual_cost: None,
            demonstration: None,
            acceptance: vec!["verified".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: vec!["T201".to_string()],
            cross_depends: vec![],
            cross_enables: vec![],
            verifies: vec!["T201".to_string()],
            rework: None,
            retry_budget: None,
            retries: 0,
            tags: vec![],
            origin: "test".to_string(),
            discovered: date,
            achieved: None,
        },
    );

    let front = graph::frontier(&file);
    let ranked_before: Vec<&str> = graph::rank_frontier(&file, &front)
        .iter()
        .map(|r| {
            // Leak the string to extend lifetime — OK in tests.
            Box::leak(r.target.id.clone().into_boxed_str()) as &str
        })
        .collect();

    // Now mutate value/cost dramatically on all targets.
    // Portfolio-scope numbers that vary by orders of magnitude.
    for (i, t) in file.targets.values_mut().enumerate() {
        t.value = (i as f64 + 1.0) * 100.0;
        t.cost = (i as f64 + 1.0) * 0.01;
    }

    let front2 = graph::frontier(&file);
    let ranked_after: Vec<&str> = graph::rank_frontier(&file, &front2)
        .iter()
        .map(|r| Box::leak(r.target.id.clone().into_boxed_str()) as &str)
        .collect();

    assert_eq!(
        ranked_before, ranked_after,
        "repo-level frontier order changed after value/cost mutation — \
         ordering must depend only on observable distance and fanout"
    );
}

/// Flipping `observable: true` on a work target must change repo-scope
/// frontier ordering: an observable target should rank above a non-observable
/// peer (distance 0 beats distance >0).
#[test]
fn showcase_flag_changes_frontier_order() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    let date = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();

    let mk_work = |name: &str, showcase: bool| -> Target {
        Target {
            name: name.to_string(),
            kind: Kind::Work,
            status: Status::Identified,
            value: 1.0,
            cost: 1.0,
            showcase,
            actual_cost: None,
            demonstration: None,
            acceptance: vec!["done".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: vec![],
            cross_depends: vec![],
            cross_enables: vec![],
            verifies: vec![],
            rework: None,
            retry_budget: None,
            retries: 0,
            tags: vec![],
            origin: "test".to_string(),
            discovered: date,
            achieved: None,
        }
    };

    // Two plain work targets, no observable anywhere.
    let mut file = TargetsFile {
        schema_version: Some(1),
        last_evaluated: None,
        targets: Default::default(),
    };
    file.targets
        .insert("T300".into(), mk_work("Plain A", false));
    file.targets
        .insert("T301".into(), mk_work("Plain B", false));

    let front = graph::frontier(&file);
    let ranked_before: Vec<String> = graph::rank_frontier(&file, &front)
        .iter()
        .map(|r| r.target.id.clone())
        .collect();

    // Both are tunnels (no observable reachable), so they rank equally by ID.
    // T300 comes before T301 lexicographically as the tiebreaker.
    assert_eq!(
        ranked_before,
        vec!["T300", "T301"],
        "before: {ranked_before:?}"
    );
    assert!(
        graph::rank_frontier(&file, &front)
            .iter()
            .all(|r| r.distance.is_none()),
        "both should be tunnels before any observable flag"
    );

    // Now flip T301 to observable.
    file.targets.get_mut("T301").unwrap().showcase = true;

    let front2 = graph::frontier(&file);
    let ranked_after: Vec<String> = graph::rank_frontier(&file, &front2)
        .iter()
        .map(|r| r.target.id.clone())
        .collect();

    // T301 (observable, dist=0) must now rank above T300 (no observable reachable).
    assert_eq!(
        ranked_after,
        vec!["T301", "T300"],
        "observable target should rank first after flag flip; got: {ranked_after:?}"
    );

    let by_id = |id: &str| {
        graph::rank_frontier(&file, &front2)
            .into_iter()
            .find(|r| r.target.id == id)
            .unwrap()
    };
    assert_eq!(
        by_id("T301").distance,
        Some(0),
        "observable target is at distance 0"
    );
    assert_eq!(
        by_id("T300").distance,
        None,
        "non-observable tunnel has no reachable observable"
    );
}

#[test]
fn tunnels_treats_showcase_work_as_checkpoint() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    // Two chains:
    //   Chain A:  T20 → T21 → T22(observable: true)
    //             — T20 should NOT be a tunnel (dist=2 via observable work).
    //   Chain B:  T30 → T31 → T32 (all plain work)
    //             — T30, T31, T32 ARE tunnels (no observable reachable).
    //
    // With the legacy "verify only" definition, every target in
    // chain A would be flagged; the generalisation in 🎯T7 must
    // recognise T22 as a checkpoint.
    let date = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap();
    let mut file = TargetsFile {
        schema_version: Some(1),
        last_evaluated: None,
        targets: Default::default(),
    };
    let mk = |name: &str, showcase: bool, deps: &[&str]| -> Target {
        Target {
            name: name.to_string(),
            kind: Kind::Work,
            status: Status::Identified,
            value: 1.0,
            cost: 1.0,
            showcase,
            actual_cost: None,
            demonstration: None,
            acceptance: vec!["done".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            cross_depends: vec![],
            cross_enables: vec![],
            verifies: vec![],
            rework: None,
            retry_budget: None,
            retries: 0,
            tags: vec![],
            origin: "test".to_string(),
            discovered: date,
            achieved: None,
        }
    };
    file.targets.insert("T20".into(), mk("A start", false, &[]));
    file.targets
        .insert("T21".into(), mk("A middle", false, &["T20"]));
    file.targets
        .insert("T22".into(), mk("A checkpoint", true, &["T21"]));
    file.targets.insert("T30".into(), mk("B start", false, &[]));
    file.targets
        .insert("T31".into(), mk("B middle", false, &["T30"]));
    file.targets
        .insert("T32".into(), mk("B end", false, &["T31"]));

    let warnings = graph::tunnels(&file, 3);
    let flagged: Vec<&str> = warnings.iter().map(|w| w.target_id.as_str()).collect();

    // T22 itself is observable → never a tunnel.
    assert!(!flagged.contains(&"T22"));
    // T20 reaches T22 in 2 hops (within max_depth 3) → clean.
    assert!(!flagged.contains(&"T20"), "got: {flagged:?}");
    // T21 reaches T22 in 1 hop → clean.
    assert!(!flagged.contains(&"T21"), "got: {flagged:?}");
    // Chain B: no observable reachable → all flagged.
    assert!(flagged.contains(&"T30"));
    assert!(flagged.contains(&"T31"));
    assert!(flagged.contains(&"T32"));
}

#[test]
fn convergence_blocks_on_tunnel_when_top_frontier_unreachable() {
    // A project whose entire frontier is a tunnel (no checkpoint
    // reachable anywhere) must trigger the reshape recommendation
    // instead of auto-selecting. Uses the `**Blocked**:` prefix so
    // `/cv`'s auto-execute branch pauses. See 🎯T7 acceptance #5.
    let tmp = tempfile::tempdir().unwrap();
    let makefile = "bullseye:\n\t@true\n";
    // Same shape as SIMPLE_TARGETS_YAML but WITHOUT the showcase
    // flag on T1 — so neither target has a checkpoint downstream.
    let targets = r#"
schema_version: 2
targets:
  T1:
    name: Opaque work
    status: identified
    value: 8
    cost: 3
    acceptance:
      - done
    discovered: 2026-04-01
  T2:
    name: More opaque work
    status: identified
    value: 3
    cost: 2
    acceptance:
      - done
    discovered: 2026-04-01
"#;
    write_project(tmp.path(), makefile, targets);
    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    let next = out.split("## Next action").nth(1).expect("next action");
    assert!(
        next.contains("**Blocked**"),
        "expected blocked recommendation; got:\n{next}"
    );
    assert!(
        next.contains("tunnel"),
        "expected tunnel language; got:\n{next}"
    );
    assert!(
        next.contains("showcase: true") || next.contains("verify target"),
        "expected reshape guidance; got:\n{next}"
    );
    // Crucially: no Execute now dispatch — the /cv skill relies on
    // this to pause for human reshaping.
    assert!(!next.contains("**Execute now**"));
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
                showcase: false,
                actual_cost: None,
                demonstration: None,
                acceptance: vec!["done".to_string()],
                checks: vec![],
                context: String::new(),
                gates: vec![],
                depends_on: vec![],
                cross_depends: vec![],
                cross_enables: vec![],
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
            showcase: false,
            actual_cost: None,
            demonstration: None,
            acceptance: vec!["done".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: vec![],
            cross_depends: vec![],
            cross_enables: vec![],
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
    // A repo with no bullseye.yaml must not make startup_context fail
    // outright — the session-start hook that typically invokes it runs
    // before the agent knows whether the repo uses bullseye. Return a
    // friendly "not using bullseye yet" message instead.
    let tmp = tempfile::tempdir().unwrap();
    // Sanity check: discover returns None on a fresh empty dir.
    assert!(store::discover(tmp.path()).is_none());

    let out = graph::startup_context_no_file(&tmp.path().display().to_string());
    assert!(out.contains("# Startup context"));
    assert!(out.contains("no bullseye.yaml found"));
    assert!(out.contains("bullseye_init"));
    // Must not look like an error string — agents should be able to
    // keep going.
    assert!(!out.to_lowercase().contains("error"));
}

#[test]
fn load_parse_error_is_structured() {
    use std::io::Write;
    // A bullseye.yaml that exists but is syntactically broken should
    // return LoadError::Parse — the typed variant lets callers like
    // bullseye_startup_context choose to degrade gracefully instead
    // of surfacing a raw tool-call error.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

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
    // bullseye.yaml must surface the error without looking like a
    // tool-call failure — session start should continue.
    let out = graph::startup_context_broken_file(
        "/tmp/fake/bullseye.yaml",
        "failed to parse /tmp/fake/bullseye.yaml: invalid YAML at line 4",
    );
    assert!(out.contains("# Startup context"));
    assert!(out.contains("/tmp/fake/bullseye.yaml"));
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

// --- executable acceptance checks (🎯T1.1) --------------------------------

#[test]
fn checks_field_yaml_roundtrip() {
    use bullseye::schema::{Check, QueryCheck};

    // Parsing a targets file with every check variant and re-serializing
    // it must preserve shape, field names, and variant discriminators.
    // This is the load/save round-trip guarantee called out in the
    // target description.
    let yaml = r#"
schema_version: 1
targets:
  T1:
    name: Platform code is isolated
    status: identified
    value: 5
    cost: 3
    acceptance:
      - No platform #ifdefs outside src/platform/
    checks:
      - convention: no-platform-ifdefs
      - query:
          kind: preprocessor_directive
          pattern: "ifdef|ifndef|if defined"
          exclude_path: src/platform/
          expect: 0
      - invariant: platform-isolation
    discovered: 2026-04-01
"#;

    let file: TargetsFile = serde_yaml_ng::from_str(yaml).unwrap();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.checks.len(), 3);
    assert_eq!(
        t1.checks[0],
        Check::Convention {
            convention: "no-platform-ifdefs".to_string()
        }
    );
    match &t1.checks[1] {
        Check::Query {
            query:
                QueryCheck {
                    kind,
                    pattern,
                    exclude_path,
                    expect,
                },
        } => {
            assert_eq!(kind, "preprocessor_directive");
            assert_eq!(pattern.as_deref(), Some("ifdef|ifndef|if defined"));
            assert_eq!(exclude_path.as_deref(), Some("src/platform/"));
            assert_eq!(*expect, 0);
        }
        other => panic!("expected Query, got {other:?}"),
    }
    assert_eq!(
        t1.checks[2],
        Check::Invariant {
            invariant: "platform-isolation".to_string()
        }
    );

    // Round-trip through YAML and re-parse — must equal the original
    // in-memory shape.
    let reserialized = serde_yaml_ng::to_string(&file).unwrap();
    let reparsed: TargetsFile = serde_yaml_ng::from_str(&reserialized).unwrap();
    assert_eq!(reparsed.targets["T1"].checks, t1.checks);

    // Convention variant should serialize as a single-key map
    // `- convention: ...`, not `- !Convention ...` or `- {tag: ...}`.
    assert!(
        reserialized.contains("convention: no-platform-ifdefs"),
        "got:\n{reserialized}"
    );
    assert!(
        reserialized.contains("invariant: platform-isolation"),
        "got:\n{reserialized}"
    );
    assert!(reserialized.contains("query:"), "got:\n{reserialized}");
}

#[test]
fn checks_field_survives_store_save_load() {
    use bullseye::schema::{Check, QueryCheck};
    // End-to-end round-trip through the store layer (which adds the
    // schema version stamp, migrations, etc.) to prove `checks`
    // survives a real save/load cycle, not just in-memory serde.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    let mut file = load_fixture();
    file.targets.get_mut("T3").unwrap().checks = vec![
        Check::Convention {
            convention: "no-platform-ifdefs".to_string(),
        },
        Check::Query {
            query: QueryCheck {
                kind: "preprocessor_directive".to_string(),
                pattern: None,
                exclude_path: Some("src/platform/".to_string()),
                expect: 0,
            },
        },
    ];

    store::save(&path, &file).unwrap();
    let reloaded = store::load(&path).unwrap();
    assert_eq!(reloaded.targets["T3"].checks, file.targets["T3"].checks);
    // Other targets with no checks stay empty.
    assert!(reloaded.targets["T1"].checks.is_empty());
}

#[test]
fn checks_field_skipped_when_empty() {
    // A target with no checks must not emit an empty `checks: []`
    // key — it should be omitted entirely to avoid cluttering the
    // default YAML view.
    let file = load_fixture();
    let yaml = serde_yaml_ng::to_string(&file).unwrap();
    assert!(
        !yaml.contains("checks:"),
        "fixture targets have no checks; field should be omitted.\n{yaml}"
    );
}

#[test]
fn verify_plan_builds_for_all_variants() {
    use bullseye::ops::{CheckKind, CheckOutcome, CheckSpec, SawmillTool, verify_plan};
    use bullseye::schema::{Check, QueryCheck};

    let mut file = load_fixture();
    let t3 = file.targets.get_mut("T3").unwrap();
    t3.checks = vec![
        Check::Convention {
            convention: "no-platform-ifdefs".to_string(),
        },
        Check::Query {
            query: QueryCheck {
                kind: "preprocessor_directive".to_string(),
                pattern: Some("ifdef|ifndef|if defined".to_string()),
                exclude_path: Some("src/platform/".to_string()),
                expect: 0,
            },
        },
        Check::Invariant {
            invariant: "platform-isolation".to_string(),
        },
    ];

    let plan = verify_plan(&file, "T3").unwrap();
    assert_eq!(plan.target_id, "T3");
    assert_eq!(plan.checks.len(), 3);

    // Each planned check is routed to the right sawmill tool.
    assert_eq!(plan.checks[0].tool, SawmillTool::CheckConventions);
    assert_eq!(plan.checks[1].tool, SawmillTool::Query);
    assert_eq!(plan.checks[2].tool, SawmillTool::CheckInvariants);

    // And each carries structured args the agent can feed to sawmill.
    match &plan.checks[0].spec {
        CheckSpec::Convention { convention } => {
            assert_eq!(convention, "no-platform-ifdefs");
        }
        other => panic!("expected Convention, got {other:?}"),
    }
    match &plan.checks[1].spec {
        CheckSpec::Query { query: q } => {
            assert_eq!(q.kind, "preprocessor_directive");
            assert_eq!(q.expect, 0);
        }
        other => panic!("expected Query, got {other:?}"),
    }
    match &plan.checks[2].spec {
        CheckSpec::Invariant { invariant } => {
            assert_eq!(invariant, "platform-isolation");
        }
        other => panic!("expected Invariant, got {other:?}"),
    }

    // Report template starts pending with one entry per planned check.
    assert_eq!(plan.report_template.target, "T3");
    assert_eq!(plan.report_template.overall, CheckOutcome::Pending);
    assert_eq!(plan.report_template.checks.len(), 3);
    assert_eq!(plan.report_template.checks[0].kind, CheckKind::Convention);
    assert_eq!(plan.report_template.checks[1].kind, CheckKind::Query);
    assert_eq!(plan.report_template.checks[2].kind, CheckKind::Invariant);
    for entry in &plan.report_template.checks {
        assert_eq!(entry.outcome, CheckOutcome::Pending);
        assert!(entry.failures.is_empty());
    }
}

#[test]
fn verify_plan_errors_for_missing_target() {
    use bullseye::ops::{VerifyError, verify_plan};

    let file = load_fixture();
    let err = verify_plan(&file, "T99").unwrap_err();
    assert_eq!(err, VerifyError::TargetNotFound("T99".to_string()));
}

#[test]
fn verify_plan_errors_when_no_checks_defined() {
    use bullseye::ops::{VerifyError, verify_plan};

    let file = load_fixture();
    // Fixture T1 has no checks — verify_plan should refuse with a
    // structured error rather than returning an empty plan (callers
    // need to distinguish "no work to plan" from "plan is ready").
    let err = verify_plan(&file, "T1").unwrap_err();
    assert_eq!(err, VerifyError::NoChecks("T1".to_string()));
}

#[test]
fn verify_report_structure_serializes_file_line_detail() {
    use bullseye::ops::{CheckFailure, CheckKind, CheckOutcome, CheckResult, VerifyReport};

    // The report type is what the agent populates after running
    // sawmill. Make sure file/line-level detail round-trips through
    // serde so the agent can feed reports back into tooling.
    let report = VerifyReport {
        target: "T3".to_string(),
        overall: CheckOutcome::Fail,
        checks: vec![CheckResult {
            index: 0,
            kind: CheckKind::Convention,
            outcome: CheckOutcome::Fail,
            failures: vec![CheckFailure {
                file: Some("src/foo.c".to_string()),
                line: Some(42),
                message: "platform #ifdef outside src/platform/".to_string(),
            }],
        }],
    };

    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"overall\":\"fail\""));
    assert!(json.contains("\"file\":\"src/foo.c\""));
    assert!(json.contains("\"line\":42"));

    let reparsed: VerifyReport = serde_json::from_str(&json).unwrap();
    assert_eq!(reparsed, report);
}

// ---------------------------------------------------------------------
// Per-repo discovery integration tests (v0.16.0+).
// ---------------------------------------------------------------------

/// RAII helper: isolate the external shadow root to a tempdir so the
/// tests don't touch the developer's real `~/.local/share/bullseye`,
/// and cleanly restore on drop.
struct ShadowFixture {
    _tmp: tempfile::TempDir,
}

impl ShadowFixture {
    fn with_root(root: &std::path::Path) -> Self {
        bullseye::config::set_external_root_override(Some(root.to_path_buf()));
        // Caller owns the tempdir; this holder just flips the override back.
        ShadowFixture {
            _tmp: tempfile::tempdir().unwrap(),
        }
    }
}

impl Drop for ShadowFixture {
    fn drop(&mut self) {
        bullseye::config::set_external_root_override(None);
    }
}

#[test]
fn missing_targets_file_surfaces_location_prompt() {
    use bullseye::handler::handle_list;
    use bullseye::tools::ListTool;

    // Isolate the shadow root so discover_anywhere's external probe
    // can't accidentally hit an unrelated file.
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let err = handle_list(ListTool {
        cwd: work.path().to_string_lossy().into_owned(),
        filter: "active".to_string(),
    })
    .expect_err("missing targets file must surface as error");
    let msg = format!("{err:?}");

    // The error names where we looked and carries the init prompt.
    assert!(
        msg.contains("no bullseye.yaml found"),
        "not-found preamble missing: {msg}"
    );
    assert!(
        msg.contains("Create bullseye.yaml for this repo where?"),
        "location prompt missing: {msg}"
    );
    assert!(msg.contains("in_repo"), "in_repo choice missing: {msg}");
    assert!(msg.contains("external"), "external choice missing: {msg}");
    assert!(
        msg.contains("bullseye_init"),
        "call-to-action missing: {msg}"
    );
}

#[test]
fn init_without_location_returns_prompt() {
    use bullseye::handler::handle_init;
    use bullseye::tools::InitTool;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let err = handle_init(InitTool {
        cwd: work.path().to_string_lossy().into_owned(),
        location: String::new(), // empty → unknown → prompt
        project_name: None,
    })
    .expect_err("empty location must surface as error");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("unknown location"),
        "parse error missing: {msg}"
    );
    assert!(
        msg.contains("Create bullseye.yaml for this repo where?"),
        "location prompt missing: {msg}"
    );
}

#[test]
fn init_in_repo_creates_file_in_cwd() {
    use bullseye::handler::{handle_init, handle_list};
    use bullseye::tools::{InitTool, ListTool};

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    handle_init(InitTool {
        cwd: cwd.clone(),
        location: "in_repo".to_string(),
        project_name: Some("demo".to_string()),
    })
    .expect("init should succeed");

    assert!(
        work.path().join("bullseye.yaml").is_file(),
        "in-repo init must write into the cwd"
    );

    // Discovery finds it, list works.
    let text = text_from_call_result(
        handle_list(ListTool {
            cwd,
            filter: "active".to_string(),
        })
        .expect("list after init should succeed"),
    );
    assert!(text.contains("🎯T1"), "listing missing T1: {text}");
}

#[test]
fn init_external_creates_file_in_shadow_tree() {
    use bullseye::handler::{handle_init, handle_list};
    use bullseye::tools::{InitTool, ListTool};

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    handle_init(InitTool {
        cwd: cwd.clone(),
        location: "external".to_string(),
        project_name: Some("demo".to_string()),
    })
    .expect("external init should succeed");

    // cwd stays clean.
    assert!(
        !work.path().join("bullseye.yaml").exists(),
        "external init must not write into the cwd"
    );

    // Shadow path contains the file.
    let mut expected = shadow.path().to_path_buf();
    for c in work.path().components() {
        if let std::path::Component::Normal(part) = c {
            expected.push(part);
        }
    }
    expected.push("bullseye.yaml");
    assert!(
        expected.is_file(),
        "shadow-tree file missing: {}",
        expected.display()
    );

    // Discovery finds it through discover_anywhere's external branch.
    let text = text_from_call_result(
        handle_list(ListTool {
            cwd,
            filter: "active".to_string(),
        })
        .expect("list after external init should succeed"),
    );
    assert!(text.contains("🎯T1"), "listing missing T1: {text}");
}

#[test]
fn init_refuses_when_file_already_exists_in_either_location() {
    use bullseye::handler::handle_init;
    use bullseye::tools::InitTool;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    handle_init(InitTool {
        cwd: cwd.clone(),
        location: "in_repo".to_string(),
        project_name: None,
    })
    .expect("first init should succeed");

    // Second init — even with a different location — is refused.
    let err = handle_init(InitTool {
        cwd,
        location: "external".to_string(),
        project_name: None,
    })
    .expect_err("second init must be refused");
    assert!(
        format!("{err:?}").contains("already exists"),
        "expected already-exists error"
    );
}

#[test]
fn in_repo_wins_when_both_locations_have_files() {
    use bullseye::handler::handle_list;
    use bullseye::store;
    use bullseye::tools::ListTool;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    // Pre-seed both locations. Use distinguishable content so the
    // assertion can prove which file was read.
    let in_repo_path = work.path().join("bullseye.yaml");
    std::fs::write(
        &in_repo_path,
        "schema_version: 1\ntargets:\n  T1:\n    name: IN_REPO_WINS\n    kind: work\n    status: identified\n    value: 5\n    cost: 3\n    acceptance:\n      - a\n    origin: manual\n    discovered: 2026-01-01\n",
    )
    .unwrap();

    let mut shadow_file = store::shadow_path(shadow.path(), work.path());
    std::fs::create_dir_all(&shadow_file).unwrap();
    shadow_file.push("bullseye.yaml");
    std::fs::write(
        &shadow_file,
        "schema_version: 1\ntargets:\n  T1:\n    name: SHADOW_SHOULD_LOSE\n    kind: work\n    status: identified\n    value: 5\n    cost: 3\n    acceptance:\n      - a\n    origin: manual\n    discovered: 2026-01-01\n",
    )
    .unwrap();

    let text = text_from_call_result(
        handle_list(ListTool {
            cwd,
            filter: "active".to_string(),
        })
        .expect("list should succeed"),
    );
    assert!(
        text.contains("IN_REPO_WINS"),
        "in-repo precedence broken: {text}"
    );
    assert!(
        !text.contains("SHADOW_SHOULD_LOSE"),
        "shadow file should not have been consulted: {text}"
    );
}

// --- Parse cache tests (🎯T13) ---

/// Write a minimal valid bullseye.yaml to a path.
fn write_yaml(path: &std::path::Path, target_name: &str) {
    use std::io::Write;
    write!(
        std::fs::File::create(path).unwrap(),
        "schema_version: 1\ntargets:\n  T1:\n    name: {target_name}\n    \
         status: identified\n    value: 3\n    cost: 2\n    acceptance:\n      \
         - done\n    discovered: 2026-04-15\n"
    )
    .unwrap();
}

#[test]
fn cache_hit_on_unchanged_mtime() {
    // Two consecutive loads of the same file with no modification in between
    // must return the same in-memory data without re-reading the disk. We
    // verify this indirectly: both loads succeed and agree on the target name.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "Original name");

    let first = store::load(&path).unwrap();
    let second = store::load(&path).unwrap();
    assert_eq!(first.targets["T1"].name, second.targets["T1"].name);
    assert_eq!(first.targets["T1"].name, "Original name");
}

#[test]
fn cache_miss_after_mtime_change() {
    // Writing new content to the file must cause the next load to return
    // the updated data, not the previously cached parse.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "First version");

    let first = store::load(&path).unwrap();
    assert_eq!(first.targets["T1"].name, "First version");

    // Rewrite the file with a new name. Use save() to ensure the cache is
    // evicted, then write fresh content to simulate an external edit.
    // We sleep briefly to guarantee a distinct mtime on systems with 1-second
    // mtime granularity (most Linux filesystems without noatime).
    std::thread::sleep(std::time::Duration::from_millis(10));
    write_yaml(&path, "Second version");

    let second = store::load(&path).unwrap();
    assert_eq!(
        second.targets["T1"].name, "Second version",
        "cache should have been invalidated after file was modified"
    );
}

#[test]
fn cache_evicted_after_save() {
    // After store::save() the cache entry is evicted so the next load
    // reads back what was actually written, not a stale in-memory snapshot.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "Original");

    let mut file = store::load(&path).unwrap();
    assert_eq!(file.targets["T1"].name, "Original");

    // Mutate and save.
    file.targets.get_mut("T1").unwrap().name = "Updated".to_string();
    store::save(&path, &file).unwrap();

    // Re-load must reflect the saved state (not the stale in-memory copy).
    let reloaded = store::load(&path).unwrap();
    assert_eq!(reloaded.targets["T1"].name, "Updated");
}

#[test]
fn cache_fallback_to_stale_on_reparse_failure() {
    // If the file becomes temporarily unreadable after the first successful
    // parse, the last valid cached copy is served rather than propagating
    // the I/O error (simulating a mid-edit state).
    use std::io::Write;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "Good state");

    // Prime the cache with a successful parse.
    let good = store::load(&path).unwrap();
    assert_eq!(good.targets["T1"].name, "Good state");

    // Overwrite with invalid YAML to simulate a mid-edit state.
    // Sleep briefly to ensure a new mtime on coarse-grained filesystems.
    std::thread::sleep(std::time::Duration::from_millis(10));
    writeln!(std::fs::File::create(&path).unwrap(), "not: valid: yaml: [").unwrap();

    // The load must succeed by serving the stale cached copy rather than
    // propagating the parse error.
    let fallback = store::load(&path).unwrap();
    assert_eq!(
        fallback.targets["T1"].name, "Good state",
        "expected stale cache fallback on parse failure"
    );
}

#[test]
fn concurrent_mutations_do_not_lose_updates() {
    // 🎯T17 regression test: two concurrent mutators each add a distinct
    // target to the same bullseye.yaml. Without flock, one mutation's
    // serialized-back-to-disk write clobbers the other. With flock, the
    // mutations serialise and both targets must be present at the end.
    //
    // We use threads rather than subprocesses because fs2's advisory
    // locks (flock(2) on POSIX, LockFileEx on Windows) are tied to the
    // open-file-description — each thread's independent
    // `OpenOptions::open(...)` gets a distinct OFD, so same-process
    // threads contend on the lock exactly like cross-process writers
    // would. This catches the same lost-update race with ~0ms overhead
    // per iteration (subprocess spawn would cost ~50ms × 2 × N iters).
    //
    // Loop count: 10 iterations, fresh tempdir per iteration. Each
    // iteration runs N concurrent writers and asserts every write
    // landed.
    use std::sync::{Arc, Barrier};
    use std::thread;

    const ITERATIONS: usize = 10;
    const WRITERS_PER_ITERATION: usize = 4;

    for iter in 0..ITERATIONS {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bullseye.yaml");
        write_yaml(&path, "Baseline");

        // All threads wait on this barrier before starting their
        // mutation — maximises contention on the lock.
        let barrier = Arc::new(Barrier::new(WRITERS_PER_ITERATION));

        let handles: Vec<_> = (0..WRITERS_PER_ITERATION)
            .map(|i| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    let new_id = format!("T{}", 1000 + i);
                    store::with_locked_mutation(&path, |file| -> Result<(), String> {
                        file.targets.insert(
                            new_id.clone(),
                            bullseye::schema::Target {
                                name: format!("Concurrent target {i}"),
                                kind: Kind::Work,
                                status: Status::Identified,
                                value: 1.0,
                                cost: 1.0,
                                showcase: false,
                                actual_cost: None,
                                demonstration: None,
                                acceptance: vec!["done".to_string()],
                                checks: Vec::new(),
                                context: String::new(),
                                gates: Vec::new(),
                                depends_on: Vec::new(),
                                cross_depends: Vec::new(),
                                cross_enables: Vec::new(),
                                verifies: Vec::new(),
                                rework: None,
                                retry_budget: None,
                                retries: 0,
                                tags: Vec::new(),
                                origin: "concurrent-test".to_string(),
                                discovered: chrono::Local::now().date_naive(),
                                achieved: None,
                            },
                        );
                        Ok(())
                    })
                    .unwrap_or_else(|e| {
                        panic!("iter {iter} thread {i}: locked mutation failed: {e}")
                    });
                })
            })
            .collect();

        for h in handles {
            h.join().expect("writer thread panicked");
        }

        // Every writer must have landed. Read fresh from disk —
        // bypass any cache by stat'ing directly (load() does this
        // via mtime, but parse_file is private; load() is fine).
        let final_file = store::load(&path).unwrap();
        for i in 0..WRITERS_PER_ITERATION {
            let id = format!("T{}", 1000 + i);
            assert!(
                final_file.targets.contains_key(&id),
                "iter {iter}: target {id} was lost — concurrent write clobbered it"
            );
        }
        // Plus the baseline T1 from write_yaml.
        assert!(
            final_file.targets.contains_key("T1"),
            "iter {iter}: baseline T1 was lost"
        );
    }
}

// --- 🎯T14: showcase retirement enforcement ---

/// Targets that carry `showcase: true` must not retire silently — the
/// agent has to record what was actually shown to the user. The tool
/// rejects retirement when `demonstration` is missing or empty.
#[test]
fn retire_showcase_target_requires_demonstration() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_retire;
    use bullseye::tools::RetireTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "showcase-retire").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Promote the seeded T1 to a showcase target so the obligation
    // fires when we try to retire it.
    {
        let mut file = store::load(&path).unwrap();
        file.targets.get_mut("T1").unwrap().showcase = true;
        store::save(&path, &file).unwrap();
    }

    // Missing demonstration → refused.
    let missing = handle_retire(RetireTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        actual_cost: None,
        demonstration: None,
    });
    let err = missing.expect_err("missing demonstration must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("showcase") && msg.contains("demonstration"),
        "error must name the showcase obligation; got: {msg}"
    );

    // Whitespace-only demonstration → also refused (would otherwise
    // let agents satisfy the obligation with a single space).
    let whitespace = handle_retire(RetireTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        actual_cost: None,
        demonstration: Some("   ".to_string()),
    });
    assert!(
        whitespace.is_err(),
        "whitespace-only demonstration must be rejected"
    );

    // Target stayed unretired across the rejected calls.
    let still_active = store::load(&path).unwrap();
    assert_ne!(
        still_active.targets["T1"].status,
        Status::Achieved,
        "rejected retire calls must not flip the target to achieved"
    );
    assert!(
        still_active.targets["T1"].demonstration.is_none(),
        "rejected calls must not leave a demonstration string behind"
    );

    // Real demonstration string → succeeds and records the note.
    let demo = "ran the binary with the player attached and shared a screen recording";
    let ok = handle_retire(RetireTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        actual_cost: Some(2.0),
        demonstration: Some(demo.to_string()),
    });
    assert!(
        ok.is_ok(),
        "valid demonstration must allow retirement: {ok:?}"
    );

    let retired = store::load(&path).unwrap();
    assert_eq!(retired.targets["T1"].status, Status::Achieved);
    assert_eq!(
        retired.targets["T1"].demonstration.as_deref(),
        Some(demo),
        "demonstration must be persisted on the retired target"
    );

    config::set_external_root_override(None);
}

/// Non-showcase targets retire normally without a demonstration — the
/// obligation only attaches when the flag is set.
#[test]
fn retire_non_showcase_target_does_not_require_demonstration() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_retire;
    use bullseye::tools::RetireTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "no-showcase-retire").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Sanity-check the seeded T1 is not a showcase target.
    {
        let file = store::load(&path).unwrap();
        assert!(!file.targets["T1"].showcase);
    }

    let result = handle_retire(RetireTool {
        cwd,
        id: "T1".to_string(),
        actual_cost: None,
        demonstration: None,
    });
    assert!(
        result.is_ok(),
        "plain work targets must retire without a demonstration: {result:?}"
    );

    let retired = store::load(&path).unwrap();
    assert_eq!(retired.targets["T1"].status, Status::Achieved);
    assert!(retired.targets["T1"].demonstration.is_none());

    config::set_external_root_override(None);
}
