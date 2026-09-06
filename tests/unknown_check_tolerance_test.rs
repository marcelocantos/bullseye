// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! A newer check kind must not brick an older reader (🎯T85).
//!
//! THE TRAP, MEASURED. `Check` is an untagged serde enum. Before this
//! change, one unrecognised kind anywhere in a ledger failed the parse of
//! the *entire file*. Against bullseye 0.52.0 (33dc9ac), a two-target
//! ledger whose first target carried a `command:` check produced:
//!
//! ```text
//! code=invalid_args
//! message: failed to parse …/bullseye.yaml: targets.T1.checks:
//!          data did not match any variant of untagged enum Check
//!          at line 13 column 5
//! ```
//!
//! Not a degraded read — nothing loaded, including the target that had
//! no checks at all. A tool whose new field bricks older readers is a
//! trap for exactly the fleet that has to migrate.
//!
//! WHAT THIS TEST CAN AND CANNOT PROVE. It cannot fix binaries already
//! released: 0.52.0's enum has three variants and no catch-all, and no
//! change here reaches it. That failure is permanent and the migration
//! order is therefore "upgrade every consumer, then adopt the kind".
//!
//! What it does prove is that the trap is closed from here on: this
//! binary — playing the "older reader" against a kind invented after it
//! — loads the ledger, reports the unknown check, and refuses to call
//! the target verified. That is the property every future kind needs,
//! and it is asserted through the shipped binary rather than the
//! library, because the failure being guarded is a parse that happens
//! before any library API is reached.

use std::process::Command;

/// A ledger using a check kind no bullseye has ever defined, alongside a
/// target with a known kind and one with no checks.
const FUTURE_KIND_LEDGER: &str = r#"
schema_version: 5
targets:
  T1:
    name: Checked by a kind from the future
    status: identified
    value: 5.0
    cost: 3.0
    acceptance:
    - the future thing holds
    origin: manual
    discovered: 2026-09-06
    checks:
    - telepathy:
        medium: quantum
        expect_confidence: 0.99
  T2:
    name: Unrelated target with a known check
    status: identified
    value: 3.0
    cost: 2.0
    acceptance:
    - a command passes
    origin: manual
    discovered: 2026-09-06
    checks:
    - command:
        run: "true"
  T3:
    name: Unrelated target with no checks at all
    status: identified
    value: 1.0
    cost: 1.0
    acceptance:
    - something else holds
    origin: manual
    discovered: 2026-09-06
"#;

fn ledger_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("bullseye.yaml"), FUTURE_KIND_LEDGER).unwrap();
    tmp
}

fn run(dir: &std::path::Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_bullseye"))
        .args(args)
        .arg("--cwd")
        .arg(dir)
        .output()
        .expect("binary runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

#[test]
fn the_binary_loads_a_ledger_containing_a_kind_it_does_not_know() {
    let tmp = ledger_dir();
    let (code, out) = run(tmp.path(), &["query", "--view", "list"]);

    assert_eq!(code, 0, "listing must succeed:\n{out}");
    for id in ["T1", "T2", "T3"] {
        assert!(
            out.contains(id),
            "every target must still load, {id} is missing:\n{out}",
        );
    }
    assert!(
        !out.contains("did not match any variant"),
        "the untagged-enum parse failure must be gone:\n{out}",
    );
}

#[test]
fn an_unknown_kind_is_reported_not_silently_ignored() {
    let tmp = ledger_dir();
    let (code, out) = run(tmp.path(), &["plan-checks", "--id", "T1"]);

    assert_eq!(code, 0, "planning must succeed:\n{out}");
    assert!(
        out.to_lowercase().contains("unsupported"),
        "the plan must say the check cannot be run here:\n{out}",
    );
    assert!(
        out.contains("upgrade"),
        "and must tell the reader what to do about it:\n{out}",
    );
}

#[test]
fn a_target_whose_only_check_is_unknown_is_not_reported_as_verified() {
    // The dangerous failure is not "cannot run it", it is "cannot run it,
    // therefore fine". Exit 2 is deliberately not 0.
    let tmp = ledger_dir();
    let (code, out) = run(tmp.path(), &["run-checks", "--id", "T1"]);

    assert_eq!(
        code, 2,
        "an unrunnable check must not exit 0 — 'not checked' is not 'checked':\n{out}",
    );
    assert!(
        out.contains("NOT VERIFIED"),
        "and must say so in words:\n{out}",
    );
}

#[test]
fn a_known_check_still_runs_in_the_same_ledger() {
    // Tolerance must be surgical: the unknown kind on T1 must not
    // degrade T2, whose check this build understands perfectly well.
    let tmp = ledger_dir();
    let (code, out) = run(tmp.path(), &["run-checks", "--id", "T2"]);

    assert_eq!(code, 0, "T2's command check should pass:\n{out}");
    assert!(out.contains("PASS"), "{out}");
}

#[test]
fn an_unknown_kind_survives_a_write_by_this_binary() {
    // A reader that tolerates a kind but drops it on the next write is
    // worse than one that refuses the file: the loss is silent, and the
    // newer reader's check is simply gone.
    let tmp = ledger_dir();
    let (code, out) = run(
        tmp.path(),
        &[
            "commit",
            "--op",
            "track",
            "--id",
            "T3",
            "--name",
            "Renamed by an older reader",
        ],
    );
    assert_eq!(code, 0, "the write must succeed:\n{out}");

    let raw = std::fs::read_to_string(tmp.path().join("bullseye.yaml")).unwrap();
    assert!(
        raw.contains("telepathy") && raw.contains("expect_confidence"),
        "the unknown check must survive a write by a build that cannot \
         interpret it:\n{raw}",
    );
}
