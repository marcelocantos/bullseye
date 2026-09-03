// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn legacy_showcase_demonstration_keys_load_and_strip_on_save() {
    // 🎯T23 removed the `showcase` and `demonstration` fields from
    // the schema entirely (v3 → v4). Pre-v4 yaml files in the wild
    // still carry these keys; the loader must accept them silently
    // (serde drops unknown fields) and the next save must strip them.
    // This is the migration path — a one-shot, no-op rewrite.
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    // schema_version: 3 mimics a file written by v0.20.0..v0.26.0.
    // Both the new `showcase` field and its legacy `observable` alias
    // must be tolerated, plus the `demonstration` companion.
    let yaml = r#"
schema_version: 3
targets:
  T1:
    name: Legacy work A
    status: achieved
    value: 5
    cost: 3
    showcase: true
    demonstration: ran the binary and shared a screenshot
    acceptance:
      - done
    discovered: 2026-03-01
    achieved: 2026-03-15
  T2:
    name: Legacy work B
    status: identified
    value: 3
    cost: 2
    observable: true
    acceptance:
      - done
    discovered: 2026-03-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();

    // Load must succeed.
    let file = store::load(&path).expect("pre-v4 file must still load");
    assert!(file.targets.contains_key("T1"));
    assert!(file.targets.contains_key("T2"));

    // Save + read raw: the retired showcase/demonstration/observable
    // keys must all be stripped by the round-trip. Match the YAML key
    // shape (`<key>:`) so target names that legitimately mention these
    // words don't false-positive.
    store::save(&path, &file).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        !raw.contains("showcase:"),
        "saved file must not retain the retired `showcase` key; got:\n{raw}"
    );
    assert!(
        !raw.contains("demonstration:"),
        "saved file must not retain the retired `demonstration` key; got:\n{raw}"
    );
    assert!(
        !raw.contains("observable:"),
        "saved file must not retain the retired `observable` alias; got:\n{raw}"
    );
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
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: None,
        name: Some("Repo-scope target with no portfolio metadata".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["CI green".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
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

#[test]
fn put_child_of_auto_assigns_next_child_id() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "child-of-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let result = handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: Some("T1".to_string()),
        name: Some("Child allocated by Bullseye".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["child exists".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect("child_of create should succeed");

    let text = text_from_call_result(result);
    assert!(
        text.contains("🎯T1.1 "),
        "response should return allocated child id; got: {text}"
    );

    let file = store::load(&path).unwrap();
    assert!(file.targets.contains_key("T1.1"));
    assert_eq!(file.targets["T1.1"].name, "Child allocated by Bullseye");
    // 🎯T39.1: child_of is an umbrella edge, not a display prefix.
    assert_eq!(file.targets["T1"].depends_on, vec!["T1.1"]);
    assert_eq!(file.targets["T1"].status, Status::Converging);

    config::set_external_root_override(None);
}

#[test]
fn put_child_of_refuses_terminal_parent() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "child-of-terminal").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    {
        let mut file = store::load(&path).unwrap();
        file.targets.get_mut("T1").unwrap().status = Status::Achieved;
        file.targets.get_mut("T1").unwrap().achieved = Some(chrono::Local::now().date_naive());
        store::save(&path, &file).unwrap();
    }

    let err = handle_put(PutTool {
        reason: None,
        cwd,
        id: None,
        child_of: Some("T1".to_string()),
        name: Some("Spillover under achieved parent".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["child exists".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect_err("child_of on achieved parent must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("terminal") || msg.contains("Achieved"),
        "error should name the terminal parent: {msg}"
    );

    config::set_external_root_override(None);
}

#[test]
fn put_explicit_dotted_id_wires_umbrella() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "explicit-dotted").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    handle_put(PutTool {
        reason: None,
        cwd,
        id: Some("T1.1".to_string()),
        child_of: None,
        name: Some("Explicit dotted child".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["child exists".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect("explicit dotted create should succeed");

    let file = store::load(&path).unwrap();
    assert_eq!(file.targets["T1"].depends_on, vec!["T1.1"]);
    assert_eq!(file.targets["T1"].status, Status::Converging);

    config::set_external_root_override(None);
}

#[test]
fn put_explicit_dotted_id_refuses_terminal_parent() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "explicit-dotted-terminal").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    {
        let mut file = store::load(&path).unwrap();
        file.targets.get_mut("T1").unwrap().status = Status::Achieved;
        file.targets.get_mut("T1").unwrap().achieved = Some(chrono::Local::now().date_naive());
        store::save(&path, &file).unwrap();
    }

    let err = handle_put(PutTool {
        reason: None,
        cwd,
        id: Some("T1.1".to_string()),
        child_of: None,
        name: Some("Spillover under achieved parent".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["child exists".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect_err("explicit dotted create on achieved parent must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("terminal") || msg.contains("Achieved"),
        "error should name the terminal parent: {msg}"
    );

    config::set_external_root_override(None);
}

#[test]
fn put_status_achieved_refuses_active_dotted_children() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "put-achieve-family").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: Some("T1".to_string()),
        name: Some("Open child".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["child exists".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect("child_of should succeed");

    let err = handle_put(PutTool {
        reason: None,
        cwd,
        id: Some("T1".to_string()),
        child_of: None,
        name: None,
        value: None,
        cost: None,
        acceptance: None,
        context: None,
        status: Some("achieved".to_string()),
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect_err("status=achieved must refuse while T1.1 is active");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("T1.1") && msg.contains("umbrella"),
        "error should name the open child: {msg}"
    );

    let file = store::load(&path).unwrap();
    assert_eq!(file.targets["T1"].status, Status::Converging);

    config::set_external_root_override(None);
}

#[test]
fn achieve_refuses_active_dotted_children() {
    use bullseye::config::{self, Location};
    use bullseye::handler::{handle_put, handle_retire};
    use bullseye::store;
    use bullseye::tools::{PutTool, RetireTool};

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "achieve-family").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: Some("T1".to_string()),
        name: Some("Open child".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["child exists".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect("child_of should succeed");

    let err = handle_retire(RetireTool {
        cwd,
        id: "T1".to_string(),
        attestation: "parent looks done if you ignore the children".to_string(),
        actual_cost: None,
    })
    .expect_err("cannot achieve parent while T1.1 is active");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("T1.1") && msg.contains("umbrella"),
        "error should name the open child: {msg}"
    );

    let file = store::load(&path).unwrap();
    assert_eq!(file.targets["T1"].status, Status::Converging);

    config::set_external_root_override(None);
}

#[test]
fn put_rejects_child_of_with_explicit_id() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let _path = store::create_at(tmp.path(), Location::InRepo, "child-of-explicit-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let err = handle_put(PutTool {
        reason: None,
        cwd,
        id: Some("T1.9".to_string()),
        child_of: Some("T1".to_string()),
        name: Some("Ambiguous child".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["child exists".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect_err("id + child_of must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("mutually exclusive"),
        "error should explain id/child_of conflict: {msg}"
    );

    config::set_external_root_override(None);
}

#[test]
fn put_rejects_zero_valued_dotted_explicit_id() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let _path = store::create_at(tmp.path(), Location::InRepo, "zero-id-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let err = handle_put(PutTool {
        reason: None,
        cwd,
        id: Some("T1.0".to_string()),
        child_of: None,
        name: Some("Ambiguous zero child".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["child exists".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    })
    .expect_err("T1.0 must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("final segment is zero"),
        "error should explain .0 ambiguity: {msg}"
    );

    config::set_external_root_override(None);
}

/// `bullseye_set_aside` flips a target's status to `set_aside` and
/// records the rationale; the target is then excluded from `active()`,
/// included in `set_aside()`, and unblocks its dependents the same
/// way an achieved target would.
#[test]
fn set_aside_marks_target_terminal_with_reason() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "set-aside").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let reason = "deferred to v2.0 — UX needs more thought";
    let result = handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: reason.to_string(),
    });
    assert!(result.is_ok(), "set_aside should succeed: {result:?}");

    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.status, Status::SetAside);
    assert_eq!(t1.set_aside_reason.as_deref(), Some(reason));

    // Excluded from active(), included in set_aside().
    assert!(!file.active().contains_key("T1"));
    assert!(file.set_aside().contains_key("T1"));
    // And NOT counted as achieved — that's the whole point.
    assert!(!file.achieved().contains_key("T1"));

    config::set_external_root_override(None);
}

/// Empty / whitespace-only reasons are rejected — the rationale is
/// the load-bearing artefact of the disposition.
#[test]
fn set_aside_rejects_empty_reason() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "set-aside-empty").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    for bad in ["", "   ", "\n\t  "] {
        let result = handle_set_aside(SetAsideTool {
            cwd: cwd.clone(),
            id: "T1".to_string(),
            reason: bad.to_string(),
        });
        assert!(
            result.is_err(),
            "empty/whitespace reason must be rejected: input={bad:?}"
        );
    }

    // Target must remain untouched after rejected calls.
    let file = store::load(&path).unwrap();
    assert_eq!(file.targets["T1"].status, Status::Identified);
    assert!(file.targets["T1"].set_aside_reason.is_none());

    config::set_external_root_override(None);
}

/// Already-achieved targets cannot be set aside — that would be
/// rewriting the achievement record. Already-set-aside targets are a
/// no-op (idempotent reporting, original reason preserved).
#[test]
fn set_aside_refuses_achieved_and_is_idempotent() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "set-aside-achieved").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Mark T1 achieved out-of-band, then try to set it aside.
    {
        let mut file = store::load(&path).unwrap();
        file.targets.get_mut("T1").unwrap().status = Status::Achieved;
        store::save(&path, &file).unwrap();
    }

    let after_achieved = handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: "doesn't matter — should be refused".to_string(),
    });
    assert!(
        after_achieved.is_err(),
        "set_aside on an achieved target must be refused"
    );
    let still_achieved = store::load(&path).unwrap();
    assert_eq!(still_achieved.targets["T1"].status, Status::Achieved);

    // Seed a T2 we can exercise idempotency on.
    {
        let mut file = store::load(&path).unwrap();
        let mut t2 = file.targets["T1"].clone();
        t2.name = "Idempotency probe".to_string();
        t2.status = Status::Identified;
        t2.set_aside_reason = None;
        t2.depends_on = vec![];
        file.targets.insert("T2".to_string(), t2);
        store::save(&path, &file).unwrap();
    }

    // Idempotency: set T2 aside with reason A, then try to set it
    // aside again with reason B — original reason wins, no error.
    let original = "parked pending design discussion";
    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T2".to_string(),
        reason: original.to_string(),
    })
    .unwrap();
    let second = handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T2".to_string(),
        reason: "different reason".to_string(),
    });
    assert!(
        second.is_ok(),
        "second set_aside on already-set-aside target should not error: {second:?}"
    );
    let file = store::load(&path).unwrap();
    assert_eq!(
        file.targets["T2"].set_aside_reason.as_deref(),
        Some(original)
    );

    config::set_external_root_override(None);
}

/// Set-aside targets unblock their dependents the same way achieved
/// targets do — the frontier surfaces the dependent once the upstream
/// is in a terminal disposition, regardless of which kind.
#[test]
fn set_aside_dependency_unblocks_frontier() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "set-aside-frontier").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Seed T2 depending on T1, then set T1 aside. T2 should appear
    // in the frontier afterwards.
    {
        let mut file = store::load(&path).unwrap();
        let mut t2 = file.targets["T1"].clone();
        t2.name = "Dependent of T1".to_string();
        t2.status = Status::Identified;
        t2.set_aside_reason = None;
        t2.depends_on = vec!["T1".to_string()];
        file.targets.insert("T2".to_string(), t2);
        store::save(&path, &file).unwrap();
    }

    // Pre-condition: T2 is blocked.
    let pre = bullseye::graph::frontier(&store::load(&path).unwrap());
    let pre_ids: Vec<_> = pre.iter().map(|f| f.id.as_str()).collect();
    assert!(
        !pre_ids.contains(&"T2"),
        "T2 should be blocked while T1 is identified; frontier was {pre_ids:?}"
    );

    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: "won't fix — superseded by 🎯T57".to_string(),
    })
    .unwrap();

    let post = bullseye::graph::frontier(&store::load(&path).unwrap());
    let post_ids: Vec<_> = post.iter().map(|f| f.id.as_str()).collect();
    assert!(
        post_ids.contains(&"T2"),
        "T2 should unblock once T1 is set aside; frontier was {post_ids:?}"
    );
    assert!(
        !post_ids.contains(&"T1"),
        "T1 should not appear in the frontier once it's set aside"
    );

    config::set_external_root_override(None);
}

/// `bullseye_validate` flags `status: set_aside` without a reason as
/// a structural error, and a `set_aside_reason` set on a non-set-aside
/// status as a stale leftover.
#[test]
fn validate_flags_set_aside_reason_mismatch() {
    use bullseye::graph::validate;
    use bullseye::schema::{Status, TargetsFile};

    // Start from a real file so the surrounding fields are valid; then
    // mutate just the status / reason to exercise validation.
    let tmp = tempfile::tempdir().unwrap();
    let path = bullseye::store::create_at(
        tmp.path(),
        bullseye::config::Location::InRepo,
        "validate-set-aside",
    )
    .unwrap();
    let mut file: TargetsFile = bullseye::store::load(&path).unwrap();

    // Case 1: set_aside without reason → error.
    file.targets.get_mut("T1").unwrap().status = Status::SetAside;
    file.targets.get_mut("T1").unwrap().set_aside_reason = None;
    let errs = validate(&file);
    assert!(
        errs.iter().any(|e| e.contains("set_aside_reason")),
        "missing reason must be flagged; errors: {errs:?}"
    );

    // Case 2: set_aside with whitespace-only reason → still error.
    file.targets.get_mut("T1").unwrap().set_aside_reason = Some("   ".to_string());
    let errs = validate(&file);
    assert!(
        errs.iter().any(|e| e.contains("set_aside_reason")),
        "whitespace-only reason must be flagged; errors: {errs:?}"
    );

    // Case 3: reason set on a non-set-aside status → error.
    file.targets.get_mut("T1").unwrap().status = Status::Identified;
    file.targets.get_mut("T1").unwrap().set_aside_reason = Some("stale".to_string());
    let errs = validate(&file);
    assert!(
        errs.iter()
            .any(|e| e.contains("set_aside_reason") && e.contains("only valid")),
        "stale reason on non-set-aside status must be flagged; errors: {errs:?}"
    );

    // Case 4: clean — set_aside with a real reason. No error from us.
    file.targets.get_mut("T1").unwrap().status = Status::SetAside;
    file.targets.get_mut("T1").unwrap().set_aside_reason =
        Some("parked pending review".to_string());
    let errs = validate(&file);
    assert!(
        !errs.iter().any(|e| e.contains("set_aside")),
        "valid set-aside should not produce set-aside-related errors; errors: {errs:?}"
    );
}

/// `op=achieve` / `bullseye_retire` rejects missing and whitespace-only
/// attestation; with a real note it succeeds, persists the field, and
/// surfaces it on list / get / summary.
#[test]
fn achieve_requires_and_persists_attestation() {
    use bullseye::config::{self, Location};
    use bullseye::handler::{handle_commit, handle_list, handle_query, handle_retire};
    use bullseye::tools::{CommitTool, ListTool, QueryTool, RetireTool};

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "attestation").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Missing / empty / whitespace / trivial → reject; file untouched.
    for bad in [
        None,
        Some(""),
        Some("   "),
        Some("\n\t"),
        Some("done"),
        Some("OK"),
    ] {
        let result = handle_commit(CommitTool {
            cwd: cwd.clone(),
            op: "achieve".into(),
            id: Some("T1".into()),
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
            attestation: bad.map(str::to_string),
            reason: None,
            postponed_until: None,
            postpone_predicate: None,
            parent: None,
            mode: None,
            children: None,
            retire_reason: None,
            tail: None,
            owner: None,
        });
        assert!(
            result.is_err(),
            "achieve without real attestation must fail: input={bad:?}"
        );
        let err = format!("{result:?}");
        assert!(
            err.to_ascii_lowercase().contains("attestation"),
            "error should name attestation: {err}"
        );
    }

    let file = store::load(&path).unwrap();
    assert_eq!(file.targets["T1"].status, Status::Identified);
    assert!(file.targets["T1"].attestation.is_none());

    // Shim path: whitespace-only also fails.
    let shim_empty = handle_retire(RetireTool {
        cwd: cwd.clone(),
        id: "T1".into(),
        attestation: "  ".into(),
        actual_cost: None,
    });
    assert!(
        shim_empty.is_err(),
        "retire shim empty attestation must fail"
    );

    let note = "cargo test achieve_requires_and_persists_attestation green; SHA local dogfood";
    let ok = handle_retire(RetireTool {
        cwd: cwd.clone(),
        id: "T1".into(),
        attestation: note.into(),
        actual_cost: Some(2.0),
    });
    assert!(ok.is_ok(), "retire with attestation must succeed: {ok:?}");
    let body = format!("{ok:?}");
    assert!(
        body.contains("Attestation:") && body.contains(note),
        "success body should echo attestation: {body}"
    );

    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.status, Status::Achieved);
    assert_eq!(t1.attestation.as_deref(), Some(note));
    assert!(
        t1.context.contains("Achieved ") && t1.context.contains(note),
        "context should carry Achieved date line; got: {}",
        t1.context
    );
    assert_eq!(t1.actual_cost, Some(2.0));

    // view=target / list / summary surface the note.
    let get = handle_query(QueryTool {
        cwd: cwd.clone(),
        view: Some("target".into()),
        id: Some("T1".into()),
        filter: None,
        recent_days: None,
        momentum: None,
        frontier_details: None,
        scope: None,
        nodes: None,
        seeds: None,
        expand: None,
    })
    .expect("view=target");
    let get_body = format!("{get:?}");
    assert!(
        get_body.contains("attestation") && get_body.contains(note),
        "view=target must show attestation: {get_body}"
    );

    let list = handle_list(ListTool {
        cwd: cwd.clone(),
        filter: "achieved".into(),
    })
    .expect("list achieved");
    let list_body = format!("{list:?}");
    assert!(
        list_body.contains("attestation:") && list_body.contains(note),
        "list achieved must show attestation: {list_body}"
    );

    let summary = handle_query(QueryTool {
        cwd: cwd.clone(),
        view: Some("summary".into()),
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
    .expect("summary");
    let sum_body = format!("{summary:?}");
    assert!(
        sum_body.contains("attestation:") && sum_body.contains(note),
        "summary must show attestation: {sum_body}"
    );

    // validate: attestation on non-achieved is rejected.
    {
        let mut file = store::load(&path).unwrap();
        file.targets.get_mut("T1").unwrap().status = Status::Identified;
        file.targets.get_mut("T1").unwrap().achieved = None;
        // keep attestation
        let errs = bullseye::graph::validate(&file);
        assert!(
            errs.iter()
                .any(|e| e.contains("attestation") && e.contains("only valid")),
            "stale attestation on identified must error: {errs:?}"
        );
    }

    config::set_external_root_override(None);
}

/// Revert clears attestation so a later re-achieve must re-attest.
#[test]
fn revert_clears_attestation() {
    use bullseye::config::{self, Location};
    use bullseye::handler::{handle_retire, handle_revert};
    use bullseye::tools::{RetireTool, RevertTool};

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "attestation-revert").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    handle_retire(RetireTool {
        cwd: cwd.clone(),
        id: "T1".into(),
        attestation: "tests green; smoke on fixture".into(),
        actual_cost: None,
    })
    .expect("retire");

    handle_revert(RevertTool {
        cwd: cwd.clone(),
        id: "T1".into(),
        reason: "regression in CI".into(),
    })
    .expect("revert");

    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.status, Status::Converging);
    assert!(t1.attestation.is_none(), "attestation must clear on revert");
    assert!(t1.achieved.is_none());

    config::set_external_root_override(None);
}

// --- 🎯T20: envelope-leak guard ---
//
// Tests for the check_no_envelope_leak validator wired into every
// mutating handler. The four markers are:
//   "<invoke "   "</invoke>"   "<parameter "   "</parameter>"
// Generic tags like <context> or <tags> are NOT rejected.

/// Each of the four envelope markers must be rejected on handle_put.name.
/// The error message must name both the field and the marker.
#[test]
fn put_rejects_envelope_markers_in_name() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let _path = store::create_at(tmp.path(), Location::InRepo, "envelope-name-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let markers = ["<invoke ", "</invoke>", "<parameter ", "</parameter>"];
    for marker in markers {
        let result = handle_put(PutTool {
            reason: None,
            cwd: cwd.clone(),
            id: None,
            child_of: None,
            name: Some(format!("some {marker} name")),
            value: None,
            cost: None,
            acceptance: Some(vec!["CI green".to_string()]),
            context: None,
            status: None,
            depends_on: None,
            blocks: None,
            origin: None,
            tags: None,
        });
        let err = result.expect_err(&format!("marker `{marker}` in name must be rejected"));
        let msg = format!("{err:?}");
        assert!(
            msg.contains("name"),
            "error must name the field `name`; marker={marker:?}; got: {msg}"
        );
        assert!(
            msg.contains(marker.trim()),
            "error must name the marker; marker={marker:?}; got: {msg}"
        );
    }

    config::set_external_root_override(None);
}

/// Markers in context, acceptance items, tags, and origin are all rejected.
/// Tests the field names appear in the error messages.
#[test]
fn put_rejects_envelope_markers_in_other_fields() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let _path = store::create_at(tmp.path(), Location::InRepo, "envelope-fields-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let marker = "<invoke ";

    // context
    let r = handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: None,
        name: Some("Legit name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["CI green".to_string()]),
        context: Some(format!("context with {marker} leaked")),
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    let msg = format!("{:?}", r.expect_err("marker in context must be rejected"));
    assert!(
        msg.contains("context"),
        "error must name field `context`; got: {msg}"
    );

    // acceptance[0]
    let r = handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: None,
        name: Some("Legit name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec![format!("criterion {marker} bad")]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    let msg = format!(
        "{:?}",
        r.expect_err("marker in acceptance[0] must be rejected")
    );
    assert!(
        msg.contains("acceptance[0]"),
        "error must name field `acceptance[0]`; got: {msg}"
    );

    // tags[0]
    let r = handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: None,
        name: Some("Legit name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["CI green".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: Some(vec![format!("bad{marker}tag")]),
    });
    let msg = format!("{:?}", r.expect_err("marker in tags[0] must be rejected"));
    assert!(
        msg.contains("tags[0]"),
        "error must name field `tags[0]`; got: {msg}"
    );

    // origin
    let r = handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: None,
        name: Some("Legit name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["CI green".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,

        origin: Some(format!("{marker}bad-origin")),
        tags: None,
    });
    let msg = format!("{:?}", r.expect_err("marker in origin must be rejected"));
    assert!(
        msg.contains("origin"),
        "error must name field `origin`; got: {msg}"
    );

    config::set_external_root_override(None);
}

/// Legitimate angle-bracket content that is NOT an envelope marker must
/// pass validation — e.g. `<context>` or `<tags>` in prose.
#[test]
fn put_allows_legitimate_angle_bracket_prose() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let _path = store::create_at(tmp.path(), Location::InRepo, "envelope-prose-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // These contain angle brackets but are NOT envelope markers.
    let result = handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: None,
        name: Some("Valid name with <context> reference".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec![
            "Output matches <expected>".to_string(),
            "No <tags> leakage".to_string(),
        ]),
        context: Some("<context> See design doc </context> for details".to_string()),
        status: None,
        depends_on: None,
        blocks: None,

        origin: Some("<manual> 2026-04-26".to_string()),
        tags: Some(vec!["<visual>".to_string()]),
    });
    assert!(
        result.is_ok(),
        "angle-bracket prose that isn't an envelope marker must pass; got: {result:?}"
    );

    config::set_external_root_override(None);
}

/// When handle_put rejects a call due to an envelope-leak, the file on
/// disk must be unchanged (no partial write).
#[test]
fn put_file_unchanged_on_envelope_rejection() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "envelope-unchanged-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Record the file content before the rejected call.
    let before = std::fs::read_to_string(&path).unwrap();

    let result = handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: None,
        name: Some("Good name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["<invoke bad".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    assert!(
        result.is_err(),
        "envelope marker in acceptance must be rejected"
    );

    // File must be byte-for-byte identical after the rejected call.
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        before, after,
        "file must be unchanged when handle_put rejects an envelope marker"
    );

    config::set_external_root_override(None);
}

#[test]
fn put_rejects_invalid_control_character_and_leaves_file_unchanged() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "control-unchanged-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let before = std::fs::read_to_string(&path).unwrap();

    let result = handle_put(PutTool {
        reason: None,
        cwd: cwd.clone(),
        id: None,
        child_of: None,
        name: Some("Good name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["bad \u{0001} criterion".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    let err = result.expect_err("U+0001 in acceptance must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("acceptance[0]") && msg.contains("U+0001"),
        "error must name field and control code; got: {msg}"
    );

    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        before, after,
        "file must be unchanged when handle_put rejects an invalid control character"
    );
    store::load(&path).expect("file must remain loadable after rejected mutation");

    config::set_external_root_override(None);
}

#[test]
fn put_allows_newline_and_tab_in_context() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let _path = store::create_at(tmp.path(), Location::InRepo, "control-context-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let result = handle_put(PutTool {
        reason: None,
        cwd,
        id: None,
        child_of: None,
        name: Some("Context can have whitespace controls".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["done".to_string()]),
        context: Some("line one\n\tline two".to_string()),
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    assert!(
        result.is_ok(),
        "newline and tab should remain valid persisted prose: {result:?}"
    );

    config::set_external_root_override(None);
}

/// handle_import rejects markdown whose parsed targets carry envelope
/// markers in any free-text field, AND no YAML file is written.
#[test]
fn import_rejects_envelope_markers_in_parsed_markdown() {
    use bullseye::config;
    use bullseye::handler::handle_import;
    use bullseye::tools::ImportTool;

    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Markdown with a leaked closing-tag in the free-text description —
    // exactly the failure mode 🎯T20 was raised against.
    let md_path = tmp.path().join("targets.md");
    std::fs::write(
        &md_path,
        "# Targets\n\n## Active\n\n\
         ### 🎯T1 Some target\n\n\
         Description with </invoke> leaked from a malformed tool call.\n\n\
         - **Value**: 1\n\
         - **Cost**: 1\n\
         - **Acceptance**: ok\n\
         - **Status**: Identified\n\
         - **Discovered**: 2026-04-26\n",
    )
    .unwrap();

    let result = handle_import(ImportTool {
        cwd: cwd.clone(),
        path: Some(md_path.to_string_lossy().to_string()),
        location: Some("in_repo".to_string()),
        force: false,
    });
    let err = result.expect_err("import must reject envelope-marker leakage");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("</invoke>"),
        "error must name the marker; got: {msg}"
    );

    // No bullseye.yaml should have been written into the cwd.
    assert!(
        store::discover_anywhere(tmp.path()).is_none(),
        "import must not write a YAML file when validation rejects the input"
    );

    config::set_external_root_override(None);
}

/// handle_set_aside rejects an envelope marker in the reason field.
#[test]
fn set_aside_rejects_envelope_marker_in_reason() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "envelope-set-aside-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let result = handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: "deferred </invoke> because".to_string(),
    });
    let err = result.expect_err("envelope marker in reason must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("reason"),
        "error must name field `reason`; got: {msg}"
    );
    assert!(
        msg.contains("</invoke>"),
        "error must name the marker; got: {msg}"
    );

    // Target must remain un-set-aside.
    let file = store::load(&path).unwrap();
    assert_ne!(file.targets["T1"].status, Status::SetAside);

    config::set_external_root_override(None);
}

/// A non-conforming target ID — e.g. "T36.v1" that snuck in via a bad
/// tool call or hand edit — is a stylistic warning, not a structural
/// error. The graph operates fine on it (depends_on, verifies, frontier
/// resolution all key on the string itself), so frontier and convergence
/// must not block on it; otherwise the user has no way to retire or
/// set-aside the offending target without an out-of-band YAML edit.
#[test]
fn non_conforming_id_is_warning_not_blocking_error() {
    let mut file = load_fixture();
    // Inject a target with a non-conforming ID.
    let mut t = file.targets["T1"].clone();
    t.name = "Stand-in for an arbitrary check".to_string();
    file.targets.insert("T1.v1".to_string(), t);

    let warnings = graph::validate_warnings(&file);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("T1.v1") && w.contains("invalid target ID format")),
        "warnings must flag the non-conforming ID; got: {warnings:?}"
    );

    let errors = graph::validate_blocking(&file);
    assert!(
        !errors
            .iter()
            .any(|e| e.contains("invalid target ID format")),
        "ID format must NOT appear in blocking errors; got: {errors:?}"
    );

    // The legacy `validate()` (used by bullseye_validate's combined
    // report) still surfaces the warning — callers that want the union
    // get it.
    let combined = graph::validate(&file);
    assert!(
        combined
            .iter()
            .any(|e| e.contains("invalid target ID format")),
        "combined validate() must include the warning; got: {combined:?}"
    );
}

/// GitHub-issue mirror IDs (`GH<n>`, and the reserved multi-repo
/// `GH<repo>-<n>`) are a recognised namespace (🎯T34, 🎯T37): the ID *is*
/// the upstream issue number, so no local↔remote mapping is needed.
/// They must NOT trip the advisory ID-format warning, whereas a
/// malformed `GH` id (non-numeric tail) still does.
#[test]
fn gh_mirror_ids_are_conforming() {
    let mut file = load_fixture();
    let proto = file.targets["T1"].clone();
    for id in ["GH123", "GHbullseye-7"] {
        let mut t = proto.clone();
        t.name = format!("Mirror of {id}");
        file.targets.insert(id.to_string(), t);
    }
    // A malformed GH id (non-numeric tail) should still warn.
    let mut bad = proto.clone();
    bad.name = "Malformed mirror".to_string();
    file.targets.insert("GHabc".to_string(), bad);

    let warnings = graph::validate_warnings(&file);
    assert!(
        !warnings
            .iter()
            .any(|w| w.contains("GH123") || w.contains("GHbullseye-7")),
        "well-formed GH mirror IDs must not warn; got: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("GHabc") && w.contains("invalid target ID format")),
        "malformed GH id must still warn; got: {warnings:?}"
    );
}

// ── 🎯T15.2: strategy schema tests ──────────────────────────────────────────

/// The observed jevons case, end to end: a target set aside with a
/// reason, reopened, and achieved carries no `set_aside_reason` and
/// validates green — no hand-editing of the YAML.
#[test]
fn t64_set_aside_reopen_achieve_leaves_no_residue() {
    use bullseye::handler::handle_commit;

    let (_tmp, _shadow, path, cwd) = t64_repo("t64-jevons");

    // 1. Set aside with a reason (the field that later became illegal).
    let mut defer = t64_commit(&cwd, "defer");
    defer.id = Some("T1".into());
    defer.reason = Some("duplicate of T288".into());
    handle_commit(defer).expect("defer should succeed");
    assert_eq!(
        store::load(&path).unwrap().targets["T1"]
            .set_aside_reason
            .as_deref(),
        Some("duplicate of T288"),
    );

    // 2. Reopen it (set-aside targets reopen by patching status).
    let mut reopen = t64_commit(&cwd, "track");
    reopen.id = Some("T1".into());
    reopen.status = Some("converging".into());
    handle_commit(reopen).expect("reopen should succeed");
    let file = store::load(&path).unwrap();
    assert!(
        file.targets["T1"].set_aside_reason.is_none(),
        "leaving set_aside must drop its reason",
    );

    // 3. Achieve it.
    let mut achieve = t64_commit(&cwd, "achieve");
    achieve.id = Some("T1".into());
    achieve.attestation = Some("verified by cargo test (🎯T64 regression)".into());
    handle_commit(achieve).expect("achieve should succeed");

    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.status, Status::Achieved);
    assert!(t1.set_aside_reason.is_none(), "{t1:?}");
    assert!(t1.attestation.is_some(), "{t1:?}");
    assert!(
        graph::validate_blocking(&file).is_empty(),
        "ledger must validate green: {:?}",
        graph::validate_blocking(&file),
    );

    bullseye::config::set_external_root_override(None);
}

/// Achieving a set-aside target directly (without the reopen step) is
/// the shortest path to the same residue, and is equally clean.
#[test]
fn t64_achieve_straight_from_set_aside_clears_the_reason() {
    use bullseye::handler::handle_commit;

    let (_tmp, _shadow, path, cwd) = t64_repo("t64-direct");

    let mut defer = t64_commit(&cwd, "defer");
    defer.id = Some("T1".into());
    defer.reason = Some("parked pending design".into());
    handle_commit(defer).expect("defer should succeed");

    let mut achieve = t64_commit(&cwd, "achieve");
    achieve.id = Some("T1".into());
    achieve.attestation = Some("delivered after all — see SHA deadbeef".into());
    handle_commit(achieve).expect("achieve should succeed");

    let file = store::load(&path).unwrap();
    assert!(file.targets["T1"].set_aside_reason.is_none());
    assert!(graph::validate_blocking(&file).is_empty());

    bullseye::config::set_external_root_override(None);
}

/// A walk over every status transition the tools offer, asserting the
/// ledger validates green after each step. Covers all four
/// status-scoped field groups: postpone fields (active-only), achieved
/// date and attestation (achieved-only), and set_aside_reason
/// (set-aside-only).
#[test]
fn t64_status_walk_validates_green_after_every_transition() {
    use bullseye::handler::handle_commit;

    let (_tmp, _shadow, path, cwd) = t64_repo("t64-walk");

    let mut steps: Vec<(&str, bullseye::tools::CommitTool)> = Vec::new();

    let mut postpone = t64_commit(&cwd, "postpone");
    postpone.id = Some("T1".into());
    postpone.postponed_until = Some("2099-01-01".into());
    postpone.postpone_predicate = Some("upstream ships v2".into());
    steps.push(("postpone", postpone));

    let mut converging = t64_commit(&cwd, "track");
    converging.id = Some("T1".into());
    converging.status = Some("converging".into());
    steps.push(("converging", converging));

    // Achieve while still postponed — the postpone fields are
    // active-only and must not survive into the achievement.
    let mut achieve = t64_commit(&cwd, "achieve");
    achieve.id = Some("T1".into());
    achieve.attestation = Some("first achievement, 🎯T64 walk".into());
    steps.push(("achieve", achieve));

    let mut reopen = t64_commit(&cwd, "reopen");
    reopen.id = Some("T1".into());
    reopen.reason = Some("regression found downstream".into());
    steps.push(("reopen", reopen));

    let mut defer = t64_commit(&cwd, "defer");
    defer.id = Some("T1".into());
    defer.reason = Some("superseded by 🎯T2".into());
    steps.push(("defer", defer));

    let mut unpark = t64_commit(&cwd, "track");
    unpark.id = Some("T1".into());
    unpark.status = Some("identified".into());
    steps.push(("unpark", unpark));

    let mut reachieve = t64_commit(&cwd, "achieve");
    reachieve.id = Some("T1".into());
    reachieve.attestation = Some("second achievement, 🎯T64 walk".into());
    steps.push(("re-achieve", reachieve));

    for (label, op) in steps {
        handle_commit(op).unwrap_or_else(|e| panic!("step `{label}` failed: {e:?}"));
        let file = store::load(&path).unwrap();
        let errors = graph::validate_blocking(&file);
        assert!(
            errors.is_empty(),
            "after step `{label}` the ledger must validate green, got {errors:?} for {:?}",
            file.targets["T1"],
        );
    }

    let t1 = &store::load(&path).unwrap().targets["T1"];
    assert_eq!(t1.status, Status::Achieved);
    assert!(t1.set_aside_reason.is_none(), "{t1:?}");
    assert!(t1.postponed_until.is_none(), "{t1:?}");
    assert!(t1.postpone_predicate.is_none(), "{t1:?}");

    bullseye::config::set_external_root_override(None);
}

/// The invariant generalised: for *every* status, a target carrying
/// *every* status-scoped field keeps exactly the fields that status
/// allows and validates green after the clear. This is the test that a
/// newly-added status-scoped field cannot slip past — it reads the same
/// `STATUS_SCOPED_FIELDS` table the production code does.
#[test]
fn t64_every_status_scoped_field_clears_when_its_status_is_left() {
    use bullseye::schema::{OwnedBy, STATUS_SCOPED_FIELDS};

    assert!(
        !STATUS_SCOPED_FIELDS.is_empty(),
        "the status-scoped field table must not be empty",
    );

    for status in [
        Status::Identified,
        Status::Converging,
        Status::Achieved,
        Status::SetAside,
    ] {
        let mut file = load_fixture();
        let t = file.targets.get_mut("T1").unwrap();

        // Every status-scoped field populated at once, then the status
        // applied on top — the shape a forgetful transition produces.
        t.set_aside_reason = Some("stale reason".into());
        t.attestation = Some("stale attestation".into());
        t.achieved = Some(chrono::NaiveDate::from_ymd_opt(2026, 8, 10).unwrap());
        t.owned_by = Some(OwnedBy {
            owner: "someone".into(),
            reason: "driving it".into(),
        });
        t.postponed_until = Some(chrono::NaiveDate::from_ymd_opt(2099, 1, 1).unwrap());
        t.postpone_predicate = Some("stale predicate".into());
        t.status = status;

        // Before the clear, the illegal fields are exactly the ones the
        // table says are not legal on this status.
        let flagged: Vec<&str> = t
            .illegal_status_scoped_fields()
            .iter()
            .map(|f| f.name)
            .collect();
        let expected: Vec<&str> = STATUS_SCOPED_FIELDS
            .iter()
            .filter(|f| !f.is_legal_on(status))
            .map(|f| f.name)
            .collect();
        assert_eq!(flagged, expected, "status {status:?}");

        let cleared = t.clear_illegal_status_scoped_fields();
        assert_eq!(cleared, expected, "status {status:?}");
        assert!(
            t.illegal_status_scoped_fields().is_empty(),
            "clear must be idempotent-complete for {status:?}",
        );

        // set_aside additionally *requires* its reason, so the only
        // status where a fully-populated target can still be invalid is
        // handled by keeping the reason there.
        let errors = graph::validate_blocking(&file);
        assert!(
            errors.is_empty(),
            "status {status:?} must validate green after the clear, got {errors:?}",
        );
    }
}

/// 🎯T74.14: `RevertError::NotAchieved`'s message contains neither
/// "not found" nor "conflict" nor any other phrase `classify_message`
/// recognizes, so before `MutationError::Apply` and friends carried a
/// typed `CodedError`, this refusal fell through to `classify_message`'s
/// default arm and was misreported as `invalid_args` — wrong for what
/// is a business-rule refusal, not a malformed argument. The code must
/// now come from `RevertError::code()` regardless of wording.
#[test]
fn t74_14_revert_of_non_achieved_target_yields_validation_not_invalid_args() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_revert;
    use bullseye::tools::RevertTool;

    let tmp = tempfile::tempdir().unwrap();
    // T1 is created `identified` by default — never achieved.
    store::create_at(tmp.path(), Location::InRepo, "t74-14-revert").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let err = handle_revert(RevertTool {
        cwd,
        id: "T1".into(),
        reason: "checking the code on a not-yet-achieved target".into(),
    })
    .expect_err("reverting a non-achieved target must be refused");
    let msg = err.to_string();

    assert!(
        !msg.contains("not found") && !msg.contains("conflict"),
        "test premise broken — message now contains a magic phrase, so it \
         no longer exercises the substring-classification gap: {msg}"
    );
    assert!(
        msg.contains("code=validation"),
        "expected the specific `validation` code even though the message \
         omits every phrase `classify_message` recognizes: {msg}"
    );
    assert!(
        !msg.contains("code=invalid_args"),
        "must not fall back to the generic invalid_args default: {msg}"
    );

    config::set_external_root_override(None);
}
