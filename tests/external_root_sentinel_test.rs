// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! 🎯T102 — On 2026-10-22 the owner found test-shaped `id-history` JSON under
//! the real `~/.local/share/bullseye/` because spawned worker threads did not
//! inherit the parent thread's `set_external_root_override`. Prevention is
//! tested by redirecting the process default (`BULLSEYE_DATA_DIR`) to a
//! sentinel tempdir and asserting it stays empty after concurrent retire.
//!
//! This test is its own binary on purpose (🎯T110). `BULLSEYE_DATA_DIR` is
//! process-wide, so in a shared binary any parallel test without an override
//! wrote into the sentinel and failed it.

use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;

use bullseye::config;
use bullseye::handler::handle_retire;
use bullseye::tools::RetireTool;

const SEED_YAML: &str = r#"schema_version: 5
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

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git invocation failed");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn assert_directory_tree_empty(dir: &std::path::Path) {
    if !dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(dir).expect("read sentinel dir") {
        let path = entry.expect("sentinel dir entry").path();
        if path.is_dir() {
            assert_directory_tree_empty(&path);
        } else {
            panic!("🎯T102 sentinel external root must stay untouched; found {path:?}");
        }
    }
}

#[test]
fn concurrent_retire_does_not_touch_production_external_root_sentinel() {
    let sentinel = tempfile::tempdir().unwrap();
    // SAFETY: this is the only test in this binary, so no other thread
    // reads or writes the environment while it runs.
    unsafe { std::env::set_var("BULLSEYE_DATA_DIR", sentinel.path()) };

    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q", "-b", "master"]);
    git(tmp.path(), &["config", "user.email", "test@example.com"]);
    git(tmp.path(), &["config", "user.name", "Test"]);
    let empty_hooks = tmp.path().join(".git/empty-hooks");
    std::fs::create_dir_all(&empty_hooks).unwrap();
    git(
        tmp.path(),
        &["config", "core.hooksPath", empty_hooks.to_str().unwrap()],
    );
    std::fs::write(tmp.path().join("bullseye.yaml"), SEED_YAML).expect("seed yaml");
    git(tmp.path(), &["add", "bullseye.yaml"]);
    git(tmp.path(), &["commit", "-q", "-m", "seed"]);

    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    let shadow_root = shadow_tmp.path().to_path_buf();
    config::set_external_root_override(Some(shadow_root.clone()));

    const WRITERS: usize = 3;
    let barrier = Arc::new(Barrier::new(WRITERS));
    let handles: Vec<_> = (1..=WRITERS)
        .map(|n| {
            let cwd = cwd.clone();
            let barrier = Arc::clone(&barrier);
            let shadow_root = shadow_root.clone();
            let id = format!("T{n}");
            thread::spawn(move || {
                config::set_external_root_override(Some(shadow_root));
                barrier.wait();
                handle_retire(RetireTool {
                    cwd,
                    id,
                    attestation: format!("oracle retire-cas sentinel T{n}"),
                    actual_cost: None,
                    base: None,
                })
                .expect("retire should succeed under contention");
            })
        })
        .collect();

    for h in handles {
        h.join().expect("thread joins");
    }

    assert_directory_tree_empty(sentinel.path());
    config::set_external_root_override(None);
}
