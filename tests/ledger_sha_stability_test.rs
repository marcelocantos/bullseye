// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! A ledger commit SHA stays reachable once bullseye has shown it to a
//! caller (🎯T72).
//!
//! Attestation is the fleet's verification currency: a worker reports
//! the SHA its ledger write produced, and a reviewer who did not watch
//! the work re-checks the claim with `git merge-base --is-ancestor`.
//! That only works if the SHA survives. Under 🎯T22's original amend
//! rule it did not: any unpushed `HEAD` touching exactly
//! `bullseye.yaml` was eligible to be folded into, which is true of
//! *every* agent's ledger commit. On 2026-08-15 four amends in under
//! three minutes orphaned two SHAs that workers had already cited.
//!
//! These tests drive the real binary, because the boundary under test
//! is the process boundary — two agents writing one repo seconds apart
//! are two `bullseye` processes, and an in-process test cannot
//! establish that. The amend-eligibility state is per-process by
//! construction, so a second invocation is exactly the case that used
//! to destroy the first invocation's SHA.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_bullseye");

/// Run `git -C repo <args>`, panicking with captured output on failure
/// so scaffolding errors are diagnosable.
fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed in {repo:?}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// One `bullseye` invocation — its own process, as a second agent's
/// would be.
fn bullseye(repo: &Path, args: &[&str]) -> String {
    let out = Command::new(BIN)
        .args(args)
        .arg("--cwd")
        .arg(repo)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {BIN} {args:?}: {e}"));
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success(),
        "bullseye {args:?} failed ({:?}):\n{text}",
        out.status.code()
    );
    text
}

/// The reachability question a reviewer asks of a cited SHA.
fn is_ancestor(repo: &Path, sha: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge-base", "--is-ancestor", sha, "HEAD"])
        .status()
        .unwrap()
        .success()
}

/// A git repo with a stable identity, no inherited hooks, and one
/// non-ledger commit so the first ledger write lands as a commit of its
/// own.
fn repo_with_ledger() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    git(repo, &["init", "-q", "-b", "master"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    let empty = repo.join(".git/empty-hooks");
    std::fs::create_dir_all(&empty).unwrap();
    git(repo, &["config", "core.hooksPath", empty.to_str().unwrap()]);

    std::fs::write(repo.join("README.md"), "# project\n").unwrap();
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-q", "-m", "init"]);

    bullseye(repo, &["open", "--location", "in_repo"]);
    assert!(repo.join("bullseye.yaml").exists(), "ledger must exist");
    tmp
}

/// Two mutations from two separate `bullseye` processes — the shape of
/// two agents writing one ledger seconds apart. The SHA the first
/// produced must still be reachable from `HEAD` after the second lands.
///
/// Negative control: against the pre-🎯T72 amend rule this fails. The
/// first process's ledger commit is unpushed and touches exactly
/// `bullseye.yaml`, so the second process amends it, and the SHA the
/// first would have reported resolves to nothing reachable.
#[test]
fn a_sha_from_one_process_survives_another_process_mutation() {
    let tmp = repo_with_ledger();
    let repo = tmp.path();

    // Agent one writes, and reports the SHA it sees.
    bullseye(
        repo,
        &[
            "commit",
            "--op",
            "track",
            "--name",
            "First agent target",
            "--acceptance",
            "a",
        ],
    );
    let first = git(repo, &["rev-parse", "HEAD"]);
    assert!(
        is_ancestor(repo, &first),
        "precondition: the SHA is reachable when the first agent reports it"
    );

    // Agent two writes seconds later.
    bullseye(
        repo,
        &[
            "commit",
            "--op",
            "track",
            "--name",
            "Second agent target",
            "--acceptance",
            "b",
        ],
    );
    let second = git(repo, &["rev-parse", "HEAD"]);

    assert_ne!(second, first, "the second mutation must have committed");
    assert!(
        is_ancestor(repo, &first),
        "the first agent's cited SHA {first} was orphaned by the second agent's \
         write — HEAD is now {second}. A reviewer can no longer verify the \
         first agent's evidence, and `git gc` will make the failure silent."
    );

    // Content from both writes survives — this is about SHA stability,
    // not data loss, and the fix must not have traded one for the other.
    let ledger = std::fs::read_to_string(repo.join("bullseye.yaml")).unwrap();
    assert!(ledger.contains("First agent target"), "{ledger}");
    assert!(ledger.contains("Second agent target"), "{ledger}");
}

/// The same property over a longer interleaving: every SHA any of the
/// processes was shown remains reachable at the end.
#[test]
fn every_sha_shown_to_any_process_stays_reachable() {
    let tmp = repo_with_ledger();
    let repo = tmp.path();

    let mut shown = Vec::new();
    for n in 0..4 {
        bullseye(
            repo,
            &[
                "commit",
                "--op",
                "track",
                "--name",
                &format!("Target from agent {n}"),
                "--acceptance",
                "a",
            ],
        );
        shown.push(git(repo, &["rev-parse", "HEAD"]));
    }

    let orphaned: Vec<&String> = shown.iter().filter(|s| !is_ancestor(repo, s)).collect();
    assert!(
        orphaned.is_empty(),
        "SHAs shown to a caller and later orphaned: {orphaned:?}"
    );
}
