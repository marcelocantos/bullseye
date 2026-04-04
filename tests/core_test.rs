// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use targets::graph;
use targets::render;
use targets::schema::{Kind, TargetsFile};
use targets::store;

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
fn weight_computation() {
    let file = load_fixture();
    let t1 = &file.targets["T1"];
    // value 8 / cost 3 = 2.67
    assert!((t1.weight() - 8.0 / 3.0).abs() < 0.01);
}

#[test]
fn validates_ok() {
    let file = load_fixture();
    let errors = graph::validate(&file);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[test]
fn ranking_order() {
    let file = load_fixture();
    let ranked = graph::rank(&file);
    // Should be sorted by weight descending.
    for w in ranked.windows(2) {
        assert!(w[0].weight >= w[1].weight);
    }
    // T5 (weight 3.0) should be first, then T1 (weight ~2.67).
    assert_eq!(ranked[0].id, "T5");
    assert_eq!(ranked[1].id, "T1");
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
fn detects_missing_parent() {
    let mut file = load_fixture();
    file.targets.get_mut("T2").unwrap().parent = Some("T99".to_string());
    let errors = graph::validate(&file);
    assert!(errors.iter().any(|e| e.contains("T99") && e.contains("does not exist")));
}

#[test]
fn detects_cycle_in_parents() {
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().parent = Some("T2".to_string());
    file.targets.get_mut("T2").unwrap().parent = Some("T1".to_string());
    let errors = graph::validate(&file);
    assert!(errors.iter().any(|e| e.contains("cycle")));
}

#[test]
fn yaml_roundtrip() {
    let file = load_fixture();
    let yaml = serde_yaml::to_string(&file).unwrap();
    let reparsed: TargetsFile = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(file.targets.len(), reparsed.targets.len());
    assert_eq!(
        file.targets["T1"].status,
        reparsed.targets["T1"].status
    );
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
fn frontier_excludes_parents_with_active_children() {
    let mut file = load_fixture();
    // Make T2 a child of T1.
    file.targets.get_mut("T2").unwrap().parent = Some("T1".to_string());
    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    // T1 now has an active child (T2), so T1 is not a leaf.
    assert!(!ids.contains(&"T1"), "T1 should be excluded (has active child T2)");
    assert!(ids.contains(&"T2"));
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
    assert!(errors.iter().any(|e| e.contains("T5") && e.contains("non-empty verifies")));
}

#[test]
fn validate_work_with_verifies_is_error() {
    let mut file = load_fixture();
    // Give T1 (work) a verifies list.
    file.targets.get_mut("T1").unwrap().verifies = vec!["T2".to_string()];
    let errors = graph::validate(&file);
    assert!(errors.iter().any(|e| e.contains("T1") && e.contains("must not have verifies")));
}

#[test]
fn frontier_includes_verify_when_unblocked() {
    let mut file = load_fixture();
    // Achieve T1 and T3 so T5 becomes unblocked.
    use targets::schema::Status;
    file.targets.get_mut("T1").unwrap().status = Status::Achieved;
    file.targets.get_mut("T3").unwrap().status = Status::Achieved;
    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    assert!(ids.contains(&"T5"), "T5 should be in frontier when T1+T3 achieved");
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
}

#[test]
fn blocked_detection() {
    let mut file = load_fixture();
    // Make T2 depend on T1 (which is converging, not achieved).
    file.targets.get_mut("T2").unwrap().depends_on = vec!["T1".to_string()];
    let ranked = graph::rank(&file);
    let t2 = ranked.iter().find(|r| r.id == "T2").unwrap();
    assert!(!t2.blocked_by.is_empty());
    assert_eq!(t2.blocked_by[0], "T1");
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

    // Has weight line.
    assert!(md.contains("- **Weight**:"));

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
