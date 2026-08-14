// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Installed-toolchain reachability oracle (🎯T68).
//!
//! Every other test in this repo proves something about *source*. This
//! one proves something about *delivery*: that the binary agents
//! actually invoke can repair a ledger carrying status-scoped residue.
//!
//! THE INCIDENT. `heal_status_scoped_residue()` (🎯T64, 372a8aa,
//! 2026-08-10) fixed exactly this defect. It landed on master, the
//! suite went green on it, and no agent in the fleet could reach it:
//! `git tag --contains 372a8aa` was empty, the newest tag was v0.44.0,
//! and every installed binary reported 0.44.0. A ledger in the claudia
//! repo was unreadable for five days because of it — `view=frontier`
//! returned nothing but one validation error.
//!
//! WHY NO SOURCE-LEVEL CHECK CAN COVER THIS. The failure shape is
//! precisely *source-present, binary-absent*. A grep for
//! `heal_status_scoped_residue` passes on the broken fleet. So does
//! `cargo test`, because it builds the binary it then tests. Only a
//! probe that shells out to the *installed* binary can tell the two
//! states apart.
//!
//! WHY NOT A VERSION COMPARISON. Tempting, and wrong. At the time of
//! writing, `Cargo.toml` still says `version = "0.44.0"`, so the fixed
//! master build and the broken released build are version-*identical*:
//!
//! ```text
//! /opt/homebrew/bin/bullseye --version  ->  bullseye 0.44.0   (broken)
//! ./target/debug/bullseye --version     ->  bullseye 0.44.0   (fixed)
//! ```
//!
//! A version gate would have reported green on the broken fleet. The
//! probe below is therefore purely *behavioural*: it feeds the
//! installed binary a ledger carrying the defect and reads what it
//! says. The version is recorded and reported, never gated on.
//!
//! WHICH BINARY. `bullseye` as resolved from `PATH`, which is what
//! 🎯T68's acceptance names. On this fleet that resolves to
//! `/opt/homebrew/bin/bullseye`, the same binary `mcpbridge` launches
//! for the MCP surface (`~/.config/mcpbridge/bullseye.json` names it
//! explicitly), so PATH is a faithful proxy for the agent path. A
//! `CARGO_BIN_EXE_bullseye` artifact would defeat the entire test, so
//! a resolution landing inside this workspace is a hard failure, not a
//! fallback.
//!
//! WHEN THERE IS NO INSTALLED BINARY, this test FAILS. That is a
//! deliberate choice and the whole point: "no installed toolchain" is
//! not a neutral state, it is the state where no agent can reach any
//! repair path. Skipping silently is the exact failure mode 🎯T68
//! exists to prevent — a green suite that says nothing about delivery.
//! A contributor with no installed release can opt out explicitly with
//! `BULLSEYE_REACHABILITY_CHECK=skip`, which prints a loud banner
//! naming what went unverified. CI sets it because GitHub runners
//! carry no installed release; see the comment in `ci.yml`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Env var that names the installed binary explicitly, bypassing PATH.
const BIN_VAR: &str = "BULLSEYE_INSTALLED_BIN";

/// Env var that opts out of the reachability probe. Value must be
/// `skip`; anything else is ignored so a stray truthy value cannot
/// silently disable the gate.
const SKIP_VAR: &str = "BULLSEYE_REACHABILITY_CHECK";

/// A ledger whose single target walked set_aside -> reopen -> achieve
/// while retaining `set_aside_reason` — the shape observed in claudia
/// on 2026-08-15. `value`/`cost` are present so the file *parses* on
/// both arms: the only thing the two binaries may disagree about is
/// the residue, not the schema.
const RESIDUE_LEDGER: &str = "\
targets:
  T1:
    name: Residue fixture target
    status: achieved
    value: 5
    cost: 2
    acceptance:
      - Loads without validation error
    set_aside_reason: parked pending upstream fix
    achieved: 2026-07-11
    attestation: landed in abc1234
    discovered: 2026-07-01
";

/// The residue field. Its presence in `RESIDUE_LEDGER` is what makes
/// the fixture a real negative control; its presence in a binary's
/// validate output is what makes that binary broken.
const RESIDUE_FIELD: &str = "set_aside_reason";

/// How the installed binary was found, for the record.
struct Installed {
    path: PathBuf,
    version: String,
    route: String,
}

impl Installed {
    /// The line every assertion quotes, so a failure always names the
    /// binary and version it observed (🎯T68 acceptance).
    fn observed(&self) -> String {
        format!(
            "observed installed toolchain: {} (via {}) reporting {:?}",
            self.path.display(),
            self.route,
            self.version
        )
    }
}

/// True when `candidate` lives inside this workspace — a `cargo build`
/// artifact rather than an installed release.
fn is_workspace_artifact(candidate: &Path) -> bool {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .canonicalize()
        .unwrap_or_else(|_| manifest.to_path_buf());
    let real = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    real.starts_with(&root)
}

/// Resolve `bullseye` the way an agent's shell would: an explicit
/// override if given, otherwise the first hit on `PATH`.
fn resolve_installed() -> Result<(PathBuf, String), String> {
    if let Ok(explicit) = std::env::var(BIN_VAR) {
        let path = PathBuf::from(&explicit);
        if !path.is_file() {
            return Err(format!("{BIN_VAR}={explicit} does not name a file"));
        }
        return Ok((path, format!("${BIN_VAR}")));
    }

    let path_var = std::env::var("PATH").map_err(|_| "PATH is unset".to_string())?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("bullseye");
        if candidate.is_file() {
            return Ok((candidate, "PATH".to_string()));
        }
    }
    Err(format!(
        "no `bullseye` on PATH. This is a FAILURE, not a skip: an agent \
         on this machine has no toolchain to reach any repair path with. \
         Install a release (`brew install marcelocantos/tap/bullseye`), \
         or point {BIN_VAR} at one, or opt out loudly with \
         {SKIP_VAR}=skip."
    ))
}

/// Resolve and interrogate the installed binary, or return `None` when
/// the operator opted out (having printed a banner saying so).
fn installed() -> Option<Installed> {
    if std::env::var(SKIP_VAR).as_deref() == Ok("skip") {
        eprintln!(
            "\n\
             ==================================================================\n\
             SKIPPED: installed-toolchain reachability probe (🎯T68)\n\
             {SKIP_VAR}=skip was set. This run proves NOTHING about whether\n\
             the binary agents invoke can repair a ledger with status-scoped\n\
             residue. Source-level green does not imply delivered.\n\
             =================================================================="
        );
        return None;
    }

    let (path, route) = resolve_installed().unwrap_or_else(|e| panic!("{e}"));

    assert!(
        !is_workspace_artifact(&path),
        "resolved `bullseye` to {}, which is inside this workspace. \
         🎯T68 requires the INSTALLED binary; a cargo-built artifact \
         passes trivially and proves nothing about delivery.",
        path.display()
    );

    let out = Command::new(&path)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("failed to run {} --version: {e}", path.display()));
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();

    Some(Installed {
        path,
        version,
        route,
    })
}

/// Write the residue ledger into a fresh temp dir. Temp, never the
/// committed fixture: the fixed binary rewrites the file when it heals,
/// and a real repo ledger must never be the subject of a probe.
fn residue_ledger_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("create temp dir");
    std::fs::write(tmp.path().join("bullseye.yaml"), RESIDUE_LEDGER).expect("write ledger");
    tmp
}

/// Run the installed binary and return stdout+stderr.
fn run(bin: &Path, args: &[&str]) -> String {
    let out = Command::new(bin)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {} {args:?}: {e}", bin.display()));
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

/// The control's own control: the fixture must actually carry the
/// defect. Without this, a fixture that silently lost its residue would
/// make every probe below pass vacuously — the fixture is the thing
/// that makes the other tests a *negative* control, so it gets its own
/// assertion. Mirrors the discrimination assertion in
/// `cli_parity_test.rs`.
#[test]
fn residue_fixture_actually_carries_the_defect() {
    assert!(
        RESIDUE_LEDGER.contains(RESIDUE_FIELD),
        "fixture lost its `{RESIDUE_FIELD}` residue; every reachability \
         probe below would pass vacuously"
    );
    assert!(
        RESIDUE_LEDGER.contains("status: achieved"),
        "residue is only illegal because the status is `achieved`; \
         fixture must keep that pairing"
    );
}

/// THE ORACLE. A ledger carrying status-scoped residue must load
/// through the installed binary without a validation error.
///
/// Both arms of this control have been observed (🎯T68):
///
/// ```text
/// pre-fix  /opt/homebrew/bin/bullseye 0.44.0 (brew tap):
///     # Validation errors
///     T1: set_aside_reason is only valid on status set_aside (status is Achieved)
///
/// post-fix ./target/debug/bullseye (master, 372a8aa):
///     Valid: /.../bullseye.yaml (1 targets)
/// ```
///
/// Note the pre-fix arm exits **0** while reporting the error, so the
/// verdict is read from stdout, never from the exit status.
#[test]
fn installed_binary_loads_a_ledger_with_status_scoped_residue() {
    let Some(inst) = installed() else { return };
    let tmp = residue_ledger_dir();
    let dir = tmp.path().to_string_lossy().into_owned();

    let out = run(&inst.path, &["query", "--cwd", &dir, "--view", "validate"]);

    // Prove the probe read OUR fixture and not one discovered up-tree.
    assert!(
        out.contains(&dir),
        "validate output does not name the fixture at {dir}; the binary \
         may have discovered a different ledger.\n{}\n--- output ---\n{out}",
        inst.observed()
    );

    assert!(
        !out.contains(RESIDUE_FIELD),
        "REACHABILITY FAILURE (🎯T68): the installed binary still rejects \
         a ledger with status-scoped residue. The fix (🎯T64, 372a8aa) is \
         in source but NOT in the toolchain agents invoke — this is the \
         claudia outage of 2026-08-15, reproduced.\n{}\n--- validate output ---\n{out}",
        inst.observed()
    );
    assert!(
        out.contains("Valid:"),
        "validate did not report the healed ledger as valid.\n{}\n--- output ---\n{out}",
        inst.observed()
    );

    println!("🎯T68 reachability OK — {}", inst.observed());
}

/// The documented repair path must actually repair. `op=rehash` is the
/// explicit load-and-save round trip; on a toolchain predating 🎯T64 it
/// returns `ok: true` with `changed: (none)` and leaves the residue on
/// disk — the inert-repair symptom claudia-po reported.
#[test]
fn installed_binary_rehash_clears_residue_from_disk() {
    let Some(inst) = installed() else { return };
    let tmp = residue_ledger_dir();
    let dir = tmp.path().to_string_lossy().into_owned();
    let ledger = tmp.path().join("bullseye.yaml");

    let out = run(
        &inst.path,
        &[
            "commit",
            "--cwd",
            &dir,
            "--op",
            "rehash",
            "--reason",
            "🎯T68 reachability probe",
        ],
    );

    let on_disk = std::fs::read_to_string(&ledger).expect("re-read ledger");
    assert!(
        !on_disk.contains(RESIDUE_FIELD),
        "REACHABILITY FAILURE (🎯T68): `commit --op rehash` reported \
         success but left `{RESIDUE_FIELD}` on disk — the repair path is \
         inert on the installed toolchain.\n{}\n--- rehash output ---\n{out}\
         \n--- ledger on disk ---\n{on_disk}",
        inst.observed()
    );

    println!("🎯T68 rehash repair OK — {}", inst.observed());
}
