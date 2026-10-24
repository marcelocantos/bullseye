// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! 🎯T105 — restore absent targets from a pinned git revision.

use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;

use bullseye::apply::{ApplyRequest, Fragment, apply};
use bullseye::commit_mode::{LEDGER_PATH, set_persist_override_for_tests};
use bullseye::handler::{apply_request, restore_request};
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

fn head_sha(dir: &std::path::Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
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

const LIVE_AFTER_DROP: &str = "\
schema_version: 5
last_evaluated: 2026-10-24
release_surface: [src/]
targets:
  T1:
    name: live keeper evolved
    status: converging
    value: 13
    cost: 5
    acceptance: [live acceptance only]
    context: live context not in source snapshot
    discovered: 2026-02-01
";

#[test]
fn restore_reinserts_absent_id_from_pinned_revision() {
    let _guard = PersistOverrideGuard::set(false);
    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo(tmp.path(), WITH_T1216);
    let source_sha = head_sha(tmp.path());

    std::fs::write(&path, WITHOUT_T1216).unwrap();
    git(tmp.path(), &["add", LEDGER_PATH]);
    git(tmp.path(), &["commit", "-q", "-m", "drop T1216"]);

    let cwd = tmp.path().to_string_lossy().to_string();
    restore_request(
        &cwd,
        RestoreRequest {
            source_rev: source_sha.clone(),
            ids: vec!["T1216".into()],
            reason: "integrator rollback dropped tool-authored row".into(),
        },
    )
    .expect("restore must succeed");

    let file = load(&path).expect("reload");
    let t = file.targets.get("T1216").expect("restored row");
    assert_eq!(t.name, "specimen rollback row");
    assert!(t.context.contains("original context"));
    assert!(t.context.contains(&source_sha));
    assert_eq!(file.targets.get("T1").unwrap().name, "keeper");
}

#[test]
fn restore_preserves_live_metadata_and_unrelated_targets() {
    let _guard = PersistOverrideGuard::set(false);
    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo(tmp.path(), WITH_T1216);
    let source_sha = head_sha(tmp.path());

    std::fs::write(&path, LIVE_AFTER_DROP).unwrap();
    git(tmp.path(), &["add", LEDGER_PATH]);
    git(tmp.path(), &["commit", "-q", "-m", "drop T1216 and evolve live T1"]);

    let live_before = load(&path).expect("live snapshot");
    assert_eq!(live_before.last_evaluated.as_deref(), Some("2026-10-24"));
    assert_eq!(live_before.release_surface, vec!["src/".to_string()]);
    let t1_before = live_before.targets.get("T1").expect("T1").clone();

    // Re-read source at pin via git show
    let source_yaml = Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["show", &format!("{source_sha}:{LEDGER_PATH}")])
        .output()
        .expect("show source");
    let source_text = String::from_utf8_lossy(&source_yaml.stdout);
    let snap = tmp.path().join("source-snapshot.yaml");
    std::fs::write(&snap, source_text.as_bytes()).expect("write snapshot");
    let source_parsed = load(&snap).expect("parse source");
    let source_t1 = source_parsed.targets.get("T1").expect("source T1");
    assert_ne!(t1_before.name, source_t1.name, "fixture must differ on T1");
    let source_t1216 = source_parsed.targets.get("T1216").expect("source T1216").clone();

    let cwd = tmp.path().to_string_lossy().to_string();
    restore_request(
        &cwd,
        RestoreRequest {
            source_rev: source_sha,
            ids: vec!["T1216".into()],
            reason: "rollback recovery".into(),
        },
    )
    .expect("restore");

    let live_after = load(&path).expect("after restore");
    assert_eq!(live_after.last_evaluated, live_before.last_evaluated);
    assert_eq!(live_after.release_surface, live_before.release_surface);
    assert_eq!(live_after.targets.get("T1").expect("T1"), &t1_before);

    let restored = live_after.targets.get("T1216").expect("T1216");
    assert_eq!(restored.name, source_t1216.name);
    assert_eq!(restored.acceptance, source_t1216.acceptance);
    assert_eq!(restored.origin, source_t1216.origin);
    assert_eq!(restored.discovered, source_t1216.discovered);
    assert!(
        restored.context.starts_with(source_t1216.context.as_str()),
        "only context may gain audit suffix"
    );
    assert!(restored.context.contains("Restored from git revision"));
}

#[test]
fn restore_refuses_when_live_id_exists() {
    let _guard = PersistOverrideGuard::set(false);
    let tmp = tempfile::tempdir().unwrap();
    init_repo(tmp.path(), WITH_T1216);
    let source_sha = head_sha(tmp.path());

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
fn restore_refuses_unrelated_fetched_object() {
    let _guard = PersistOverrideGuard::set(false);
    let live_dir = tempfile::tempdir().unwrap();
    let foreign_dir = tempfile::tempdir().unwrap();
    init_repo(live_dir.path(), WITHOUT_T1216);
    let foreign_path = init_repo(foreign_dir.path(), WITH_T1216);
    let foreign_sha = head_sha(foreign_dir.path());
    assert!(
        !foreign_path.starts_with(live_dir.path()),
        "separate repo roots"
    );

    git(
        live_dir.path(),
        &[
            "fetch",
            "--quiet",
            foreign_dir.path().to_str().unwrap(),
            &format!("refs/heads/master:refs/bullseye-foreign/{foreign_sha}"),
        ],
    );

    let cwd = live_dir.path().to_string_lossy().to_string();
    let err = restore_request(
        &cwd,
        RestoreRequest {
            source_rev: foreign_sha,
            ids: vec!["T1216".into()],
            reason: "must refuse unrelated history".into(),
        },
    )
    .expect_err("unrelated object must be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("code=validation") && msg.contains("no git history"),
        "got {msg}"
    );
    let file = load(&live_dir.path().join(LEDGER_PATH)).expect("live ledger");
    assert!(!file.targets.contains_key("T1216"));
}

#[test]
fn restore_commit_mode_persists_via_integration_ref() {
    let _guard = PersistOverrideGuard::set(true);
    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo(tmp.path(), WITH_T1216);
    let source_sha = head_sha(tmp.path());

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
fn concurrent_apply_and_restore_commit_mode_both_land() {
    let _guard = PersistOverrideGuard::set(true);
    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo(tmp.path(), WITH_T1216);
    let source_sha = head_sha(tmp.path());

    std::fs::write(&path, WITHOUT_T1216).unwrap();
    git(tmp.path(), &["add", LEDGER_PATH]);
    git(tmp.path(), &["commit", "-q", "-m", "drop T1216"]);

    let cwd = tmp.path().to_string_lossy().to_string();
    let barrier = Arc::new(Barrier::new(2));
    let cwd_apply = cwd.clone();
    let cwd_restore = cwd.clone();
    let source_for_restore = source_sha.clone();

    let apply_handle = thread::spawn({
        let barrier = Arc::clone(&barrier);
        move || {
            set_persist_override_for_tests(Some(true));
            barrier.wait();
            apply_request(
                &cwd_apply,
                ApplyRequest {
                    targets: [(
                        "T1".to_string(),
                        Fragment {
                            tags: Some(vec!["concurrent-apply-marker".into()]),
                            ..Default::default()
                        },
                    )]
                    .into_iter()
                    .collect(),
                    reason: Some("concurrent apply tag".into()),
                    ..Default::default()
                },
            )
            .expect("apply under contention");
        }
    });

    let restore_handle = thread::spawn({
        let barrier = Arc::clone(&barrier);
        move || {
            set_persist_override_for_tests(Some(true));
            barrier.wait();
            restore_request(
                &cwd_restore,
                RestoreRequest {
                    source_rev: source_for_restore,
                    ids: vec!["T1216".into()],
                    reason: "concurrent restore".into(),
                },
            )
            .expect("restore under contention");
        }
    });

    apply_handle.join().expect("apply thread");
    restore_handle.join().expect("restore thread");

    let file = load(&path).expect("final ledger");
    let t1 = file.targets.get("T1").expect("T1");
    assert!(
        t1.tags.iter().any(|t| t == "concurrent-apply-marker"),
        "apply effect must survive"
    );
    assert!(
        file.targets.contains_key("T1216"),
        "restore effect must survive"
    );
}

#[test]
fn explicit_create_still_id_reserved_after_row_removed_from_live() {
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
