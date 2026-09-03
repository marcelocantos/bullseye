// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn t43_owned_by_excludes_from_frontier_but_blocks_dependents() {
    use bullseye::graph;
    use bullseye::schema::{OwnedBy, Status, Target, TargetsFile};
    use chrono::NaiveDate;

    let today = NaiveDate::from_ymd_opt(2026, 7, 11).unwrap();
    let mut targets = std::collections::BTreeMap::new();
    targets.insert(
        "T1".to_string(),
        Target {
            name: "foundation".into(),
            status: Status::Identified,
            value: 0.0,
            cost: 0.0,
            actual_cost: None,
            attestation: None,
            set_aside_reason: None,
            acceptance: vec!["ok".into()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: vec![],
            cross_depends: vec![],
            cross_enables: vec![],
            tags: vec![],
            strategy: None,
            origin: "manual".into(),
            discovered: today,
            achieved: None,
            owned_by: Some(OwnedBy {
                owner: "alice".into(),
                reason: "open PR".into(),
            }),
            postponed_until: None,
            postpone_predicate: None,
        },
    );
    targets.insert(
        "T2".to_string(),
        Target {
            name: "depends on foundation".into(),
            status: Status::Identified,
            value: 0.0,
            cost: 0.0,
            actual_cost: None,
            attestation: None,
            set_aside_reason: None,
            acceptance: vec!["ok".into()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: vec!["T1".into()],
            cross_depends: vec![],
            cross_enables: vec![],
            tags: vec![],
            strategy: None,
            origin: "manual".into(),
            discovered: today,
            achieved: None,
            owned_by: None,
            postponed_until: None,
            postpone_predicate: None,
        },
    );
    let file = TargetsFile {
        schema_version: Some(5),
        last_evaluated: None,
        release_surface: vec![],
        targets,
    };

    let front = graph::frontier(&file);
    assert!(
        front.iter().all(|t| t.id != "T1"),
        "owned-by target must not appear on frontier: {front:?}"
    );
    assert!(
        front.iter().all(|t| t.id != "T2"),
        "dependent of owned-by target must stay blocked: {front:?}"
    );
    assert!(
        graph::owned_elsewhere(&file)
            .iter()
            .any(|(id, _)| *id == "T1"),
        "owned_elsewhere must list T1"
    );

    let summary = graph::summary(&file, "test.yaml", None, false);
    assert!(
        summary.contains("## Owned elsewhere"),
        "summary must have owned elsewhere section: {summary}"
    );
    assert!(summary.contains("owner: alice"), "{summary}");
}

#[test]
fn t43_assign_and_unassign_via_commit() {
    use bullseye::config::{self, Location};
    use bullseye::graph;
    use bullseye::handler::handle_commit;
    use bullseye::store;
    use bullseye::tools::CommitTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "t43").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Starter creates T1 active.
    let assign = handle_commit(CommitTool {
        cwd: cwd.clone(),
        op: "assign".to_string(),
        id: Some("T1".to_string()),
        child_of: None,
        name: None,
        value: None,
        cost: None,
        acceptance: None,
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
        actual_cost: None,
        attestation: None,
        reason: Some("collaborator PR".to_string()),
        postponed_until: None,
        postpone_predicate: None,
        parent: None,
        mode: None,
        children: None,
        retire_reason: None,
        tail: None,
        owner: Some("bob".to_string()),
    })
    .expect("assign");
    let out = text_from_call_result(assign);
    assert!(out.contains("op: assign"), "{out}");

    let file = store::load(&path).unwrap();
    assert!(file.targets["T1"].owned_by.is_some());
    assert!(graph::frontier(&file).iter().all(|t| t.id != "T1"));

    let unassign = handle_commit(CommitTool {
        cwd,
        op: "unassign".to_string(),
        id: Some("T1".to_string()),
        child_of: None,
        name: None,
        value: None,
        cost: None,
        acceptance: None,
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
    .expect("unassign");
    assert!(text_from_call_result(unassign).contains("op: unassign"));

    let file = store::load(&path).unwrap();
    assert!(file.targets["T1"].owned_by.is_none());
    assert!(
        graph::frontier(&file).iter().any(|t| t.id == "T1"),
        "after unassign T1 returns to frontier"
    );
    config::set_external_root_override(None);
}

#[test]
fn t42_release_surface_roundtrips_in_yaml() {
    use bullseye::schema::TargetsFile;

    let yaml = r#"
schema_version: 5
release_surface:
  - dist/
  - src/
targets: {}
"#;
    let file: TargetsFile = serde_yaml_ng::from_str(yaml).unwrap();
    assert_eq!(file.release_surface, vec!["dist/", "src/"]);
    let out = serde_yaml_ng::to_string(&file).unwrap();
    assert!(out.contains("release_surface"), "{out}");
    assert!(out.contains("dist/"), "{out}");
}

#[test]
fn t42_partition_classifies_by_declared_surface() {
    use bullseye::convergence::{UnreleasedFix, partition_by_release_surface};
    use std::process::Command;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // Minimal git repo with two fix commits: one touches src/, one only tests/
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["config", "user.email", "t@example.com"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["config", "user.name", "t"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );

    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("tests/t.rs"), "fn t() {}\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["tag", "v0.1.0"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );

    std::fs::write(root.join("src/lib.rs"), "fn a() { /* fix */ }\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "src/lib.rs"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "fix: surface bug"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let surface_hash = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    std::fs::write(root.join("tests/t.rs"), "fn t() { /* fix */ }\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "tests/t.rs"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "fix: test only"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let test_hash = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    let fixes = vec![
        UnreleasedFix {
            hash: surface_hash.clone(),
            subject: "fix: surface bug".into(),
        },
        UnreleasedFix {
            hash: test_hash.clone(),
            subject: "fix: test only".into(),
        },
    ];
    let surface = vec!["src/".to_string()];
    let (ship, ext) = partition_by_release_surface(root, &fixes, &surface);
    assert_eq!(
        ship.len(),
        1,
        "only surface fix ships: ship={ship:?} ext={ext:?}"
    );
    assert_eq!(ship[0].hash, surface_hash);
    assert_eq!(ext.len(), 1);
    assert_eq!(ext[0].hash, test_hash);

    // No declaration → all ship-relevant
    let (ship2, ext2) = partition_by_release_surface(root, &fixes, &[]);
    assert_eq!(ship2.len(), 2);
    assert!(ext2.is_empty());
}
