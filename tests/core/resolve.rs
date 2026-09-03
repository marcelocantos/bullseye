// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn resolve_leaf_name_returns_single_repo() {
    use bullseye::resolve;
    let tmp = tempfile::tempdir().unwrap();
    t29_workspace(
        tmp.path(),
        &[
            "github.com/marcelocantos/spyder",
            "github.com/marcelocantos/bullseye",
        ],
    );
    resolve::clear_cache_for_tests();

    let got = resolve::resolve(tmp.path(), "spyder").expect("leaf name should resolve");
    assert_eq!(got, tmp.path().join("github.com/marcelocantos/spyder"));
}

#[test]
fn resolve_partial_path_matches_more_specifically() {
    use bullseye::resolve;
    let tmp = tempfile::tempdir().unwrap();
    t29_workspace(
        tmp.path(),
        &[
            "github.com/marcelocantos/spyder",
            "github.com/otheruser/spyder",
        ],
    );
    resolve::clear_cache_for_tests();

    // Leaf `spyder` matches both repos → ambiguous.
    let err = resolve::resolve(tmp.path(), "spyder").expect_err("leaf name is ambiguous here");
    match err {
        resolve::ResolveError::Ambiguous { candidates, .. } => {
            assert_eq!(candidates.len(), 2);
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }

    // `marcelocantos/spyder` narrows to one.
    let got = resolve::resolve(tmp.path(), "marcelocantos/spyder").expect("partial path resolves");
    assert_eq!(got, tmp.path().join("github.com/marcelocantos/spyder"));
}

#[test]
fn resolve_not_found_names_workspace_root() {
    use bullseye::resolve;
    let tmp = tempfile::tempdir().unwrap();
    t29_workspace(tmp.path(), &["github.com/marcelocantos/bullseye"]);
    resolve::clear_cache_for_tests();

    let err = resolve::resolve(tmp.path(), "nonexistent").expect_err("should not find");
    match err {
        resolve::ResolveError::NotFound {
            reference,
            workspace_root,
        } => {
            assert_eq!(reference, "nonexistent");
            assert_eq!(workspace_root, tmp.path());
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn resolve_absolute_path_passes_through() {
    use bullseye::resolve;
    let tmp = tempfile::tempdir().unwrap();
    t29_workspace(tmp.path(), &["github.com/marcelocantos/bullseye"]);
    let abs = tmp.path().join("github.com/marcelocantos/bullseye");
    resolve::clear_cache_for_tests();

    let got = resolve::resolve(tmp.path(), abs.to_str().unwrap())
        .expect("absolute path should pass through");
    assert_eq!(got, abs);
}

#[test]
fn resolve_absolute_path_without_bullseye_yaml_errors() {
    use bullseye::resolve;
    let tmp = tempfile::tempdir().unwrap();
    let dangling = tmp.path().join("not-a-repo");
    std::fs::create_dir_all(&dangling).unwrap();
    resolve::clear_cache_for_tests();

    let err = resolve::resolve(tmp.path(), dangling.to_str().unwrap())
        .expect_err("absolute path without bullseye.yaml must error");
    assert!(matches!(
        err,
        resolve::ResolveError::AbsoluteNotFound { .. }
    ));
}

#[test]
fn resolve_skips_hidden_and_vendor_dirs() {
    use bullseye::resolve;
    let tmp = tempfile::tempdir().unwrap();
    t29_workspace(
        tmp.path(),
        &[
            // These should be skipped — under hidden / vendor / target / node_modules.
            ".cache/buried",
            "vendor/buried",
            "target/buried",
            "node_modules/buried",
            // This should be found.
            "github.com/marcelocantos/buried",
        ],
    );
    resolve::clear_cache_for_tests();

    let got = resolve::resolve(tmp.path(), "buried").expect("only the non-skipped buried matches");
    assert_eq!(got, tmp.path().join("github.com/marcelocantos/buried"));
}
