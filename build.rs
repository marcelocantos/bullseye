// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Build-time git provenance (🎯T69).
//!
//! `--version` printed the crate version alone, so a binary built from a
//! fix and the released binary carrying the same unbumped version were
//! indistinguishable — exactly when provenance matters, during an
//! incident. This script derives the source commit from git at build
//! time and hands it to the crate as `BULLSEYE_BUILD_PROVENANCE`.
//!
//! Nothing here may fail the build: a source tarball, a vendored crate,
//! or a checkout with no git metadata degrades to the explicit
//! `unknown` marker.

use std::path::Path;
use std::process::Command;

/// Emitted when git cannot answer — never a silent empty string, so a
/// provenance-less binary says so out loud.
const UNKNOWN: &str = "unknown";

/// Short SHA width. Wide enough to stay unambiguous in this repo's
/// history for the foreseeable future, short enough to read aloud.
const SHA_WIDTH: &str = "--short=12";

fn main() {
    println!("cargo:rustc-env=BULLSEYE_BUILD_PROVENANCE={}", provenance());

    // Rerun triggers. `.git/HEAD` and the resolved ref cover commits and
    // branch switches; `src` covers the working-tree edits that flip the
    // dirty marker. A build script that names no trigger is rerun on any
    // package change, which would be accurate but rebuilds the crate for
    // unrelated files (docs, YAML) — this set is the cheaper subset that
    // still catches both provenance inputs.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    for path in git_watch_paths() {
        println!("cargo:rerun-if-changed={path}");
    }
}

/// `<short-sha>` or `<short-sha>-dirty`, or [`UNKNOWN`] with no git.
fn provenance() -> String {
    let Some(sha) = git(&["rev-parse", SHA_WIDTH, "HEAD"]) else {
        return UNKNOWN.to_string();
    };
    // `--porcelain` is empty exactly when the tree matches HEAD. A git
    // that errors here (permissions, index lock) must not silently claim
    // clean, so an absent answer marks the build dirty-unknown.
    match git(&["status", "--porcelain", "--untracked-files=no"]) {
        Some(status) if status.is_empty() => sha,
        Some(_) => format!("{sha}-dirty"),
        None => format!("{sha}-dirty?"),
    }
}

/// Run `git` in the crate directory, returning trimmed stdout on a
/// clean exit. `None` for "git could not answer" in every flavour:
/// binary missing, not a repository, non-zero exit, non-UTF-8 output.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Git metadata files worth watching, skipping any that do not exist —
/// cargo treats a missing `rerun-if-changed` path as always-changed,
/// which would rerun this script on every build.
fn git_watch_paths() -> Vec<String> {
    // `--git-path` resolves through worktrees and `.git` files, where a
    // hardcoded `.git/HEAD` would point at nothing.
    let mut paths = Vec::new();
    for name in ["HEAD", "index"] {
        if let Some(p) = git(&["rev-parse", "--git-path", name])
            && Path::new(&p).exists()
        {
            paths.push(p);
        }
    }
    // The ref HEAD points at: its file is what changes on a new commit
    // when HEAD itself (`ref: refs/heads/master`) does not.
    if let Some(head_ref) = git(&["symbolic-ref", "--quiet", "HEAD"])
        && let Some(p) = git(&["rev-parse", "--git-path", &head_ref])
        && Path::new(&p).exists()
    {
        paths.push(p);
    }
    paths
}
