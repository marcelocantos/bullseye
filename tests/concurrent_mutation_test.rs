// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Same-process concurrent mutation (🎯T78.1).
//!
//! Serving MCP from one supervised daemon means many agents' writes now
//! arrive as concurrent threads of a single process rather than as
//! separate short-lived processes. That raised a fair question: does
//! the flock in `with_locked_mutation` actually exclude *threads*, or
//! only processes?
//!
//! It excludes threads: `flock(2)` is per open file description, and
//! each acquisition opens its own descriptor, so two threads contend
//! exactly as two processes would. This test pins that, because the
//! failure it guards against is a lost update — silent, and only
//! reachable once the transport made same-process concurrency normal.
//!
//! It also means an in-process serialization layer would be a latency
//! and fairness optimisation, not a correctness fix.

#[test]
fn concurrent_mutations_in_one_process_do_not_lose_updates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let yaml = dir.path().join("bullseye.yaml");
    std::fs::write(
        &yaml,
        "schema_version: 5\ntargets:\n  T1:\n    name: n\n    status: identified\n\
         \x20   value: 0.0\n    cost: 0.0\n    acceptance: [a]\n    discovered: 2026-01-01\n",
    )
    .expect("seed");

    const THREADS: usize = 8;
    let handles: Vec<_> = (0..THREADS)
        .map(|i| {
            let p = yaml.clone();
            std::thread::spawn(move || {
                bullseye::store::with_locked_mutation(&p, |f| -> Result<(), String> {
                    let t = f.targets.get_mut("T1").ok_or("missing T1")?;
                    // Read-modify-write: a lost update shows up as a
                    // missing tag, which a last-writer-wins race would
                    // produce and a correct lock cannot.
                    t.tags.push(format!("tag{i}"));
                    Ok(())
                })
            })
        })
        .collect();

    let mut succeeded = 0;
    let mut errors = Vec::new();
    for h in handles {
        match h.join().expect("thread joins") {
            Ok(()) => succeeded += 1,
            Err(e) => errors.push(e.to_string()),
        }
    }

    let landed = bullseye::store::load(&yaml).expect("reload").targets["T1"]
        .tags
        .len();
    assert!(
        errors.is_empty(),
        "no mutation should fail under same-process contention: {errors:?}"
    );
    assert_eq!(succeeded, THREADS, "every thread should have committed");
    assert_eq!(
        landed, THREADS,
        "every successful mutation must survive — {landed} of {THREADS} tags present means a lost update"
    );
}
