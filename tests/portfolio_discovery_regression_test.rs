// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

#![cfg(unix)]

use bullseye::portfolio::{discover_repos, format_portfolio};
use std::os::unix::fs::symlink;

#[test]
fn recursive_portfolio_does_not_follow_external_symlinks() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let local = workspace.path().join("local");
    std::fs::create_dir(&local).unwrap();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bullseye.yaml");
    std::fs::copy(&fixture, local.join("bullseye.yaml")).unwrap();
    std::fs::copy(&fixture, outside.path().join("bullseye.yaml")).unwrap();
    let link = workspace.path().join("cloud-repo");
    symlink(outside.path(), &link).unwrap();

    let scan = discover_repos(workspace.path(), 5, &[]);
    assert_eq!(
        scan.repos.len(),
        1,
        "recursive discovery must stay off linked trees"
    );
    assert_eq!(scan.repos[0].path, local);
    assert!(scan.warnings.iter().any(|w| w.path == link));
    let output = format_portfolio(&scan);
    assert!(output.contains("portfolio coverage is partial"));
    assert!(output.contains(link.to_str().unwrap()));

    // An explicitly selected root is still an intentional scan.
    let explicit = discover_repos(&link, 5, &[]);
    assert_eq!(explicit.repos.len(), 1);
    assert_eq!(explicit.repos[0].path, link);
}

#[test]
fn recursive_portfolio_does_not_expand_symlink_cycles() {
    let workspace = tempfile::tempdir().unwrap();
    let local = workspace.path().join("local");
    std::fs::create_dir(&local).unwrap();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bullseye.yaml");
    std::fs::copy(&fixture, local.join("bullseye.yaml")).unwrap();
    symlink(workspace.path(), workspace.path().join("loop")).unwrap();
    let scan = discover_repos(workspace.path(), 5, &[]);
    assert_eq!(
        scan.repos.len(),
        1,
        "a cycle must not multiply ledger contributions"
    );
    assert_eq!(scan.repos[0].path, local);
    assert_eq!(scan.warnings.len(), 1);
}
