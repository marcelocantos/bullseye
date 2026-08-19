// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Mutations do not write yaml-only git commits (🎯T73).
//!
//! 🎯T22 auto-committed a dirty in-repo `bullseye.yaml` after every
//! mutation (and at the start of convergence). 🎯T72 narrowed amend
//! eligibility to a SHA this process created, so a second agent's
//! cited SHA stayed reachable — at the cost of one yaml-only commit
//! per process. 🎯T73 removes that rail: bullseye writes the file and
//! leaves it dirty. Durability is `/commit` (stage) and `/push`
//! (refuse if still dirty).
//!
//! These tests drive the real binary because the production path is
//! the CLI/MCP process, and because two invocations are the shape of
//! two agents writing one repo.

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

/// A git repo with a stable identity, no inherited hooks, and one
/// non-ledger commit so HEAD is not a yaml-only commit to start with.
fn repo_with_ledger() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    git(repo, &["init", "-q", "-b", "master"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
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

fn commit_count(repo: &Path) -> usize {
    git(repo, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap_or(0)
}

fn head_files(repo: &Path) -> Vec<String> {
    git(repo, &["show", "--pretty=format:", "--name-only", "HEAD"])
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Two mutations from two separate `bullseye` processes — the shape of
/// two agents writing one ledger seconds apart. Neither writes a
/// yaml-only git commit. Ledger content from both writes is on disk.
#[test]
fn two_process_mutations_do_not_create_yaml_only_commits() {
    let tmp = repo_with_ledger();
    let repo = tmp.path();
    let before = commit_count(repo);
    let head_before = git(repo, &["rev-parse", "HEAD"]);
    assert_eq!(before, 1, "fixture is one non-ledger commit");

    let first = bullseye(
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
    assert!(
        first.contains("ok: true") && first.contains("Created"),
        "first mutation must be acknowledged; got:\n{first}"
    );

    let second = bullseye(
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
    assert!(
        second.contains("ok: true") && second.contains("Created"),
        "second mutation must be acknowledged; got:\n{second}"
    );

    assert_eq!(
        commit_count(repo),
        before,
        "mutations must not add a yaml-only commit"
    );
    assert_eq!(
        git(repo, &["rev-parse", "HEAD"]),
        head_before,
        "HEAD must not move"
    );
    let log = git(repo, &["log", "--oneline"]);
    assert!(
        !log.contains("Update bullseye.yaml"),
        "git history must not gain a bullseye-produced yaml-only commit; got:\n{log}"
    );
    let files = head_files(repo);
    assert_ne!(
        files,
        vec!["bullseye.yaml".to_string()],
        "HEAD must not touch only bullseye.yaml; files={files:?}"
    );

    let ledger = std::fs::read_to_string(repo.join("bullseye.yaml")).unwrap();
    assert!(ledger.contains("First agent target"), "{ledger}");
    assert!(ledger.contains("Second agent target"), "{ledger}");
}

/// Four CLI mutations in a row still leave commit count and HEAD
/// unchanged. Negative control against the pre-T73 rail, which would
/// have produced one yaml-only commit per process.
#[test]
fn every_cli_mutation_leaves_git_history_untouched() {
    let tmp = repo_with_ledger();
    let repo = tmp.path();
    let before = commit_count(repo);
    let head_before = git(repo, &["rev-parse", "HEAD"]);

    for n in 0..4 {
        let out = bullseye(
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
        assert!(
            out.contains("ok: true"),
            "mutation {n} must succeed; got:\n{out}"
        );
    }

    assert_eq!(commit_count(repo), before);
    assert_eq!(git(repo, &["rev-parse", "HEAD"]), head_before);
    let log = git(repo, &["log", "--oneline", "--name-only"]);
    assert!(
        !log.contains("Update bullseye.yaml"),
        "git log must not contain a yaml-only auto-commit; got:\n{log}"
    );

    let ledger = std::fs::read_to_string(repo.join("bullseye.yaml")).unwrap();
    for n in 0..4 {
        assert!(
            ledger.contains(&format!("Target from agent {n}")),
            "missing target {n} in ledger:\n{ledger}"
        );
    }
}
