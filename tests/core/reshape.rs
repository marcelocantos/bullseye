// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

/// Fan-out insertion: file a new prerequisite above an existing node.
/// The fixture has T2 (depends on T1). Adding G that depends on T1
/// and blocks T2 inserts G between them in a single put call.
#[test]
fn reshape_fanout_insertion_above_existing_node_via_blocks() {
    use bullseye::config;
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    let result = handle_put(PutTool {
        reason: None,
        cwd,
        id: None,
        child_of: None,
        name: Some("Gate above T2 and T3".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["gate satisfied".to_string()]),
        context: None,
        status: None,
        depends_on: Some(vec!["T1".to_string()]),
        blocks: Some(vec!["T2".to_string(), "T3".to_string()]),
        origin: None,
        tags: None,
    });
    assert!(result.is_ok(), "put with blocks should succeed: {result:?}");

    let file = store::load(&path).unwrap();

    // A new target was created; find it (next free top-level slot).
    let new_id = file
        .targets
        .keys()
        .find(|k| !["T1", "T2", "T3"].contains(&k.as_str()))
        .cloned()
        .expect("a new gate target should exist");

    assert_eq!(file.targets[&new_id].depends_on, vec!["T1"]);
    // T2 and T3 both gained the new target as a prerequisite
    // alongside T1 — fan-out inserted above them in one call.
    assert_eq!(
        file.targets["T2"].depends_on,
        vec!["T1".to_string(), new_id.clone()]
    );
    assert_eq!(
        file.targets["T3"].depends_on,
        vec!["T1".to_string(), new_id.clone()]
    );

    config::set_external_root_override(None);
}

/// Chain extension: insert a node between two existing chain links.
/// Fixture chain T1 → T2. Inserting M between them: M depends on T1
/// and blocks T2.
#[test]
fn reshape_chain_extension_via_depends_on_and_blocks() {
    use bullseye::config;
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    handle_put(PutTool {
        reason: None,
        cwd,
        id: None,
        child_of: None,
        name: Some("Mid-chain step".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["intermediate done".to_string()]),
        context: None,
        status: None,
        depends_on: Some(vec!["T1".to_string()]),
        blocks: Some(vec!["T2".to_string()]),
        origin: None,
        tags: None,
    })
    .expect("chain extension should succeed");

    let file = store::load(&path).unwrap();
    let new_id = file
        .targets
        .keys()
        .find(|k| !["T1", "T2", "T3"].contains(&k.as_str()))
        .cloned()
        .expect("a new chain link should exist");

    assert_eq!(file.targets[&new_id].depends_on, vec!["T1"]);
    // T2 now blocks on T1 AND the new step (chain extended in place).
    assert_eq!(
        file.targets["T2"].depends_on,
        vec!["T1".to_string(), new_id]
    );
    // T3 untouched — it was on a sibling branch.
    assert_eq!(file.targets["T3"].depends_on, vec!["T1"]);

    config::set_external_root_override(None);
}

/// Choke-point hoisting: a new node converges multiple existing
/// parents and gates multiple existing children, all in one put call.
/// Build a five-node fixture (A, B, C, D, E) then hoist a new G:
/// G depends on A, B, C and blocks D, E.
#[test]
fn reshape_choke_point_hoisting_via_blocks_listing_multiple_downstreams() {
    use bullseye::config;
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    // Seed A, B, C (no deps) and D, E (depend on T1 placeholder).
    {
        let mut file = store::load(&path).unwrap();
        let today = chrono::Local::now().date_naive();
        for id in ["A", "B", "C"] {
            file.targets.insert(
                id.to_string(),
                bullseye::schema::Target {
                    name: format!("Upstream {id}"),
                    status: bullseye::schema::Status::Identified,
                    value: 0.0,
                    cost: 0.0,
                    actual_cost: None,
                    attestation: None,
                    set_aside_reason: None,
                    acceptance: vec!["done".to_string()],
                    checks: vec![],
                    context: String::new(),
                    gates: vec![],
                    depends_on: vec![],
                    cross_depends: vec![],
                    cross_enables: vec![],
                    tags: vec![],
                    strategy: None,
                    origin: "test".to_string(),
                    discovered: today,
                    achieved: None,
                    owned_by: None,
                    postponed_until: None,
                    postpone_predicate: None,
                },
            );
        }
        for id in ["D", "E"] {
            file.targets.insert(
                id.to_string(),
                bullseye::schema::Target {
                    name: format!("Downstream {id}"),
                    status: bullseye::schema::Status::Identified,
                    value: 0.0,
                    cost: 0.0,
                    actual_cost: None,
                    attestation: None,
                    set_aside_reason: None,
                    acceptance: vec!["done".to_string()],
                    checks: vec![],
                    context: String::new(),
                    gates: vec![],
                    depends_on: vec![],
                    cross_depends: vec![],
                    cross_enables: vec![],
                    tags: vec![],
                    strategy: None,
                    origin: "test".to_string(),
                    discovered: today,
                    achieved: None,
                    owned_by: None,
                    postponed_until: None,
                    postpone_predicate: None,
                },
            );
        }
        store::save(&path, &file).unwrap();
    }

    handle_put(PutTool {
        reason: None,
        cwd,
        id: Some("G".to_string()),
        child_of: None,
        name: Some("Choke-point gate".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["all upstreams converged".to_string()]),
        context: None,
        status: None,
        depends_on: Some(vec!["A".to_string(), "B".to_string(), "C".to_string()]),
        blocks: Some(vec!["D".to_string(), "E".to_string()]),
        origin: None,
        tags: None,
    })
    .expect("choke-point hoisting should succeed");

    let file = store::load(&path).unwrap();
    assert_eq!(file.targets["G"].depends_on, vec!["A", "B", "C"]);
    assert_eq!(file.targets["D"].depends_on, vec!["G"]);
    assert_eq!(file.targets["E"].depends_on, vec!["G"]);

    config::set_external_root_override(None);
}

// --- 🎯T28: git-history-aware ID allocation -------------------------------
