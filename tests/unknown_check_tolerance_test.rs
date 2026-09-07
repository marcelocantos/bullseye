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

// --- Adoption is a deliberate act, not a side effect (🎯T86) -------------

const PLAIN_LEDGER: &str = r#"
schema_version: 5
targets:
  T1:
    name: A target with no checks yet
    status: identified
    value: 3.0
    cost: 2.0
    acceptance:
    - it works
    origin: manual
    discovered: 2026-09-06
"#;

fn plain_ledger_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("bullseye.yaml"), PLAIN_LEDGER).unwrap();
    tmp
}

#[test]
fn adding_the_first_command_check_is_refused_without_acknowledgement() {
    // A rollout rule that lives only in a report gets violated. The
    // constraint here is real and unfixable in code — a bullseye older
    // than the kind cannot read a ledger containing one, at all — so the
    // default has to be refusal.
    let tmp = plain_ledger_dir();
    let (code, out) = run(
        tmp.path(),
        &[
            "commit",
            "--op",
            "track",
            "--id",
            "T1",
            "--checks",
            r#"[{command: {run: "true"}}]"#,
        ],
    );

    assert_ne!(code, 0, "adoption must not succeed silently:\n{out}");
    assert!(
        out.contains("refusing to add the first `command` check"),
        "the refusal must name what is happening:\n{out}",
    );
    assert!(
        out.contains("--adopt-command-checks"),
        "and must say how to proceed deliberately:\n{out}",
    );

    let raw = std::fs::read_to_string(tmp.path().join("bullseye.yaml")).unwrap();
    assert!(
        !raw.contains("command"),
        "a refused adoption must leave the ledger untouched:\n{raw}",
    );
    assert!(
        raw.contains("schema_version: 5"),
        "and must not raise the stamp:\n{raw}",
    );
}

#[test]
fn acknowledged_adoption_succeeds_and_raises_the_stamp() {
    let tmp = plain_ledger_dir();
    let (code, out) = run(
        tmp.path(),
        &[
            "commit",
            "--op",
            "track",
            "--id",
            "T1",
            "--checks",
            r#"[{command: {run: "true"}}]"#,
            "--adopt-command-checks",
        ],
    );

    assert_eq!(code, 0, "acknowledged adoption should succeed:\n{out}");
    let raw = std::fs::read_to_string(tmp.path().join("bullseye.yaml")).unwrap();
    assert!(
        raw.contains("run: 'true'") || raw.contains("run: true"),
        "{raw}"
    );
    assert!(
        raw.contains("schema_version: 6"),
        "an adopting ledger is stamped at the adopting version:\n{raw}",
    );
}

#[test]
fn a_ledger_that_already_adopted_does_not_re_ask() {
    // The gate is about crossing the line, not about every later write.
    // Once a ledger is already unreadable to old binaries, a second
    // prompt buys nothing and would just train people to pass the flag.
    let tmp = plain_ledger_dir();
    let (code, _) = run(
        tmp.path(),
        &[
            "commit",
            "--op",
            "track",
            "--id",
            "T1",
            "--checks",
            r#"[{command: {run: "true"}}]"#,
            "--adopt-command-checks",
        ],
    );
    assert_eq!(code, 0);

    let (code, out) = run(
        tmp.path(),
        &[
            "commit",
            "--op",
            "track",
            "--id",
            "T1",
            "--checks",
            r#"[{command: {run: "false"}}, {command: {run: "true"}}]"#,
        ],
    );
    assert_eq!(code, 0, "a second command check needs no new gate:\n{out}");
}

#[test]
fn a_non_command_check_is_never_gated() {
    // Sawmill kinds are v5-compatible, so nothing about them should
    // trip a rollout gate.
    let tmp = plain_ledger_dir();
    let (code, out) = run(
        tmp.path(),
        &[
            "commit",
            "--op",
            "track",
            "--id",
            "T1",
            "--checks",
            r#"[{invariant: platform-isolation}]"#,
        ],
    );
    assert_eq!(code, 0, "a sawmill check must not be gated:\n{out}");
    let raw = std::fs::read_to_string(tmp.path().join("bullseye.yaml")).unwrap();
    assert!(
        raw.contains("schema_version: 5"),
        "and must not raise the stamp:\n{raw}",
    );
}

#[test]
fn a_too_new_ledger_says_upgrade_rather_than_naming_an_enum() {
    // ORDER IS THE WHOLE POINT (🎯T86). A ledger from the future fails
    // to deserialize either way — its kinds are not in this build's enum
    // — so if the struct parse runs before the version check, the reader
    // gets `data did not match any variant of untagged enum Check`
    // instead of "upgrade bullseye". Both are failures; only one tells
    // the reader what to do.
    //
    // Measured on 0.52.0, which checks the version *after* parsing: it
    // emits the useless message no matter what version a newer writer
    // stamps, which is why raising the stamp alone did not fix the
    // trap. This test pins the order so the next bump is legible.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("bullseye.yaml"),
        r#"
schema_version: 7
targets:
  T1:
    name: Uses a kind from two versions ahead
    status: identified
    value: 3.0
    cost: 2.0
    acceptance:
    - it works
    origin: manual
    discovered: 2026-09-06
    checks:
    - quantum_entanglement:
        qubits: 12
"#,
    )
    .unwrap();

    let (code, out) = run(tmp.path(), &["query", "--view", "list"]);
    assert_ne!(code, 0, "a too-new ledger must fail:\n{out}");
    assert!(
        out.contains("schema_version 7") && out.contains("only supports up to"),
        "the error must name the version gap:\n{out}",
    );
    assert!(
        out.to_lowercase().contains("upgrade"),
        "and must tell the reader what to do:\n{out}",
    );
    assert!(
        !out.contains("did not match any variant"),
        "the version check must run BEFORE deserialization, or the \
         actionable message is buried by a serde error:\n{out}",
    );
}
