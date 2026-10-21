// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! 🎯T96 — retire/achieve must not report ok:true on a lost update under
//! concurrent ledger writers.

use std::sync::{Arc, Barrier};
use std::thread;

use bullseye::config::{self, Location};
use bullseye::handler::{handle_commit, handle_retire};
use bullseye::schema::Status;
use bullseye::store;
use bullseye::tools::{CommitTool, RetireTool};

fn seed_three_targets(path: &std::path::Path) {
    let yaml = r#"schema_version: 5
targets:
  T1:
    name: one
    status: identified
    value: 1.0
    cost: 1.0
    acceptance: [a]
    discovered: 2026-01-01
  T2:
    name: two
    status: identified
    value: 1.0
    cost: 1.0
    acceptance: [a]
    discovered: 2026-01-01
  T3:
    name: three
    status: identified
    value: 1.0
    cost: 1.0
    acceptance: [a]
    discovered: 2026-01-01
"#;
    std::fs::write(path, yaml).expect("seed yaml");
}

#[test]
fn concurrent_retire_achievements_all_land_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    seed_three_targets(&path);
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    let shadow_root = shadow_tmp.path().to_path_buf();
    config::set_external_root_override(Some(shadow_root.clone()));

    const WRITERS: usize = 3;
    let barrier = Arc::new(Barrier::new(WRITERS));
    let handles: Vec<_> = (1..=WRITERS)
        .map(|n| {
            let cwd = cwd.clone();
            let barrier = Arc::clone(&barrier);
            let shadow_root = shadow_root.clone();
            let id = format!("T{n}");
            thread::spawn(move || {
                config::set_external_root_override(Some(shadow_root));
                barrier.wait();
                handle_retire(RetireTool {
                    cwd,
                    id,
                    attestation: format!("oracle retire-cas T{n}"),
                    actual_cost: None,
                    base: None,
                })
                .expect("retire should succeed under contention");
            })
        })
        .collect();

    for h in handles {
        h.join().expect("thread joins");
    }

    let file = store::load(&path).unwrap();
    for id in ["T1", "T2", "T3"] {
        assert_eq!(
            file.targets[id].status,
            Status::Achieved,
            "🎯{id} must be achieved on disk after concurrent retire"
        );
    }
}

#[test]
fn achieve_with_stale_base_refuses_and_leaves_target_identified() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    seed_three_targets(&path);
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let stale_base = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    let err = handle_commit(CommitTool {
        cwd: cwd.clone(),
        op: "achieve".into(),
        id: Some("T1".into()),
        child_of: None,
        name: None,
        value: None,
        cost: None,
        acceptance: None,
        checks: None,
        adopt_command_checks: false,
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
        actual_cost: None,
        attestation: Some("green test retire-cas".into()),
        reason: None,
        postponed_until: None,
        postpone_predicate: None,
        parent: None,
        mode: None,
        children: None,
        retire_reason: None,
        tail: None,
        owner: None,
        base: Some(stale_base.into()),
    })
    .expect_err("stale base must refuse");

    let msg = format!("{err:?}");
    assert!(
        msg.contains("conflict") || msg.contains("Conflict"),
        "expected conflict code: {msg}"
    );
    assert_eq!(
        store::load(&path).unwrap().targets["T1"].status,
        Status::Identified
    );
}

#[test]
fn achieve_success_is_visible_to_query_in_the_same_process() {
    let tmp = tempfile::tempdir().unwrap();
    store::create_at(tmp.path(), Location::InRepo, "retire-cas-roundtrip").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    handle_retire(RetireTool {
        cwd: cwd.clone(),
        id: "T1".into(),
        attestation: "roundtrip oracle".into(),
        actual_cost: None,
        base: None,
    })
    .expect("retire");

    assert_eq!(
        store::load(&store::discover_anywhere(tmp.path()).unwrap())
            .unwrap()
            .targets["T1"]
            .status,
        Status::Achieved,
        "achieve must be visible on disk immediately after ok retire"
    );
}
