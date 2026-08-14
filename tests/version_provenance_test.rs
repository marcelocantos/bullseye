// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! `--version` carries build provenance (🎯T69).
//!
//! Two binaries built from different commits used to print the same
//! `bullseye 0.44.0`, because the crate version is the only identity in
//! the string and releases are owner-gated. These tests assert the
//! provenance component is present and well-formed, so deleting it from
//! `version.rs` (or letting `build.rs` stop emitting it) fails rather
//! than passing by silent omission.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_bullseye");

/// The marker `build.rs` emits when it finds no git metadata.
const UNKNOWN: &str = "unknown";

/// Run the binary and return stdout+stderr, asserting a clean exit.
fn run(args: &[&str]) -> String {
    let out = Command::new(BIN)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {BIN} {args:?}: {e}"));
    assert!(out.status.success(), "{BIN} {args:?} exited {}", out.status);
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

/// Extract the parenthesised provenance component, panicking with the
/// whole line when it is absent — the negative control's failure mode.
fn provenance_of(version_line: &str) -> &str {
    let (_, after) = version_line.split_once(" (").unwrap_or_else(|| {
        panic!("version string carries no provenance component: {version_line:?}")
    });
    after
        .split_once(')')
        .unwrap_or_else(|| panic!("provenance component is unterminated: {version_line:?}"))
        .0
}

#[test]
fn version_carries_a_provenance_component() {
    let out = run(&["--version"]);
    let line = out.lines().next().expect("--version printed nothing");
    assert!(
        line.starts_with(&format!("bullseye {}", bullseye::version::CRATE_VERSION)),
        "--version dropped the crate version: {line:?}"
    );
    let prov = provenance_of(line);
    assert!(!prov.is_empty(), "provenance component is empty: {line:?}");
    assert_eq!(
        prov,
        bullseye::version::PROVENANCE,
        "--version and the library constant disagree"
    );
}

/// A commit SHA, optionally marked dirty, or the explicit unknown
/// marker. Nothing else — a provenance that degrades to an empty string
/// or to the crate version repeated would pass a mere presence check.
#[test]
fn provenance_is_a_commit_sha_or_the_unknown_marker() {
    let prov = bullseye::version::PROVENANCE;
    if prov == UNKNOWN {
        return;
    }
    let sha = prov
        .strip_suffix("-dirty?")
        .or_else(|| prov.strip_suffix("-dirty"))
        .unwrap_or(prov);
    assert!(
        sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()),
        "provenance {prov:?} is neither a short SHA (± dirty marker) nor {UNKNOWN:?}"
    );
}

/// The whole point: the string distinguishes builds. It must not be
/// the crate version alone, which is what the released binary prints.
#[test]
fn version_string_differs_from_the_bare_crate_version() {
    assert_ne!(
        bullseye::version::VERSION,
        bullseye::version::CRATE_VERSION,
        "version string is indistinguishable from a same-version release binary"
    );
}

/// Every surface that prints a version prints the same one: `--help`
/// heads its output with the binary's identity, and an agent reading
/// help must not see a version that contradicts `--version`.
#[test]
fn help_output_names_the_build() {
    let out = run(&["--help"]);
    assert!(
        out.contains(bullseye::version::VERSION),
        "--help does not carry the provenance-bearing version"
    );
}
