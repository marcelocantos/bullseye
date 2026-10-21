// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! 🎯T100.2 — commit-mode persist path in `with_locked_mutation` / `with_locked_write`.

use std::process::Command;

use bullseye::commit_mode::{
    DEFAULT_REF, LEDGER_PATH, integration_ref_name, ref_tip, set_persist_override_for_tests,
};
use bullseye::schema::Status;
use bullseye::store::{HashCheck, load, load_and_check_hash, with_locked_mutation};

struct PersistOverrideGuard;

impl PersistOverrideGuard {
    fn set(enabled: bool) -> Self {
        set_persist_override_for_tests(Some(enabled));
        Self
    }
}

impl Drop for PersistOverrideGuard {
    fn drop(&mut self) {
        set_persist_override_for_tests(None);
    }
}

fn init_repo_with_ledger(dir: &std::path::Path, yaml: &str) -> std::path::PathBuf {
    let path = dir.join(LEDGER_PATH);
    std::fs::write(&path, yaml).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q", "-b", "master"]);
    git(&["config", "user.email", "t100@example.com"]);
    git(&["config", "user.name", "T100"]);
    git(&["config", "commit.gpgsign", "false"]);
    let hooks = dir.join(".git/empty-hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    git(&["add", LEDGER_PATH]);
    git(&["commit", "-q", "-m", "seed"]);
    path
}

const MINIMAL_YAML: &str = "\
schema_version: 5
last_evaluated: null
release_surface: []
targets:
  T1:
    name: one
    status: identified
    value: 0
    cost: 0
    acceptance: [a]
    discovered: 2026-01-01
";

#[test]
fn commit_mode_off_writes_working_tree_with_content_hash() {
    let _guard = PersistOverrideGuard::set(false);
    assert!(!bullseye::commit_mode::persist_enabled());

    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo_with_ledger(tmp.path(), MINIMAL_YAML);
    let before = std::fs::read_to_string(&path).unwrap();

    with_locked_mutation(&path, |file| {
        let t = file.targets.get_mut("T1").expect("T1");
        t.status = Status::Converging;
        Ok::<(), String>(())
    })
    .expect("mutation");

    let after = std::fs::read_to_string(&path).unwrap();
    assert_ne!(before, after, "working-tree file must be updated");
    let (_, check) = load_and_check_hash(&path).expect("load");
    assert_eq!(
        check,
        HashCheck::Ok,
        "on-disk hash must match serialized payload"
    );
}

#[test]
fn commit_mode_on_moves_integration_ref_not_working_tree() {
    let _guard = PersistOverrideGuard::set(true);
    assert!(bullseye::commit_mode::persist_enabled());

    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo_with_ledger(tmp.path(), MINIMAL_YAML);
    let worktree_before = std::fs::read_to_string(&path).unwrap();
    let parent_before = ref_tip(tmp.path(), DEFAULT_REF).expect("ref tip");

    with_locked_mutation(&path, |file| {
        let t = file.targets.get_mut("T1").expect("T1");
        t.status = Status::Converging;
        Ok::<(), String>(())
    })
    .expect("commit-mode mutation");

    let parent_after = ref_tip(tmp.path(), &integration_ref_name()).expect("ref tip");
    assert_ne!(parent_before, parent_after, "ref must advance");

    let on_ref = Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["show", &format!("{DEFAULT_REF}:{LEDGER_PATH}")])
        .output()
        .expect("git show");
    assert!(on_ref.status.success());
    let ref_body = String::from_utf8_lossy(&on_ref.stdout);
    assert!(
        ref_body.contains("converging"),
        "ref ledger must carry mutation; got:\n{ref_body}"
    );

    let worktree_after = std::fs::read_to_string(&path).unwrap();
    assert_ne!(
        worktree_before, worktree_after,
        "working tree must be refreshed to match ref tip"
    );
    assert!(
        worktree_after.contains("converging"),
        "refreshed worktree must match ref ledger"
    );
    assert_eq!(
        worktree_after, ref_body,
        "worktree file must match git show at ref"
    );

    let cached = load(&path).expect("load after commit-mode write");
    assert_eq!(
        cached.targets["T1"].status,
        Status::Converging,
        "parse cache must not serve pre-mutation state"
    );
}
