// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn id_alloc_historical_ids_sees_other_branch() {
    let _guard = t28_lock();
    use bullseye::id_alloc;
    let tmp = t28_repo_with_branched_id();
    let yaml = tmp.path().join("bullseye.yaml");

    let ids = id_alloc::historical_ids(&yaml);
    assert!(
        ids.contains("T1"),
        "master's T1 must be in history; got: {ids:?}"
    );
    assert!(
        ids.contains("T2") && ids.contains("T3"),
        "feature-branch T2 and T3 must be visible via `git log --all`; got: {ids:?}"
    );
}

#[test]
fn id_alloc_put_skips_branched_ids() {
    let _guard = t28_lock();
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;
    let tmp = t28_repo_with_branched_id();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Master's bullseye.yaml has only T1, so the in-memory-only
    // allocator would pick T2 — which collides with the feature
    // branch. The git-history-aware path must pick T4.
    let cwd = tmp.path().to_string_lossy().to_string();
    let result = handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: None,
        name: Some("Master's new target".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["done".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    assert!(result.is_ok(), "put should succeed: {result:?}");
    let text = text_from_call_result(result.unwrap());
    assert!(
        text.contains("ids: T4") || text.contains("🎯T4 "),
        "expected next free slot to skip T2 and T3; got: {text}"
    );
    assert!(
        text.contains("allocated_id_note:"),
        "create result must surface un-missable id note (🎯T55); got: {text}"
    );

    let _ = Location::InRepo; // keep import alive
    config::set_external_root_override(None);
}

#[test]
fn id_alloc_external_mode_falls_back_to_in_memory() {
    let _guard = t28_lock();
    use bullseye::id_alloc;
    // A plain tempdir with no `.git` — historical_ids must return an
    // empty set rather than panicking or hanging.
    bullseye::id_alloc::clear_cache_for_tests();
    let tmp = tempfile::tempdir().unwrap();
    let yaml = tmp.path().join("bullseye.yaml");
    std::fs::write(&yaml, "schema_version: 5\ntargets:\n  T1:\n    name: x\n    status: identified\n    value: 0\n    cost: 0\n    acceptance: [a]\n    discovered: 2026-01-01\n").unwrap();

    let ids = id_alloc::historical_ids(&yaml);
    assert!(
        ids.is_empty(),
        "no git repo → empty historical set; got: {ids:?}"
    );
}

#[test]
fn id_alloc_explicit_collision_with_branched_id_is_rejected() {
    let _guard = t28_lock();
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;
    let tmp = t28_repo_with_branched_id();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Try to explicitly create T2 — which doesn't exist on master's
    // tree but DOES exist on the feature branch's history. Must be
    // rejected with a message pointing at git history.
    let cwd = tmp.path().to_string_lossy().to_string();
    let err = handle_put(PutTool {
        reason: None,
        cwd,
        id: Some("T2".to_string()),
        child_of: None,
        name: Some("Trying to re-use a branched ID".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["done".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect_err("explicit T2 must be rejected — T2 exists in history");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("T2") && msg.contains("git history"),
        "error should mention T2 and git history: {msg}"
    );

    let _ = Location::InRepo;
    config::set_external_root_override(None);
}

#[test]
fn id_alloc_subdivide_skips_branched_subtarget_ids() {
    let _guard = t28_lock();
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;
    use std::process::Command;
    bullseye::id_alloc::clear_cache_for_tests();

    // Repo where T1 exists on master and a branch added T1.1.
    let tmp = tempfile::tempdir().unwrap();
    t28_git_init(tmp.path());
    let path = store::create_at(tmp.path(), Location::InRepo, "t28-sub").unwrap();
    t28_git(tmp.path(), &["add", "bullseye.yaml"]);
    t28_git(tmp.path(), &["commit", "-q", "-m", "init"]);

    t28_git(tmp.path(), &["checkout", "-q", "-b", "feature"]);
    {
        let mut file = store::load(&path).unwrap();
        let mut t11 = file.targets["T1"].clone();
        t11.name = "Branched T1.1".to_string();
        t11.depends_on = vec![];
        file.targets.insert("T1.1".to_string(), t11);
        store::save(&path, &file).unwrap();
    }
    t28_git(tmp.path(), &["add", "bullseye.yaml"]);
    t28_git(tmp.path(), &["commit", "-q", "-m", "feature: add T1.1"]);
    t28_git(tmp.path(), &["checkout", "-q", "master"]);
    bullseye::id_alloc::clear_cache_for_tests();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Master sees T1 only. Subdividing T1 should produce T1.2 (skipping
    // the branched T1.1), not T1.1 itself.
    let cwd = tmp.path().to_string_lossy().to_string();
    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![child_spec("First spillover", &["a"])],
        retire_reason: None,
        tail: None,
    })
    .expect("subdivide should succeed");

    let file = store::load(&path).unwrap();
    assert!(
        file.targets.contains_key("T1.2"),
        "subdivide should have picked T1.2 to avoid the branched T1.1; \
         targets: {:?}",
        file.targets.keys().collect::<Vec<_>>()
    );
    assert!(
        !file.targets.contains_key("T1.1"),
        "T1.1 is on the other branch, not master — must not appear on master here"
    );

    // Silence the unused-Command import on platforms where the test
    // body doesn't reference it directly.
    let _ = Command::new("git");
    config::set_external_root_override(None);
}

#[test]
fn id_alloc_memoises_until_refs_move() {
    let _guard = t28_lock();
    use bullseye::id_alloc;
    let tmp = t28_repo_with_branched_id();
    let yaml = tmp.path().join("bullseye.yaml");

    // First call seeds the cache. A second call after the repo's refs
    // move must RESCAN (🎯T78.1).
    //
    // This assertion is inverted from what it was. The old contract —
    // "process-scoped and intentionally stale" — was coherent while
    // bullseye was a stdio server: the process was one agent session,
    // so the snapshot could not outlive the scope it was taken for.
    // Serving MCP from a supervised daemon breaks that assumption, and
    // the stale snapshot became a silent ID-collision bug: a fresh
    // process refused an ID reserved in git history while a long-lived
    // one created it. Validity is now keyed on a ref fingerprint, so
    // memoisation survives (see the unchanged-repo case below) while
    // staleness does not.
    let first = id_alloc::historical_ids(&yaml);
    // Add a new commit that introduces T99 *after* the first scan.
    {
        let mut file = store::load(&yaml).unwrap();
        let today = chrono::Local::now().date_naive();
        file.targets.insert(
            "T99".to_string(),
            bullseye::schema::Target {
                name: "Added post-scan".to_string(),
                status: bullseye::schema::Status::Identified,
                value: 0.0,
                cost: 0.0,
                actual_cost: None,
                attestation: None,
                set_aside_reason: None,
                acceptance: vec!["a".to_string()],
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
        store::save(&yaml, &file).unwrap();
    }
    t28_git(tmp.path(), &["add", "bullseye.yaml"]);
    t28_git(tmp.path(), &["commit", "-q", "-m", "add T99"]);

    let second = id_alloc::historical_ids(&yaml);
    assert!(
        second.contains("T99"),
        "T99 entered history after the first scan; a moved ref must invalidate the cache"
    );
    assert!(
        first.is_subset(&second),
        "a rescan must not lose IDs the first scan found: {first:?} vs {second:?}"
    );

    // Memoisation still holds when nothing moved: same refs, same set,
    // so the fix did not turn every call into a full history rescan.
    let repeat = id_alloc::historical_ids(&yaml);
    assert_eq!(second, repeat, "an unchanged repo must still be memoised");

    // Clearing the cache is still supported for tests.
    bullseye::id_alloc::clear_cache_for_tests();
    let third = id_alloc::historical_ids(&yaml);
    assert!(
        third.contains("T99"),
        "after cache clear, fresh scan must include T99"
    );
}

#[test]
fn id_alloc_deleted_targets_remain_reserved() {
    let _guard = t28_lock();
    use bullseye::id_alloc;
    bullseye::id_alloc::clear_cache_for_tests();

    // master starts with T1, T2. A later commit deletes T2. T2 must
    // still appear in historical_ids — it was added once.
    let tmp = tempfile::tempdir().unwrap();
    t28_git_init(tmp.path());
    let path = store::create_at(
        tmp.path(),
        bullseye::config::Location::InRepo,
        "t28-deleted",
    )
    .unwrap();
    {
        let mut file = store::load(&path).unwrap();
        let today = chrono::Local::now().date_naive();
        file.targets.insert(
            "T2".to_string(),
            bullseye::schema::Target {
                name: "Will be deleted".to_string(),
                status: bullseye::schema::Status::Identified,
                value: 0.0,
                cost: 0.0,
                actual_cost: None,
                attestation: None,
                set_aside_reason: None,
                acceptance: vec!["a".to_string()],
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
        store::save(&path, &file).unwrap();
    }
    t28_git(tmp.path(), &["add", "bullseye.yaml"]);
    t28_git(tmp.path(), &["commit", "-q", "-m", "add T1, T2"]);

    // Delete T2.
    {
        let mut file = store::load(&path).unwrap();
        file.targets.remove("T2");
        store::save(&path, &file).unwrap();
    }
    t28_git(tmp.path(), &["add", "bullseye.yaml"]);
    t28_git(tmp.path(), &["commit", "-q", "-m", "remove T2"]);
    bullseye::id_alloc::clear_cache_for_tests();

    let ids = id_alloc::historical_ids(&path);
    assert!(
        ids.contains("T2"),
        "T2 was added once; even after deletion it must stay reserved. Got: {ids:?}"
    );
}

// --- 🎯T29: bullseye_resolve --------------------------------------------
