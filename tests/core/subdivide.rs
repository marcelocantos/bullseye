// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn subdivide_add_mode_wires_dotted_parent_and_extends_dependents() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    handle_subdivide(SubdivideTool {
        cwd: cwd.clone(),
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![
            child_spec("Spillover A", &["does A"]),
            child_spec("Spillover B", &["does B"]),
        ],
        retire_reason: None,
        tail: None,
    })
    .expect("add-mode subdivide should succeed");

    let file = store::load(&path).unwrap();

    // Two children created as sub-targets of T1.
    assert!(file.targets.contains_key("T1.1"));
    assert!(file.targets.contains_key("T1.2"));
    assert_eq!(file.targets["T1.1"].name, "Spillover A");
    assert_eq!(file.targets["T1.2"].name, "Spillover B");
    assert_eq!(file.targets["T1.1"].origin, "subdivide(🎯T1)");

    // 🎯T39.1: dotted children make the parent an umbrella. Dependents
    // still gain the children alongside T1 (add vs aggregate).
    let t1 = &file.targets["T1"];
    assert_eq!(t1.status, Status::Converging);
    assert_eq!(t1.depends_on, vec!["T1.1", "T1.2"]);

    // Dependents gain the new children alongside T1.
    let t2 = &file.targets["T2"];
    assert_eq!(t2.depends_on, vec!["T1", "T1.1", "T1.2"]);
    let t3 = &file.targets["T3"];
    assert_eq!(t3.depends_on, vec!["T1", "T1.1", "T1.2"]);

    config::set_external_root_override(None);
}

#[test]
fn subdivide_aggregate_mode_makes_parent_umbrella() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "aggregate".to_string(),
        children: vec![
            child_spec("Sub A", &["does A"]),
            child_spec("Sub B", &["does B"]),
        ],
        retire_reason: None,
        tail: None,
    })
    .expect("aggregate-mode subdivide should succeed");

    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];

    // Parent now depends on the new children and moves to converging.
    assert_eq!(t1.depends_on, vec!["T1.1", "T1.2"]);
    assert_eq!(t1.status, Status::Converging);

    // Dependents untouched (still pointing at T1 only).
    let t2 = &file.targets["T2"];
    assert_eq!(t2.depends_on, vec!["T1"]);
    let t3 = &file.targets["T3"];
    assert_eq!(t3.depends_on, vec!["T1"]);

    config::set_external_root_override(None);
}

#[test]
fn subdivide_retire_mode_retires_parent_and_rewires_dependents() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "retire".to_string(),
        children: vec![
            child_spec("Spillover A", &["does A"]),
            child_spec("Spillover B", &["does B"]),
        ],
        retire_reason: Some("original scope met; A and B emerged during work".to_string()),
        tail: None,
    })
    .expect("retire-mode subdivide should succeed");

    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];

    // Parent retired with today's date and audit line in context.
    assert_eq!(t1.status, Status::Achieved);
    assert_eq!(t1.achieved, Some(chrono::Local::now().date_naive()));
    assert!(
        t1.context.contains("Subdivided ") && t1.context.contains("A and B emerged"),
        "retire-mode audit line missing or malformed; context: {:?}",
        t1.context,
    );

    // Dependents had T1 replaced by the children — original order
    // preserved, just spliced.
    let t2 = &file.targets["T2"];
    assert_eq!(t2.depends_on, vec!["T1.1", "T1.2"]);
    let t3 = &file.targets["T3"];
    assert_eq!(t3.depends_on, vec!["T1.1", "T1.2"]);

    config::set_external_root_override(None);
}

#[test]
fn subdivide_supports_explicit_child_ids() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::{SubdivideTool, SubdivisionChild};

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![SubdivisionChild {
            id: Some("T42".to_string()),
            name: "Top-level spillover".to_string(),
            acceptance: vec!["explicit ID".to_string()],
            context: None,
            tags: None,
            depends_on: None,
        }],
        retire_reason: None,
        tail: None,
    })
    .expect("explicit-id subdivide should succeed");

    let file = store::load(&path).unwrap();
    assert!(
        file.targets.contains_key("T42"),
        "explicit child id T42 should be created"
    );
    // Dependents pick up T42 alongside T1.
    assert_eq!(file.targets["T2"].depends_on, vec!["T1", "T42"]);

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_id_collision() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::{SubdivideTool, SubdivisionChild};

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![SubdivisionChild {
            id: Some("T2".to_string()), // already exists
            name: "Collider".to_string(),
            acceptance: vec!["nope".to_string()],
            context: None,
            tags: None,
            depends_on: None,
        }],
        retire_reason: None,
        tail: None,
    })
    .expect_err("id collision must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("T2"),
        "error should name the colliding id: {msg}"
    );

    // File untouched.
    let file = store::load(&path).unwrap();
    assert_eq!(file.targets["T2"].name, "Dependent T2");

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_terminal_parent() {
    use bullseye::config;
    use bullseye::handler::{handle_set_aside, handle_subdivide};
    use bullseye::tools::{SetAsideTool, SubdivideTool};

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    // Retire T2 directly (achieved).
    {
        let mut file = store::load(&path).unwrap();
        file.targets.get_mut("T2").unwrap().status = Status::Achieved;
        file.targets.get_mut("T2").unwrap().achieved = Some(chrono::Local::now().date_naive());
        store::save(&path, &file).unwrap();
    }

    // Achieved parent: rejected.
    let err = handle_subdivide(SubdivideTool {
        cwd: cwd.clone(),
        parent: "T2".to_string(),
        mode: "add".to_string(),
        children: vec![child_spec("nope", &["nope"])],
        retire_reason: None,
        tail: None,
    })
    .expect_err("subdivide on achieved parent must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Achieved") && msg.contains("bullseye_revert"),
        "error should mention Achieved + bullseye_revert hint: {msg}"
    );

    // Set aside T3 and check the same rejection path.
    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T3".to_string(),
        reason: "deferred for the test".to_string(),
    })
    .unwrap();
    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T3".to_string(),
        mode: "add".to_string(),
        children: vec![child_spec("nope", &["nope"])],
        retire_reason: None,
        tail: None,
    })
    .expect_err("subdivide on set-aside parent must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("SetAside") && msg.contains("bullseye_put"),
        "error should mention SetAside + bullseye_put hint: {msg}"
    );

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_empty_children() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![],
        retire_reason: None,
        tail: None,
    })
    .expect_err("empty children must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("at least one child"),
        "error should explain the empty-children rule: {msg}"
    );

    // File untouched — T1 still has no sub-targets.
    let file = store::load(&path).unwrap();
    assert!(!file.targets.contains_key("T1.1"));

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_invalid_mode() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (_tmp, _shadow, cwd) = subdivide_fixture();

    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "replace".to_string(),
        children: vec![child_spec("nope", &["nope"])],
        retire_reason: None,
        tail: None,
    })
    .expect_err("invalid mode must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("replace") && msg.contains("add"),
        "error should name the offending mode and list valid choices: {msg}"
    );

    config::set_external_root_override(None);
}

#[test]
fn subdivide_auto_assigns_unique_ids_for_multiple_children() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    // If T1.1 already exists (e.g. earlier sub-target), auto-assignment
    // must skip past it instead of colliding.
    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");
    {
        let mut file = store::load(&path).unwrap();
        let mut t11 = file.targets["T1"].clone();
        t11.name = "Pre-existing T1.1".to_string();
        t11.status = Status::Identified;
        t11.depends_on = vec![];
        file.targets.insert("T1.1".to_string(), t11);
        store::save(&path, &file).unwrap();
    }

    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![child_spec("First", &["a"]), child_spec("Second", &["b"])],
        retire_reason: None,
        tail: None,
    })
    .expect("auto-assign past T1.1 should succeed");

    let file = store::load(&path).unwrap();
    // Existing T1.1 untouched.
    assert_eq!(file.targets["T1.1"].name, "Pre-existing T1.1");
    // New children start at T1.2 and continue from there.
    assert_eq!(file.targets["T1.2"].name, "First");
    assert_eq!(file.targets["T1.3"].name, "Second");

    config::set_external_root_override(None);
}

// --- 🎯T27: reshape patterns (tail + named shapes) ------------------------

/// Diamond decomposition via subdivide-retire-with-tail. The fixture's
/// chain T1 ← {T2, T3} is reshaped into a diamond by retiring T1 into
/// four children (Design → Build ∥ Tests → Validate). Without `tail`,
/// T2 and T3 would gain edges to all four children; with
/// `tail=["T1.4"]` (the validate convergence node), they cleanly point
/// at the diamond's tail only.
#[test]
fn subdivide_retire_with_tail_rewires_dependents_to_tail_only() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::{SubdivideTool, SubdivisionChild};

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    let child = |id: &str, name: &str, deps: &[&str]| SubdivisionChild {
        id: Some(id.to_string()),
        name: name.to_string(),
        acceptance: vec!["done".to_string()],
        context: None,
        tags: None,
        depends_on: Some(deps.iter().map(|s| s.to_string()).collect()),
    };

    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "retire".to_string(),
        children: vec![
            child("T1.1", "Design", &[]),
            child("T1.2", "Build", &["T1.1"]),
            child("T1.3", "Tests", &["T1.1"]),
            child("T1.4", "Validate", &["T1.2", "T1.3"]),
        ],
        retire_reason: Some("decomposed into diamond shape".to_string()),
        tail: Some(vec!["T1.4".to_string()]),
    })
    .expect("diamond decomposition should succeed");

    let file = store::load(&path).unwrap();

    // Parent retired, audit line present.
    assert_eq!(file.targets["T1"].status, Status::Achieved);
    assert!(
        file.targets["T1"].context.contains("diamond shape"),
        "retire reason should land in parent context"
    );

    // Diamond children present with internal deps.
    assert_eq!(file.targets["T1.2"].depends_on, vec!["T1.1"]);
    assert_eq!(file.targets["T1.3"].depends_on, vec!["T1.1"]);
    assert_eq!(file.targets["T1.4"].depends_on, vec!["T1.2", "T1.3"]);

    // The load-bearing assertion: dependents rewire to ONLY the tail
    // node, not the whole subgraph. Without `tail`, the assertions
    // below would read `vec!["T1.1", "T1.2", "T1.3", "T1.4"]`.
    assert_eq!(file.targets["T2"].depends_on, vec!["T1.4"]);
    assert_eq!(file.targets["T3"].depends_on, vec!["T1.4"]);

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_tail_outside_retire_mode() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (_tmp, _shadow, cwd) = subdivide_fixture();

    for mode in ["add", "aggregate"] {
        let err = handle_subdivide(SubdivideTool {
            cwd: cwd.clone(),
            parent: "T1".to_string(),
            mode: mode.to_string(),
            children: vec![child_spec("Sub", &["does X"])],
            retire_reason: None,
            tail: Some(vec!["T1.1".to_string()]),
        })
        .expect_err("tail outside retire mode must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("tail") && msg.contains("retire"),
            "error should mention tail + retire (mode={mode}): {msg}",
        );
    }

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_empty_tail() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (_tmp, _shadow, cwd) = subdivide_fixture();

    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "retire".to_string(),
        children: vec![child_spec("Sub", &["does X"])],
        retire_reason: None,
        tail: Some(vec![]),
    })
    .expect_err("empty tail must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("tail"),
        "error should name the offending parameter: {msg}"
    );

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_tail_id_not_in_children() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::{SubdivideTool, SubdivisionChild};

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "retire".to_string(),
        children: vec![SubdivisionChild {
            id: Some("T1.1".to_string()),
            name: "Sole child".to_string(),
            acceptance: vec!["done".to_string()],
            context: None,
            tags: None,
            depends_on: None,
        }],
        retire_reason: None,
        tail: Some(vec!["T99".to_string()]),
    })
    .expect_err("tail ID outside children must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("T99"),
        "error should name the offending tail id: {msg}"
    );

    // File untouched — failure happens before any mutation lands.
    let file = store::load(&path).unwrap();
    assert!(!file.targets.contains_key("T1.1"));
    assert_eq!(file.targets["T1"].status, Status::Identified);

    config::set_external_root_override(None);
}
