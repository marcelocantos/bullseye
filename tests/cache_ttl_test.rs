// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! The TTL is wired, not merely defined (🎯T78.1).
//!
//! `resolve`'s workspace scan has no exactness check to fall back on:
//! repos appear and vanish by filesystem operations bullseye never
//! observes. Under stdio the process died with the session, so a stale
//! scan could not outlive the work that would notice it. Under a
//! daemon it can, so the TTL *is* the correctness mechanism here — and
//! a correctness mechanism nobody has watched fire is a claim, not a
//! guarantee.
//!
//! Its own binary, and single-threaded, because it sets a process-wide
//! environment override.

use std::path::Path;

fn make_repo(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("bullseye.yaml"),
        "schema_version: 5\ntargets: {}\n",
    )
    .expect("write ledger");
}

#[test]
fn a_repo_created_after_the_scan_is_found_once_the_ttl_lapses() {
    let ws = tempfile::tempdir().expect("tempdir");
    make_repo(ws.path(), "alpha");

    // A generous TTL first: the scan is memoised and must NOT notice a
    // repo that appears afterwards. This is the pre-fix behaviour, and
    // under a daemon it used to be permanent.
    unsafe { std::env::set_var(bullseye::cache::TTL_ENV, "3600") };
    let first = bullseye::resolve::resolve(ws.path(), "alpha");
    assert!(first.is_ok(), "seed repo should resolve: {first:?}");

    make_repo(ws.path(), "beta");
    let stale = bullseye::resolve::resolve(ws.path(), "beta");
    assert!(
        stale.is_err(),
        "within the TTL the memoised scan should not yet see beta — if this \
         passes, the cache is not memoising and the test proves nothing"
    );

    // Now expire everything. The next interaction must reload.
    unsafe { std::env::set_var(bullseye::cache::TTL_ENV, "0") };
    let reloaded = bullseye::resolve::resolve(ws.path(), "beta");
    assert!(
        reloaded.is_ok(),
        "once the TTL lapses the next interaction must rescan and find beta: {reloaded:?}"
    );
    assert!(reloaded.expect("resolved").ends_with("beta"));
}
