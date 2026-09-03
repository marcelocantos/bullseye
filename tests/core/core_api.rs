// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn t45_commit_track_returns_envelope_and_frontier() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_commit;
    use bullseye::store;
    use bullseye::tools::CommitTool;

    let tmp = tempfile::tempdir().unwrap();
    store::create_at(tmp.path(), Location::InRepo, "t45-commit").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let result = handle_commit(CommitTool {
        cwd,
        op: "track".to_string(),
        id: None,
        child_of: None,
        name: Some("Envelope target".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["done".to_string()]),
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
    .expect("commit track should succeed");
    let out = text_from_call_result(result);
    assert!(out.contains("# result"), "missing envelope header: {out}");
    assert!(out.contains("ok: true"), "{out}");
    assert!(out.contains("op: track"), "{out}");
    assert!(out.contains("ids:"), "{out}");
    assert!(out.contains("frontier:"), "{out}");
    assert!(out.contains("Created"), "{out}");
    config::set_external_root_override(None);
}

#[test]
fn t45_query_default_view_is_context() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_query;
    use bullseye::store;
    use bullseye::tools::QueryTool;

    let tmp = tempfile::tempdir().unwrap();
    store::create_at(tmp.path(), Location::InRepo, "t45-query").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let result = handle_query(QueryTool {
        cwd,
        view: None,
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
    .expect("query context should succeed");
    let out = text_from_call_result(result);
    assert!(
        out.contains("Startup context") || out.contains("Frontier") || out.contains("Active"),
        "expected context-like output: {out}"
    );
    config::set_external_root_override(None);
}

#[test]
fn t45_open_without_file_is_not_initialized() {
    use bullseye::config;
    use bullseye::handler::handle_open;
    use bullseye::tools::OpenTool;

    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let err = handle_open(OpenTool {
        cwd,
        location: None,
        project_name: None,
        recent_days: None,
    })
    .expect_err("open without file should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("code=not_initialized") || msg.contains("not_initialized"),
        "expected not_initialized code: {msg}"
    );
    config::set_external_root_override(None);
}

#[test]
fn t45_commit_achieve_and_reopen_roundtrip() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_commit;
    use bullseye::store;
    use bullseye::tools::CommitTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "t45-life").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let created = handle_commit(CommitTool {
        cwd: cwd.clone(),
        op: "track".to_string(),
        id: Some("T9".to_string()),
        child_of: None,
        name: Some("Lifecycle".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["ok".to_string()]),
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
    .expect("track");
    let created_text = text_from_call_result(created);
    assert!(created_text.contains("ids: T9"), "{created_text}");

    let achieved = handle_commit(CommitTool {
        cwd: cwd.clone(),
        op: "achieve".to_string(),
        id: Some("T9".to_string()),
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
        attestation: Some("t45 lifecycle roundtrip: hermetic fixture".to_string()),
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
    .expect("achieve");
    assert!(text_from_call_result(achieved).contains("op: achieve"));

    let reopened = handle_commit(CommitTool {
        cwd,
        op: "reopen".to_string(),
        id: Some("T9".to_string()),
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
        reason: Some("premature".to_string()),
        postponed_until: None,
        postpone_predicate: None,
        parent: None,
        mode: None,
        children: None,
        retire_reason: None,
        tail: None,
        owner: None,
    })
    .expect("reopen");
    assert!(text_from_call_result(reopened).contains("op: reopen"));

    let file = store::load(&path).unwrap();
    assert_eq!(
        file.targets["T9"].status,
        bullseye::schema::Status::Converging
    );
    config::set_external_root_override(None);
}

#[test]
fn t45_immutable_achieved_uses_error_code() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "t45-imm").unwrap();
    // Starter creates T1; retire it then try content patch.
    let mut file = store::load(&path).unwrap();
    if let Some(t) = file.targets.get_mut("T1") {
        t.status = bullseye::schema::Status::Achieved;
        t.achieved = Some(chrono::Local::now().date_naive());
    }
    store::save(&path, &file).unwrap();

    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let err = handle_put(PutTool {
        reason: None,
        cwd,
        id: Some("T1".to_string()),
        child_of: None,
        name: Some("hacked".to_string()),
        value: None,
        cost: None,
        acceptance: None,
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect_err("content patch on achieved must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("immutable_achieved"),
        "expected immutable_achieved code: {msg}"
    );
    config::set_external_root_override(None);
}

// --- 🎯T42 / T43 / T46 -----------------------------------------------------
