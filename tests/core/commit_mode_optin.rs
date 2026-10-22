// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! 🎯T100.4 — commit-mode opt-in surface (external shadow, integration ref guard).

use std::process::Command;

use bullseye::commit_mode::{integration_ref_name, set_persist_override_for_tests};
use bullseye::config::{Location, set_external_root_override};
use bullseye::repo_guard::{MutationGuard, check_commit_mode_integration_ref};
use bullseye::schema::Status;
use bullseye::store::{
    create_at, is_external_ledger_path, persist_enabled_for, shadow_path, with_locked_mutation,
};
use tempfile::TempDir;

struct PersistOverrideGuard;

impl PersistOverrideGuard {
    fn on() -> Self {
        set_persist_override_for_tests(Some(true));
        Self
    }
}

impl Drop for PersistOverrideGuard {
    fn drop(&mut self) {
        set_persist_override_for_tests(None);
    }
}

struct ExternalRootGuard {
    _dir: TempDir,
}

impl ExternalRootGuard {
    fn new() -> Self {
        let tmp = TempDir::new().expect("tmpdir");
        set_external_root_override(Some(tmp.path().to_path_buf()));
        Self { _dir: tmp }
    }
}

impl Drop for ExternalRootGuard {
    fn drop(&mut self) {
        set_external_root_override(None);
    }
}

fn init_git_repo(dir: &std::path::Path, yaml: &str) -> std::path::PathBuf {
    let path = dir.join("bullseye.yaml");
    std::fs::write(&path, yaml).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q", "-b", "master"]);
    git(&["config", "user.email", "t100.4@example.com"]);
    git(&["config", "user.name", "T100.4"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["add", "bullseye.yaml"]);
    git(&["commit", "-q", "-m", "init"]);
    path
}

#[test]
fn external_shadow_ledger_never_uses_commit_mode_even_when_opted_in() {
    let _persist = PersistOverrideGuard::on();
    let _shadow = ExternalRootGuard::new();
    let repo = TempDir::new().expect("repo");
    let cwd = repo.path().join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let path = create_at(&cwd, Location::External, "shadow").expect("create external");
    assert!(is_external_ledger_path(&path));
    assert!(
        !persist_enabled_for(&path),
        "external shadow tree must stay file-mode"
    );

    with_locked_mutation(&path, |file| {
        file.targets.get_mut("T1").expect("starter T1").status = Status::Converging;
        Ok::<(), String>(())
    })
    .expect("mutation");

    let meta = std::fs::metadata(&path).expect("ledger file");
    assert!(meta.len() > 0);
}

#[test]
fn bare_commit_sha_integration_ref_is_refused() {
    let repo = TempDir::new().expect("repo");
    let yaml = "schema_version: 5\nproject: t\n\ntargets:\n  T1:\n    name: n\n    status: identified\n    value: 1\n    cost: 1\n    acceptance: []\n    context: c\n";
    let _path = init_git_repo(repo.path(), yaml);
    let tip = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git");
    let sha = String::from_utf8_lossy(&tip.stdout).trim().to_string();

    let err =
        check_commit_mode_integration_ref(repo.path(), &sha).expect_err("bare SHA must be refused");
    assert!(
        matches!(err, MutationGuard::DetachedIntegrationRef { .. }),
        "expected DetachedIntegrationRef, got {err:?}"
    );
}

#[test]
fn short_branch_name_integration_ref_is_refused() {
    let repo = TempDir::new().expect("repo");
    let yaml = "schema_version: 5\nproject: t\n\ntargets:\n  T1:\n    name: n\n    status: identified\n    value: 1\n    cost: 1\n    acceptance: []\n    context: c\n";
    let _path = init_git_repo(repo.path(), yaml);

    let err = check_commit_mode_integration_ref(repo.path(), "master")
        .expect_err("short branch name must be refused");
    assert!(
        matches!(err, MutationGuard::DetachedIntegrationRef { .. }),
        "expected DetachedIntegrationRef, got {err:?}"
    );
}

#[test]
fn default_integration_ref_name_is_full_ref() {
    let name = integration_ref_name();
    assert!(
        name.starts_with("refs/"),
        "default integration ref must be a full ref name, got {name}"
    );
}

#[test]
fn shadow_path_is_classified_external() {
    let _shadow = ExternalRootGuard::new();
    let cwd = std::path::Path::new("/tmp/example/shadow-classify");
    let root = bullseye::config::external_root();
    let shadow = shadow_path(&root, cwd).join("bullseye.yaml");
    assert!(is_external_ledger_path(&shadow));
}
