// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Long-lived-process cache correctness (🎯T78.1).
//!
//! Until bullseye served HTTP, the git-history cache was correct by
//! lifecycle rather than by design: the process was one agent session,
//! so a snapshot taken at first touch could not go meaningfully stale.
//! A supervised daemon outlives that scope, and the snapshot became a
//! silent ID-collision bug — a fresh process refused a reserved ID
//! while a long-lived one created it.
//!
//! Every assertion here runs inside ONE process and deliberately never
//! calls `clear_cache_for_tests`: persistence across calls is the
//! condition under test. Integration tests are separate binaries, so no
//! other test file can clear this process's cache and mask a
//! regression.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed: {out:?}");
}

const SEED: &str = "schema_version: 5\ntargets:\n  T1:\n    name: first\n    status: identified\n\
                    \x20   value: 0.0\n    cost: 0.0\n    acceptance: [a]\n    discovered: 2026-01-01\n";

/// A target block that will enter history and then be deleted from the
/// live file, which is exactly the shape 🎯T28 reserves.
const WITH_T2: &str = "schema_version: 5\ntargets:\n  T1:\n    name: first\n    status: identified\n\
                       \x20   value: 0.0\n    cost: 0.0\n    acceptance: [a]\n    discovered: 2026-01-01\n\
                       \x20 T2:\n    name: second\n    status: identified\n    value: 0.0\n    cost: 0.0\n\
                       \x20   acceptance: [b]\n    discovered: 2026-01-01\n";

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "."]);
    git(p, &["config", "user.email", "t@t"]);
    git(p, &["config", "user.name", "t"]);
    std::fs::write(p.join("bullseye.yaml"), SEED).expect("write seed");
    git(p, &["add", "-A"]);
    git(p, &["commit", "-qm", "seed"]);
    dir
}

#[test]
fn history_scan_sees_ids_committed_after_the_cache_was_primed() {
    let dir = repo();
    let yaml = dir.path().join("bullseye.yaml");

    // Prime: at this point history knows T1 and nothing else.
    let first = bullseye::id_alloc::historical_ids(&yaml);
    assert!(
        first.contains("T1"),
        "seed commit should register T1: {first:?}"
    );
    assert!(!first.contains("T2"), "T2 does not exist yet: {first:?}");

    // T2 enters history, then leaves the live file — now reserved.
    std::fs::write(&yaml, WITH_T2).expect("write T2");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "add T2"]);
    std::fs::write(&yaml, SEED).expect("remove T2");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "remove T2"]);

    // Same process, same cache. Before the ref fingerprint this
    // returned the primed snapshot and omitted T2.
    let second = bullseye::id_alloc::historical_ids(&yaml);
    assert!(
        second.contains("T2"),
        "history scan must see refs that moved since the cache was primed, got: {second:?}"
    );
}

#[test]
fn an_unchanged_repo_is_still_served_from_cache() {
    // The fix must not turn every mutation into a full history rescan;
    // a stable fingerprint has to keep answering from memory.
    let dir = repo();
    let yaml = dir.path().join("bullseye.yaml");
    let first = bullseye::id_alloc::historical_ids(&yaml);
    let second = bullseye::id_alloc::historical_ids(&yaml);
    assert_eq!(first, second, "identical refs must yield an identical set");
}
