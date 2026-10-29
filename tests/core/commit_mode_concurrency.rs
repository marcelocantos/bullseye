// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! 🎯T100.5 — commit-mode concurrent writers must not report ok on a lost update.

use std::process::Command;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bullseye::commit_mode::{DEFAULT_REF, LEDGER_PATH, ref_tip, set_persist_override_for_tests};
use bullseye::handler::handle_retire;
use bullseye::schema::Status;
use bullseye::store::{MutationError, load, with_locked_mutation};
use bullseye::tools::RetireTool;

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

/// How long one writer may keep retrying a lock timeout (🎯T109). A
/// `LockTimeout` means the writer never got the lock, so nothing was
/// written and retrying is what a real caller does. Under heavy host load
/// the serialized writes in these tests can outlast one 5 s `LOCK_WAIT`
/// (eight writes at load ~65 did). These oracles are about lost updates,
/// not lock latency; this bound still fails a writer that is truly starved.
const CONTENDED_WRITE_BUDGET: Duration = Duration::from_secs(120);

fn retry_lock_timeouts<T, E>(
    mut write: impl FnMut() -> Result<T, E>,
    is_lock_timeout: impl Fn(&E) -> bool,
) -> Result<T, E> {
    let deadline = Instant::now() + CONTENDED_WRITE_BUDGET;
    loop {
        match write() {
            Err(e) if is_lock_timeout(&e) && Instant::now() < deadline => continue,
            result => return result,
        }
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

fn is_ancestor(repo: &std::path::Path, ancestor: &str, head: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge-base", "--is-ancestor", ancestor, head])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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

#[derive(Clone)]
struct LandedWrite {
    marker: String,
    commit_sha: String,
}

#[test]
fn concurrent_commit_mode_tag_writes_all_reach_ref_tip() {
    let _guard = PersistOverrideGuard::set(true);

    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    let path = init_repo_with_ledger(repo, MINIMAL_YAML);
    let seed_tip = ref_tip(repo, DEFAULT_REF).expect("seed tip");

    const WRITERS: usize = 8;
    let barrier = Arc::new(Barrier::new(WRITERS));
    let landed = Arc::new(Mutex::new(Vec::<LandedWrite>::new()));

    let handles: Vec<_> = (0..WRITERS)
        .map(|i| {
            let path = path.clone();
            let repo = repo.to_path_buf();
            let barrier = Arc::clone(&barrier);
            let landed = Arc::clone(&landed);
            thread::spawn(move || {
                set_persist_override_for_tests(Some(true));
                let marker = format!("commit-mode-tag-{i}");
                barrier.wait();
                retry_lock_timeouts(
                    || {
                        with_locked_mutation(&path, |file| {
                            let t = file.targets.get_mut("T1").expect("T1");
                            t.tags.push(marker.clone());
                            Ok::<(), String>(())
                        })
                    },
                    |e| matches!(e, MutationError::LockTimeout { .. }),
                )
                .expect("commit-mode mutation must succeed under contention");
                let commit_sha = ref_tip(&repo, DEFAULT_REF).expect("tip after write");
                landed
                    .lock()
                    .expect("landed lock")
                    .push(LandedWrite { marker, commit_sha });
            })
        })
        .collect();

    for h in handles {
        h.join().expect("thread joins");
    }

    let final_tip = ref_tip(repo, DEFAULT_REF).expect("final tip");
    assert!(
        is_ancestor(repo, &seed_tip, &final_tip),
        "seed commit must remain reachable from final tip"
    );

    let writes = landed.lock().expect("landed lock");
    assert_eq!(writes.len(), WRITERS, "every writer reported Ok");
    for w in writes.iter() {
        assert!(
            is_ancestor(repo, &w.commit_sha, &final_tip),
            "landing commit {} for {} must be reachable from tip {}",
            w.commit_sha,
            w.marker,
            final_tip
        );
    }

    let on_ref = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show", &format!("{DEFAULT_REF}:{LEDGER_PATH}")])
        .output()
        .expect("git show");
    assert!(on_ref.status.success());
    let ref_body = String::from_utf8_lossy(&on_ref.stdout);

    for w in writes.iter() {
        assert!(
            ref_body.contains(&w.marker),
            "marker {} from ok write must appear on ref ledger",
            w.marker
        );
    }

    let cached = load(&path).expect("load after concurrent commit-mode writes");
    assert_eq!(
        cached.targets["T1"].tags.len(),
        WRITERS,
        "parse cache must reflect every landed tag"
    );
}

#[test]
fn concurrent_commit_mode_retire_achievements_all_land_on_ref() {
    let _guard = PersistOverrideGuard::set(true);

    let tmp = tempfile::tempdir().unwrap();
    let path = init_repo_with_ledger(tmp.path(), "");
    seed_three_targets(&path);
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["add", LEDGER_PATH]);
    git(&["commit", "-q", "-m", "three targets"]);

    let cwd = tmp.path().to_string_lossy().to_string();
    let repo = tmp.path().to_path_buf();
    let path_for_load = path.clone();
    let seed_tip = ref_tip(&repo, DEFAULT_REF).expect("seed tip");

    const WRITERS: usize = 3;
    let barrier = Arc::new(Barrier::new(WRITERS));
    let landed = Arc::new(Mutex::new(Vec::<LandedWrite>::new()));

    let handles: Vec<_> = (1..=WRITERS)
        .map(|n| {
            let cwd = cwd.clone();
            let repo = repo.clone();
            let barrier = Arc::clone(&barrier);
            let landed = Arc::clone(&landed);
            let id = format!("T{n}");
            thread::spawn(move || {
                set_persist_override_for_tests(Some(true));
                barrier.wait();
                // The handler's error is a rendered MCP error, so the lock
                // timeout is recognised by MutationError's Display text.
                retry_lock_timeouts(
                    || {
                        handle_retire(RetireTool {
                            cwd: cwd.clone(),
                            id: id.clone(),
                            attestation: format!("commit-mode retire-cas T{n}"),
                            actual_cost: None,
                            base: None,
                        })
                    },
                    |e| e.to_string().contains("waiting for exclusive lock"),
                )
                .expect("retire should succeed under commit-mode contention");
                let commit_sha = ref_tip(&repo, DEFAULT_REF).expect("tip after retire");
                landed.lock().expect("landed lock").push(LandedWrite {
                    marker: id,
                    commit_sha,
                });
            })
        })
        .collect();

    for h in handles {
        h.join().expect("thread joins");
    }

    let final_tip = ref_tip(&repo, DEFAULT_REF).expect("final tip");
    assert!(
        is_ancestor(&repo, &seed_tip, &final_tip),
        "seed must remain reachable from final tip"
    );

    let writes = landed.lock().expect("landed lock");
    assert_eq!(writes.len(), WRITERS);
    for w in writes.iter() {
        assert!(
            is_ancestor(&repo, &w.commit_sha, &final_tip),
            "retire landing {} must be ancestor of tip",
            w.commit_sha
        );
    }

    let file = load(&path_for_load).unwrap();
    for id in ["T1", "T2", "T3"] {
        assert_eq!(
            file.targets[id].status,
            Status::Achieved,
            "{id} must be achieved on ref-backed ledger after concurrent retire"
        );
    }
}
