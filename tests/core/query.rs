// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

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
    // Blocking errors must be empty; advisory graph-hygiene (🎯T53) may
    // warn on fixture T5 (blocked leaf that unblocks nothing).
    let errors = graph::validate_blocking(&file);
    assert!(errors.is_empty(), "unexpected blocking errors: {errors:?}");
    let warnings = graph::validate_warnings(&file);
    assert!(
        warnings
            .iter()
            .all(|w| w.contains("advisory") || w.contains("hygiene")),
        "unexpected non-advisory warnings: {warnings:?}"
    );
}

#[test]
fn validate_rejects_zero_valued_dotted_target_id() {
    let mut file = load_fixture();
    let mut target = file.targets["T2"].clone();
    target.name = "Ambiguous zero-valued child".to_string();
    file.targets.insert("T2.0".to_string(), target);

    let errors = graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T2.0") && e.contains("final segment is zero")),
        "expected .0 validation error, got: {errors:?}"
    );
}

#[test]
fn mermaid_generation() {
    let file = load_fixture();
    let diagram = graph::mermaid(&file);
    assert!(diagram.contains("graph TD"));
    // T5 depends on T1 and T3 — should have "needs" edges.
    assert!(diagram.contains("needs"));
    // Default is active-only whole graph: T4 achieved is excluded.
    assert!(
        !diagram.contains("Documentation covers"),
        "achieved T4 not in default active graph"
    );
    // Active nodes present.
    assert!(diagram.contains("T1[") || diagram.contains("T1[\""));
    assert!(diagram.contains("T5[") || diagram.contains("T5[\""));
}

/// 🎯T57: scope, explicit nodes, seed expansion, disjoint components.
#[test]
fn mermaid_subgraph_selection_and_scope() {
    use graph::{MermaidExpand, MermaidOpts, MermaidScope};

    let file = load_fixture();

    // scope=all includes achieved T4.
    let all = graph::mermaid_with_opts(
        &file,
        &MermaidOpts {
            scope: MermaidScope::All,
            ..Default::default()
        },
    );
    assert!(all.contains("graph TD"));
    assert!(
        all.contains("achieved") || all.contains("Documentation"),
        "scope=all must include achieved T4: {all}"
    );

    // Explicit node list: only T2 (disjoint / single node, no edges).
    let only_t2 = graph::mermaid_with_opts(
        &file,
        &MermaidOpts {
            nodes: vec!["T2".into()],
            ..Default::default()
        },
    );
    assert!(only_t2.contains("T2[") || only_t2.contains("T2[\""));
    assert!(!only_t2.contains("T1[") && !only_t2.contains("T1[\""));
    assert!(
        !only_t2.contains("needs"),
        "single node has no edges: {only_t2}"
    );

    // Disjoint explicit selection: T2 and T5 without shared component edges
    // (T5's deps T1/T3 not selected) — must not error, both nodes present.
    let disjoint = graph::mermaid_with_opts(
        &file,
        &MermaidOpts {
            nodes: vec!["T2".into(), "T5".into()],
            ..Default::default()
        },
    );
    assert!(disjoint.contains("T2[") || disjoint.contains("T2[\""));
    assert!(disjoint.contains("T5[") || disjoint.contains("T5[\""));
    assert!(
        !disjoint.contains("needs"),
        "T5 deps not selected so no edges: {disjoint}"
    );

    // Seed T5 + ancestors → T5, T1, T3 with needs edges.
    let around = graph::mermaid_with_opts(
        &file,
        &MermaidOpts {
            seeds: vec!["T5".into()],
            expand: MermaidExpand {
                ancestors: true,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(around.contains("T5[") || around.contains("T5[\""));
    assert!(around.contains("T1[") || around.contains("T1[\""));
    assert!(around.contains("T3[") || around.contains("T3[\""));
    assert!(!around.contains("T2[") && !around.contains("T2[\""));
    assert!(around.contains("needs"));

    // Empty selection (nodes that don't pass scope) → valid mermaid, not error.
    let empty = graph::mermaid_with_opts(
        &file,
        &MermaidOpts {
            nodes: vec!["T4".into()], // achieved; scope active excludes it
            ..Default::default()
        },
    );
    assert!(empty.contains("graph TD"));
    assert!(
        empty.contains("no targets") || empty.contains("empty"),
        "empty selection should be valid mermaid: {empty}"
    );

    // Structural select helper.
    let selected = graph::select_mermaid_nodes(
        &file,
        &MermaidOpts {
            seeds: vec!["T1".into()],
            expand: MermaidExpand {
                descendants: true,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(selected.contains_key("T1"));
    assert!(
        selected.contains_key("T5"),
        "descendants of T1 should include T5: {selected:?}"
    );
}

/// 🎯T57: MCP/CLI path returns fenced mermaid with opts.
#[test]
fn query_graph_view_accepts_subgraph_params() {
    use bullseye::handler::handle_query;
    use bullseye::tools::QueryTool;

    // fixture_path is the directory that contains bullseye.yaml
    // (tests/fixtures) — use it as cwd so discover finds the fixture,
    // not the repo-root ledger.
    let cwd = fixture_path().to_string_lossy().to_string();

    // Default whole active graph — fenced.
    let result = handle_query(QueryTool {
        cwd: cwd.clone(),
        view: Some("graph".into()),
        id: None,
        filter: None,
        recent_days: None,
        momentum: None,
        frontier_details: None,
        scope: None,
        nodes: None,
        seeds: None,
        expand: None,
    })
    .expect("view=graph default");
    let body = text_from_call_result(result);
    assert!(
        body.contains("```mermaid"),
        "must fence for chat clients: {body}"
    );
    assert!(body.contains("graph TD"));
    assert!(body.contains("needs"));

    // Explicit nodes via query params.
    let sub = handle_query(QueryTool {
        cwd: cwd.clone(),
        view: Some("graph".into()),
        id: None,
        filter: None,
        recent_days: None,
        momentum: None,
        frontier_details: None,
        scope: None,
        nodes: Some(vec!["T1".into(), "T5".into()]),
        seeds: None,
        expand: None,
    })
    .expect("view=graph nodes");
    let sub_body = text_from_call_result(sub);
    assert!(sub_body.contains("T1") && sub_body.contains("T5"));
    assert!(
        !sub_body.contains("Logging uses"),
        "T2 name should be absent: {sub_body}"
    );

    // Seeds + expand ancestors.
    let exp = handle_query(QueryTool {
        cwd,
        view: Some("graph".into()),
        id: None,
        filter: None,
        recent_days: None,
        momentum: None,
        frontier_details: None,
        scope: Some("active".into()),
        nodes: None,
        seeds: Some(vec!["T5".into()]),
        expand: Some(vec!["ancestors".into()]),
    })
    .expect("view=graph seeds expand");
    let exp_body = text_from_call_result(exp);
    assert!(exp_body.contains("T1") && exp_body.contains("T3") && exp_body.contains("T5"));
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
fn validate_flags_active_parent_missing_dotted_child() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    let mut file = load_fixture();
    let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
    file.targets.insert(
        "T1.1".to_string(),
        Target {
            name: "Orphan-looking child".to_string(),
            status: Status::Identified,
            value: 1.0,
            cost: 1.0,
            actual_cost: None,
            attestation: None,
            set_aside_reason: None,
            acceptance: vec!["exists".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: vec![],
            cross_depends: vec![],
            cross_enables: vec![],
            tags: vec![],
            strategy: None,
            origin: "test".to_string(),
            discovered: date,
            achieved: None,
            owned_by: None,
            postponed_until: None,
            postpone_predicate: None,
        },
    );

    let errors = graph::validate_blocking(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T1") && e.contains("T1.1")),
        "expected umbrella validation error; got {errors:?}"
    );
}

/// An achieved target whose blockers are still open is a ledger that
/// contradicts itself. Nothing enforced this on the write path until
/// 🎯T79, so ledgers already carry violations; validate must surface
/// them rather than leave them silent. Found in the wild in xbnf, where
/// T8 was achieved with T7 (and transitively T6) still identified.
#[test]
fn validate_reports_an_achieved_target_with_open_dependencies() {
    let file: bullseye::schema::TargetsFile = serde_yaml_ng::from_str(
        "schema_version: 5\ntargets:\n\
         \x20 T6:\n    name: base\n    status: identified\n    value: 0.0\n    cost: 0.0\n\
         \x20   acceptance: [a]\n    discovered: 2026-01-01\n\
         \x20 T7:\n    name: top\n    status: achieved\n    value: 0.0\n    cost: 0.0\n\
         \x20   acceptance: [a]\n    attestation: done\n    depends_on: [T6]\n\
         \x20   discovered: 2026-01-01\n    achieved: 2026-01-02\n",
    )
    .expect("fixture parses");

    let errors = bullseye::graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T7") && e.contains("open target(s): T6")),
        "validate must report the contradiction, got: {errors:?}"
    );
}

/// The converse: a coherent chain must not be flagged.
#[test]
fn validate_accepts_an_achieved_target_whose_dependencies_are_terminal() {
    let file: bullseye::schema::TargetsFile = serde_yaml_ng::from_str(
        "schema_version: 5\ntargets:\n\
         \x20 T6:\n    name: base\n    status: achieved\n    value: 0.0\n    cost: 0.0\n\
         \x20   acceptance: [a]\n    attestation: done\n    discovered: 2026-01-01\n\
         \x20   achieved: 2026-01-01\n\
         \x20 T7:\n    name: top\n    status: achieved\n    value: 0.0\n    cost: 0.0\n\
         \x20   acceptance: [a]\n    attestation: done\n    depends_on: [T6]\n\
         \x20   discovered: 2026-01-01\n    achieved: 2026-01-02\n",
    )
    .expect("fixture parses");
    let errors = bullseye::graph::validate(&file);
    assert!(
        !errors.iter().any(|e| e.contains("open target")),
        "a terminal dependency must not be reported: {errors:?}"
    );
}

/// A blocked target must announce itself as blocked in the views an
/// agent actually reads (🎯T80). The frontier already excluded it; the
/// gap was that `view=target` and `view=list` rendered
/// `status: identified`, which reads as "ready to start", so skipping a
/// dependency chain required no deliberate override — just believing
/// the surface.
#[test]
fn blocked_targets_are_marked_blocked_in_the_read_views() {
    let file: bullseye::schema::TargetsFile = serde_yaml_ng::from_str(
        "schema_version: 5\ntargets:\n\
         \x20 T6:\n    name: base\n    status: identified\n    value: 0.0\n    cost: 0.0\n\
         \x20   acceptance: [a]\n    discovered: 2026-01-01\n\
         \x20 T7:\n    name: middle\n    status: identified\n    value: 0.0\n    cost: 0.0\n\
         \x20   acceptance: [a]\n    depends_on: [T6]\n    discovered: 2026-01-01\n",
    )
    .expect("fixture parses");

    let ready = &file.targets["T6"];
    let blocked = &file.targets["T7"];
    assert!(
        bullseye::graph::open_blockers(&file, ready).is_empty(),
        "a target with no dependencies is not blocked"
    );
    assert_eq!(
        bullseye::graph::open_blockers(&file, blocked),
        vec!["T6".to_string()],
        "an open dependency must be reported as a blocker"
    );
}

/// Terminal dependencies do not block, and the two terminal states are
/// equivalent for this purpose: a target the owner set aside no longer
/// gates its dependents any more than an achieved one does.
#[test]
fn terminal_dependencies_do_not_block() {
    for terminal in ["achieved", "set_aside"] {
        let extra = if terminal == "achieved" {
            "    attestation: done\n    achieved: 2026-01-02\n"
        } else {
            "    set_aside_reason: not pursuing\n"
        };
        let yaml = format!(
            "schema_version: 5\ntargets:\n\
             \x20 T6:\n    name: base\n    status: {terminal}\n    value: 0.0\n    cost: 0.0\n\
             \x20   acceptance: [a]\n    discovered: 2026-01-01\n{extra}\
             \x20 T7:\n    name: middle\n    status: identified\n    value: 0.0\n    cost: 0.0\n\
             \x20   acceptance: [a]\n    depends_on: [T6]\n    discovered: 2026-01-01\n"
        );
        let file: bullseye::schema::TargetsFile =
            serde_yaml_ng::from_str(&yaml).expect("fixture parses");
        assert!(
            bullseye::graph::open_blockers(&file, &file.targets["T7"]).is_empty(),
            "a {terminal} dependency must not block"
        );
    }
}
