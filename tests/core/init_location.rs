// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

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
    // Force no server default so omitted location still prompts (🎯T61).
    bullseye::config::set_default_location_override(Some(None));

    let err = handle_init(InitTool {
        cwd: work.path().to_string_lossy().into_owned(),
        location: Some(String::new()), // empty → treated as omitted → prompt
        project_name: None,
    })
    .expect_err("empty location without default must surface as error");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Create bullseye.yaml for this repo where?"),
        "location prompt missing: {msg}"
    );
    bullseye::config::set_default_location_override(None);
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
        location: Some("in_repo".to_string()),
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
        location: Some("external".to_string()),
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
        location: Some("in_repo".to_string()),
        project_name: None,
    })
    .expect("first init should succeed");

    // Second init — even with a different location — is refused.
    let err = handle_init(InitTool {
        cwd,
        location: Some("external".to_string()),
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
        "schema_version: 1\ntargets:\n  T1:\n    name: IN_REPO_WINS\n    status: identified\n    value: 5\n    cost: 3\n    acceptance:\n      - a\n    origin: manual\n    discovered: 2026-01-01\n",
    )
    .unwrap();

    let mut shadow_file = store::shadow_path(shadow.path(), work.path());
    std::fs::create_dir_all(&shadow_file).unwrap();
    shadow_file.push("bullseye.yaml");
    std::fs::write(
        &shadow_file,
        "schema_version: 1\ntargets:\n  T1:\n    name: SHADOW_SHOULD_LOSE\n    status: identified\n    value: 5\n    cost: 3\n    acceptance:\n      - a\n    origin: manual\n    discovered: 2026-01-01\n",
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

// --- default_location create defaults (🎯T61) ---

#[test]
fn t61_create_with_default_external_lands_in_shadow() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_init;
    use bullseye::store;
    use bullseye::tools::InitTool;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    config::set_default_location_override(Some(Some(Location::External)));

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    handle_init(InitTool {
        cwd: cwd.clone(),
        location: None, // honour server default
        project_name: Some("t61".to_string()),
    })
    .expect("init with default external should succeed");

    assert!(
        !work.path().join("bullseye.yaml").exists(),
        "default external must not write in-repo"
    );
    let expected = store::target_path_for_new(work.path(), Location::External);
    assert!(
        expected.is_file(),
        "default external must land under shadow: {}",
        expected.display()
    );
    // Discover finds the external-only ledger.
    assert_eq!(
        store::discover_anywhere(work.path()).as_deref(),
        Some(expected.as_path())
    );

    config::set_default_location_override(None);
}

#[test]
fn t61_explicit_in_repo_overrides_external_default() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_init;
    use bullseye::store;
    use bullseye::tools::InitTool;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    config::set_default_location_override(Some(Some(Location::External)));

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    handle_init(InitTool {
        cwd,
        location: Some("in_repo".to_string()),
        project_name: None,
    })
    .expect("explicit in_repo must override external default");

    assert!(
        work.path().join("bullseye.yaml").is_file(),
        "explicit in_repo must write into cwd despite external default"
    );
    // Shadow must stay empty for this cwd.
    let mut shadow_file = store::shadow_path(shadow.path(), work.path());
    shadow_file.push("bullseye.yaml");
    assert!(
        !shadow_file.exists(),
        "override must not also write external"
    );

    config::set_default_location_override(None);
}

#[test]
fn t61_discover_external_only() {
    use bullseye::config::Location;
    use bullseye::store;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let path = store::create_at(work.path(), Location::External, "ext-only").unwrap();
    assert!(
        !work.path().join("bullseye.yaml").exists(),
        "setup: no in-repo file"
    );
    let found = store::discover_anywhere(work.path()).expect("must find external-only");
    assert_eq!(found, path);
}

#[test]
fn t61_discover_in_repo_only() {
    use bullseye::config::Location;
    use bullseye::store;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let path = store::create_at(work.path(), Location::InRepo, "in-only").unwrap();
    let found = store::discover_anywhere(work.path()).expect("must find in_repo-only");
    assert_eq!(found, path);
    assert_eq!(found, work.path().join("bullseye.yaml"));
}

#[test]
fn t61_open_honours_default_location_when_location_omitted() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_open;
    use bullseye::store;
    use bullseye::tools::OpenTool;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    config::set_default_location_override(Some(Some(Location::External)));

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    let text = text_from_call_result(
        handle_open(OpenTool {
            cwd,
            location: None,
            project_name: Some("open-default".to_string()),
            recent_days: None,
        })
        .expect("open with default_location should create + return context"),
    );
    assert!(
        text.contains("🎯T1") || text.contains("active"),
        "expected session context after create: {text}"
    );
    assert!(
        !work.path().join("bullseye.yaml").exists(),
        "open default external must not write in-repo"
    );
    assert!(
        store::discover_anywhere(work.path()).is_some(),
        "open must leave a discoverable external ledger"
    );

    config::set_default_location_override(None);
}

// --- Parse cache tests (🎯T13) ---
