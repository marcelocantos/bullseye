// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! 🎯T100.1 — git ledger writer (commit-mode primitive).

use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;

use bullseye::commit_mode::{
    DEFAULT_REF, LEDGER_PATH, WriteError, ref_tip, write_ledger_at_ref,
    write_ledger_at_ref_with_retry,
};

fn init_repo_with_ledger(dir: &std::path::Path, yaml: &str) {
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
    std::fs::write(dir.join(LEDGER_PATH), yaml).unwrap();
    git(&["init", "-q", "-b", "master"]);
    git(&["config", "user.email", "t100@example.com"]);
    git(&["config", "user.name", "T100"]);
    git(&["config", "commit.gpgsign", "false"]);
    let hooks = dir.join(".git/empty-hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    git(&["add", LEDGER_PATH]);
    git(&["commit", "-q", "-m", "seed ledger"]);
}

fn show_ledger_at_ref(dir: &std::path::Path, ref_name: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["show", &format!("{ref_name}:{LEDGER_PATH}")])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
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

const SEED_YAML: &str = "schema_version: 5\ntargets: {}\n";

#[test]
fn one_write_lands_on_ref_tip() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_ledger(tmp.path(), SEED_YAML);
    let next = format!("{SEED_YAML}# marker: one-write\n");
    let res = write_ledger_at_ref_with_retry(
        tmp.path(),
        DEFAULT_REF,
        next.as_bytes(),
        "commit-mode one write",
        8,
    )
    .expect("write lands");
    let tip = ref_tip(tmp.path(), DEFAULT_REF).unwrap();
    assert_eq!(tip, res.commit_sha);
    assert_eq!(show_ledger_at_ref(tmp.path(), DEFAULT_REF), next);
}

#[test]
fn concurrent_conflicting_writes_both_land_with_retry() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_ledger(tmp.path(), SEED_YAML);
    let barrier = Arc::new(Barrier::new(2));
    let dir = tmp.path().to_path_buf();

    let handles = ["writer-a", "writer-b"]
        .into_iter()
        .map(|tag| {
            let barrier = Arc::clone(&barrier);
            let dir = dir.clone();
            thread::spawn(move || {
                barrier.wait();
                let body = format!("{SEED_YAML}# landed-by: {tag}\n");
                write_ledger_at_ref_with_retry(
                    &dir,
                    DEFAULT_REF,
                    body.as_bytes(),
                    &format!("commit-mode {tag}"),
                    8,
                )
            })
        })
        .collect::<Vec<_>>();

    let results = handles
        .into_iter()
        .map(|h| h.join().expect("join").expect("retry write"))
        .collect::<Vec<_>>();

    let tip = ref_tip(&dir, DEFAULT_REF).unwrap();
    for r in &results {
        assert!(
            is_ancestor(&dir, &r.commit_sha, &tip),
            "commit {} must be reachable from tip {}",
            r.commit_sha,
            tip
        );
    }
    assert_ne!(results[0].commit_sha, results[1].commit_sha);
}

#[test]
fn concurrent_single_attempt_one_conflicts() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_ledger(tmp.path(), SEED_YAML);
    let barrier = Arc::new(Barrier::new(2));
    let dir = tmp.path().to_path_buf();
    let handles = (0..2)
        .map(|n| {
            let barrier = Arc::clone(&barrier);
            let dir = dir.clone();
            let index = dir.join(format!(".git/index-race-{n}"));
            thread::spawn(move || {
                barrier.wait();
                let body = format!("{SEED_YAML}# race-{n}\n");
                write_ledger_at_ref(&dir, DEFAULT_REF, body.as_bytes(), "race", &index)
            })
        })
        .collect::<Vec<_>>();

    let outcomes = handles
        .into_iter()
        .map(|h| h.join().expect("join"))
        .collect::<Vec<_>>();

    let ok = outcomes.iter().filter(|r| r.is_ok()).count();
    let conflict = outcomes
        .iter()
        .filter(|r| matches!(r, Err(WriteError::Conflict)))
        .count();
    assert_eq!(ok, 1, "exactly one writer wins: {:?}", outcomes);
    assert_eq!(
        conflict, 1,
        "loser must see retriable conflict: {:?}",
        outcomes
    );
}
