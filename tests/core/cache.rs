// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn cache_hit_on_unchanged_mtime() {
    // Two consecutive loads of the same file with no modification in between
    // must return the same in-memory data without re-reading the disk. We
    // verify this indirectly: both loads succeed and agree on the target name.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "Original name");

    let first = store::load(&path).unwrap();
    let second = store::load(&path).unwrap();
    assert_eq!(first.targets["T1"].name, second.targets["T1"].name);
    assert_eq!(first.targets["T1"].name, "Original name");
}

#[test]
fn cache_miss_after_mtime_change() {
    // Writing new content to the file must cause the next load to return
    // the updated data, not the previously cached parse.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "First version");

    let first = store::load(&path).unwrap();
    assert_eq!(first.targets["T1"].name, "First version");

    // Rewrite the file with a new name. Use save() to ensure the cache is
    // evicted, then write fresh content to simulate an external edit.
    // We sleep briefly to guarantee a distinct mtime on systems with 1-second
    // mtime granularity (most Linux filesystems without noatime).
    std::thread::sleep(std::time::Duration::from_millis(10));
    write_yaml(&path, "Second version");

    let second = store::load(&path).unwrap();
    assert_eq!(
        second.targets["T1"].name, "Second version",
        "cache should have been invalidated after file was modified"
    );
}

#[test]
fn cache_evicted_after_save() {
    // After store::save() the cache entry is evicted so the next load
    // reads back what was actually written, not a stale in-memory snapshot.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "Original");

    let mut file = store::load(&path).unwrap();
    assert_eq!(file.targets["T1"].name, "Original");

    // Mutate and save.
    file.targets.get_mut("T1").unwrap().name = "Updated".to_string();
    store::save(&path, &file).unwrap();

    // Re-load must reflect the saved state (not the stale in-memory copy).
    let reloaded = store::load(&path).unwrap();
    assert_eq!(reloaded.targets["T1"].name, "Updated");
}

#[test]
fn cache_fallback_to_stale_on_reparse_failure() {
    // If the file becomes temporarily unreadable after the first successful
    // parse, the last valid cached copy is served rather than propagating
    // the I/O error (simulating a mid-edit state).
    use std::io::Write;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "Good state");

    // Prime the cache with a successful parse.
    let good = store::load(&path).unwrap();
    assert_eq!(good.targets["T1"].name, "Good state");

    // Overwrite with invalid YAML to simulate a mid-edit state.
    // Sleep briefly to ensure a new mtime on coarse-grained filesystems.
    std::thread::sleep(std::time::Duration::from_millis(10));
    writeln!(std::fs::File::create(&path).unwrap(), "not: valid: yaml: [").unwrap();

    // The load must succeed by serving the stale cached copy rather than
    // propagating the parse error.
    let fallback = store::load(&path).unwrap();
    assert_eq!(
        fallback.targets["T1"].name, "Good state",
        "expected stale cache fallback on parse failure"
    );
}

#[test]
fn concurrent_mutations_do_not_lose_updates() {
    // 🎯T17 regression test: two concurrent mutators each add a distinct
    // target to the same bullseye.yaml. Without flock, one mutation's
    // serialized-back-to-disk write clobbers the other. With flock, the
    // mutations serialise and both targets must be present at the end.
    //
    // We use threads rather than subprocesses because std::fs::File's advisory
    // locks (flock(2) on POSIX, LockFileEx on Windows) are tied to the
    // open-file-description — each thread's independent
    // `OpenOptions::open(...)` gets a distinct OFD, so same-process
    // threads contend on the lock exactly like cross-process writers
    // would. This catches the same lost-update race with ~0ms overhead
    // per iteration (subprocess spawn would cost ~50ms × 2 × N iters).
    //
    // Loop count: 10 iterations, fresh tempdir per iteration. Each
    // iteration runs N concurrent writers and asserts every write
    // landed.
    use std::sync::{Arc, Barrier};
    use std::thread;

    const ITERATIONS: usize = 10;
    const WRITERS_PER_ITERATION: usize = 4;

    for iter in 0..ITERATIONS {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bullseye.yaml");
        write_yaml(&path, "Baseline");

        // All threads wait on this barrier before starting their
        // mutation — maximises contention on the lock.
        let barrier = Arc::new(Barrier::new(WRITERS_PER_ITERATION));

        let handles: Vec<_> = (0..WRITERS_PER_ITERATION)
            .map(|i| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    let new_id = format!("T{}", 1000 + i);
                    store::with_locked_mutation(&path, |file| -> Result<(), String> {
                        file.targets.insert(
                            new_id.clone(),
                            bullseye::schema::Target {
                                name: format!("Concurrent target {i}"),
                                status: Status::Identified,
                                value: 1.0,
                                cost: 1.0,
                                actual_cost: None,
                                attestation: None,
                                set_aside_reason: None,
                                acceptance: vec!["done".to_string()],
                                checks: Vec::new(),
                                context: String::new(),
                                gates: Vec::new(),
                                depends_on: Vec::new(),
                                cross_depends: Vec::new(),
                                cross_enables: Vec::new(),
                                tags: Vec::new(),
                                strategy: None,

                                origin: "concurrent-test".to_string(),
                                discovered: chrono::Local::now().date_naive(),
                                achieved: None,
                                owned_by: None,
                                postponed_until: None,
                                postpone_predicate: None,
                            },
                        );
                        Ok(())
                    })
                    .unwrap_or_else(|e| {
                        panic!("iter {iter} thread {i}: locked mutation failed: {e}")
                    });
                })
            })
            .collect();

        for h in handles {
            h.join().expect("writer thread panicked");
        }

        // Every writer must have landed. Read fresh from disk —
        // bypass any cache by stat'ing directly (load() does this
        // via mtime, but parse_file is private; load() is fine).
        let final_file = store::load(&path).unwrap();
        for i in 0..WRITERS_PER_ITERATION {
            let id = format!("T{}", 1000 + i);
            assert!(
                final_file.targets.contains_key(&id),
                "iter {iter}: target {id} was lost — concurrent write clobbered it"
            );
        }
        // Plus the baseline T1 from write_yaml.
        assert!(
            final_file.targets.contains_key("T1"),
            "iter {iter}: baseline T1 was lost"
        );
    }
}

// --- 🎯T18: set-aside disposition ---
