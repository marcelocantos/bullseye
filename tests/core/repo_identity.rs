// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! External ledgers keyed by the remote's identity (🎯T113).

use super::support::*;
use bullseye::config::Location;
use bullseye::store;
use std::path::{Path, PathBuf};

const REMOTE: &str = "git@github.com:Acme/Widget.git";

/// A clone with one commit (worktrees need a branch tip) and `origin`.
fn clone_with_origin(dir: &Path, url: &str) {
    t73_git_repo_with_readme(dir);
    t73_git(dir, &["remote", "add", "origin", url]);
}

fn keyed(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.join("by-remote/github.com/acme/widget");
    if !rel.is_empty() {
        p.push(rel);
    }
    p.join("bullseye.yaml")
}

#[test]
fn t113_external_create_and_discover_use_remote_key() {
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    let clone = tempfile::tempdir().unwrap();
    clone_with_origin(clone.path(), REMOTE);

    let path = store::create_at(clone.path(), Location::External, "widget").unwrap();
    assert_eq!(path, keyed(shadow.path(), ""));
    assert_eq!(store::discover_anywhere(clone.path()), Some(path.clone()));

    // A subdirectory without its own ledger finds the repo's.
    let deep = clone.path().join("src/inner");
    std::fs::create_dir_all(&deep).unwrap();
    assert_eq!(store::discover_anywhere(&deep), Some(path));
}

#[test]
fn t113_subproject_ledger_keeps_its_prefix() {
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    let clone = tempfile::tempdir().unwrap();
    clone_with_origin(clone.path(), REMOTE);
    let sub = clone.path().join("tools/cli");
    std::fs::create_dir_all(&sub).unwrap();

    let path = store::create_at(&sub, Location::External, "cli").unwrap();
    assert_eq!(path, keyed(shadow.path(), "tools/cli"));
    assert_eq!(store::discover_anywhere(&sub), Some(path));
    assert_eq!(
        store::discover_anywhere(clone.path()),
        None,
        "the repo root must not see a subproject's ledger"
    );
}

#[test]
fn t113_worktree_and_second_clone_share_the_ledger() {
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    let clone = tempfile::tempdir().unwrap();
    clone_with_origin(clone.path(), REMOTE);
    let path = store::create_at(clone.path(), Location::External, "widget").unwrap();

    let elsewhere = tempfile::tempdir().unwrap();
    let wt = elsewhere.path().join("wt");
    t73_git(
        clone.path(),
        &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
    );
    assert_eq!(store::discover_anywhere(&wt), Some(path.clone()));

    // Same repo over https, different case: same identity.
    let second = tempfile::tempdir().unwrap();
    clone_with_origin(second.path(), "https://github.com/acme/widget");
    assert_eq!(store::discover_anywhere(second.path()), Some(path));
}

#[test]
fn t113_fork_gets_its_own_ledger() {
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    let clone = tempfile::tempdir().unwrap();
    clone_with_origin(clone.path(), REMOTE);
    store::create_at(clone.path(), Location::External, "widget").unwrap();

    let fork = tempfile::tempdir().unwrap();
    clone_with_origin(fork.path(), "git@github.com:someone/widget.git");
    assert_eq!(store::discover_anywhere(fork.path()), None);
}

#[test]
fn t113_no_origin_falls_back_to_path_key() {
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    let repo = tempfile::tempdir().unwrap();
    t73_git_repo_with_readme(repo.path());

    let path = store::create_at(repo.path(), Location::External, "local").unwrap();
    assert_eq!(
        path,
        store::shadow_path(shadow.path(), repo.path()).join("bullseye.yaml")
    );
    assert_eq!(store::discover_anywhere(repo.path()), Some(path));
}

#[test]
fn t113_path_keyed_ledger_moves_to_remote_key() {
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    let clone = tempfile::tempdir().unwrap();
    clone_with_origin(clone.path(), REMOTE);

    let legacy_dir = store::shadow_path(shadow.path(), clone.path());
    std::fs::create_dir_all(&legacy_dir).unwrap();
    let legacy = legacy_dir.join("bullseye.yaml");
    write_yaml(&legacy, "LEGACY_CONTENT");

    let found = store::discover_anywhere(clone.path()).expect("legacy ledger must be found");
    assert_eq!(found, keyed(shadow.path(), ""));
    assert!(
        std::fs::read_to_string(&found)
            .unwrap()
            .contains("LEGACY_CONTENT"),
        "content must survive the move"
    );
    assert!(!legacy.exists(), "old copy must be gone");
    assert!(
        !legacy_dir.exists(),
        "empty path-keyed directories must be pruned"
    );
    let file = store::load(&found).expect("moved ledger loads");
    assert_eq!(file.targets["T1"].name, "LEGACY_CONTENT");

    // A worktree now reaches it too.
    let elsewhere = tempfile::tempdir().unwrap();
    let wt = elsewhere.path().join("wt");
    t73_git(
        clone.path(),
        &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
    );
    assert_eq!(store::discover_anywhere(&wt), Some(found));
}

#[test]
fn t113_multi_repo_path_ledger_stays_in_place() {
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    let workspace = tempfile::tempdir().unwrap();
    let clone = workspace.path().join("widget");
    std::fs::create_dir_all(&clone).unwrap();
    clone_with_origin(&clone, REMOTE);

    // A ledger for the whole workspace, above the repo's root.
    let legacy_dir = store::shadow_path(shadow.path(), workspace.path());
    std::fs::create_dir_all(&legacy_dir).unwrap();
    let legacy = legacy_dir.join("bullseye.yaml");
    write_yaml(&legacy, "WORKSPACE");

    assert_eq!(store::discover_anywhere(&clone), Some(legacy.clone()));
    assert!(legacy.is_file(), "a multi-repo ledger must not move");
    assert!(!keyed(shadow.path(), "").exists());
}

#[test]
fn t113_remote_keyed_wins_over_path_keyed() {
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    let clone = tempfile::tempdir().unwrap();
    clone_with_origin(clone.path(), REMOTE);

    let current = keyed(shadow.path(), "");
    std::fs::create_dir_all(current.parent().unwrap()).unwrap();
    write_yaml(&current, "CURRENT");
    let legacy_dir = store::shadow_path(shadow.path(), clone.path());
    std::fs::create_dir_all(&legacy_dir).unwrap();
    let legacy = legacy_dir.join("bullseye.yaml");
    write_yaml(&legacy, "STALE");

    assert_eq!(
        store::discover_anywhere(clone.path()),
        Some(current.clone())
    );
    assert!(
        std::fs::read_to_string(&current)
            .unwrap()
            .contains("CURRENT")
    );
    assert!(legacy.is_file(), "the stale copy is left for a human");
}

#[test]
fn t113_in_repo_still_wins() {
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());
    let clone = tempfile::tempdir().unwrap();
    clone_with_origin(clone.path(), REMOTE);
    store::create_at(clone.path(), Location::External, "ext").unwrap();
    let in_repo = store::create_at(clone.path(), Location::InRepo, "in").unwrap();

    assert_eq!(store::discover_anywhere(clone.path()), Some(in_repo));
}
