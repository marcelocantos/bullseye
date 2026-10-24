// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! 🎯T105 — restore absent targets from a pinned git revision.

use std::process::Command;

use bullseye::apply::{ApplyRequest, Fragment};
use bullseye::commit_mode::{LEDGER_PATH, set_persist_override_for_tests};
use bullseye::handler::restore_request;
use bullseye::restore::RestoreRequest;
use bullseye::store::{load, with_locked_mutation};

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

fn git(dir: &std::path::Path, args: &[&str]) {
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
}

fn init_repo(dir: &std::path::Path, yaml: &str) -> std::path::PathBuf {
    let path = dir.join(LEDGER_PATH);
    std::fs::write(&path, yaml).unwrap();
    git(dir, &["init", "-q", "-b", "master"]);
    git(dir, &["config", "user.email", "t105@example.com"]);
    git(dir, &["config", "user.name", "T105"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    let hooks = dir.join(".git/empty-hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    git(dir, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    git(dir, &["add", LEDGER_PATH]);
    git(dir, &["commit", "-q", "-m", "seed"]);
    path
}

const WITH_T1216: &str = "\
schema_version: 5
last_evaluated: null
release_surface: []
targets:
  T1:
    name: keeper
    status: identified
    value: 0
    cost: 0
    acceptance: [a]
    discovered: 2026-01-01
  T1216:
    name: specimen rollback row
    status: identified
    value: 0
    cost: 0
    acceptance: [row must survive restore]
    context: original context from a7b84f49 era
    origin: manual
    discovered: 2026-10-24
";

const WITHOUT_T1216: &str = "\
schema_version: 5
last_evaluated: null
release_surface: []
targets:
  T1:
    name: keeper
    status: identified
    value: 0
    cost: 0
    acceptance: [a]
    discovered: 2026-01-01
";

#[test]
fn restore_reinserts_absent_id_from_pinned_revision() {
    let _guard = PersistOverrideGuard::set(false);
    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo(tmp.path(), WITH_T1216);
    let source_sha = Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse");
    let source_sha = String::from_utf8_lossy(&source_sha.stdout).trim().to_string();

    std::fs::write(&path, WITHOUT_T1216).unwrap();
    git(tmp.path(), &["add", LEDGER_PATH]);
    git(tmp.path(), &["commit", "-q", "-m", "drop T1216"]);

    let cwd = tmp.path().to_string_lossy().to_string();
    let result = restore_request(
        &cwd,
        RestoreRequest {
            source_rev: source_sha.clone(),
            ids: vec!["T1216".into()],
            reason: "integrator rollback dropped tool-authored row".into(),
        },
    )
    .expect("restore must succeed");
    let text = format!("{result:?}");
    assert!(text.contains("ok") || text.contains("Restored"), "got {text}");

    let file = load(&path).expect("reload");
    let t = file.targets.get("T1216").expect("restored row");
    assert_eq!(t.name, "specimen rollback row");
    assert!(t.context.contains("original context"));
    assert!(t.context.contains(&source_sha));
    assert_eq!(file.targets.get("T1").unwrap().name, "keeper");
}

#[test]
fn restore_refuses_when_live_id_exists() {
    let _guard = PersistOverrideGuard::set(false);
    let tmp = tempfile::tempdir().unwrap();
    init_repo(tmp.path(), WITH_T1216);
    let source_sha = Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse");
    let source_sha = String::from_utf8_lossy(&source_sha.stdout).trim().to_string();

    let cwd = tmp.path().to_string_lossy().to_string();
    let err = restore_request(
        &cwd,
        RestoreRequest {
            source_rev: source_sha,
            ids: vec!["T1".into()],
            reason: "should refuse".into(),
        },
    )
    .expect_err("must refuse overwrite");
    assert!(format!("{err:?}").contains("code=conflict"));
}

#[test]
fn restore_commit_mode_persists_via_integration_ref() {
    let _guard = PersistOverrideGuard::set(true);
    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo(tmp.path(), WITH_T1216);
    let source_sha = Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse");
    let source_sha = String::from_utf8_lossy(&source_sha.stdout).trim().to_string();

    std::fs::write(&path, WITHOUT_T1216).unwrap();
    git(tmp.path(), &["add", LEDGER_PATH]);
    git(tmp.path(), &["commit", "-q", "-m", "drop T1216"]);

    let cwd = tmp.path().to_string_lossy().to_string();
    restore_request(
        &cwd,
        RestoreRequest {
            source_rev: source_sha,
            ids: vec!["T1216".into()],
            reason: "commit-mode restore path".into(),
        },
    )
    .expect("restore in commit mode");

    let on_ref = Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["show", &format!("HEAD:{LEDGER_PATH}")])
        .output()
        .expect("show");
    let body = String::from_utf8_lossy(&on_ref.stdout);
    assert!(body.contains("T1216:"), "ref tip must contain restored id");
}

#[test]
fn explicit_create_still_id_reserved_after_row_removed_from_live() {
    use bullseye::apply::apply;
    let _guard = PersistOverrideGuard::set(false);
    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo(tmp.path(), WITH_T1216);
    std::fs::write(&path, WITHOUT_T1216).unwrap();
    git(tmp.path(), &["add", LEDGER_PATH]);
    git(tmp.path(), &["commit", "-q", "-m", "drop T1216"]);

    let hist = bullseye::id_alloc::historical_ids(&path).expect("history");
    assert!(hist.contains("T1216"), "id must remain in git history");

    with_locked_mutation(&path, |file| {
        let err = apply(
            file,
            &ApplyRequest {
                targets: [(
                    "T1216".to_string(),
                    Fragment {
                        name: Some("recreate".into()),
                        acceptance: Some(vec!["a".into()]),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            },
            &hist,
        )
        .expect_err("apply recreate must stay id_reserved");
        assert_eq!(err.code, bullseye::api::ErrorCode::IdReserved);
        Ok::<(), bullseye::api::CodedError>(())
    })
    .expect("mutation wrapper");
}
