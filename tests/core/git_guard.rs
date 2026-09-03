// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

/// Mutating `bullseye_put` from inside a submodule replica must be
/// refused with a clear error naming the superproject path. The
/// submodule worktree must remain at its original commit count — the
/// write is refused before any mutation.
#[test]
fn t24_mutation_refused_in_submodule_replica() {
    use bullseye::config;
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    t24_set_git_env();

    let scratch = tempfile::tempdir().unwrap();
    let parent = scratch.path().join("parent");
    let inner = scratch.path().join("inner-source");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::create_dir_all(&inner).unwrap();

    // Build a self-contained "inner" repo to serve as the submodule
    // source. Carries a bullseye.yaml so `discover_anywhere` finds it
    // inside the submodule path.
    t24_git_init(&inner);
    std::fs::write(inner.join("bullseye.yaml"), T24_FIXTURE_YAML).unwrap();
    t24_run_git(&inner, &["add", "bullseye.yaml"]);
    t24_run_git(&inner, &["commit", "-q", "-m", "init inner"]);

    // Parent repo with the inner repo nested as a submodule. Modern
    // git refuses `submodule add` on file:// URLs by default; allow it.
    t24_git_init(&parent);
    std::fs::write(parent.join("README.md"), "parent\n").unwrap();
    t24_run_git(&parent, &["add", "README.md"]);
    t24_run_git(&parent, &["commit", "-q", "-m", "init parent"]);
    let inner_url = format!("file://{}", inner.display());
    t24_run_git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &inner_url,
            "sub",
        ],
    );
    t24_run_git(&parent, &["commit", "-q", "-m", "add submodule"]);

    let submodule = parent.join("sub");
    assert!(
        submodule.join("bullseye.yaml").exists(),
        "submodule should carry bullseye.yaml after add",
    );

    // Isolate the external shadow root so discover_anywhere can't pick
    // up a file in the developer's real ~/.local/share/bullseye.
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            bullseye::config::set_external_root_override(None);
        }
    }
    let _cleanup = Cleanup;

    let commits_before = t24_commit_count(&submodule);

    let result = handle_put(PutTool {
        reason: None,
        cwd: submodule.to_string_lossy().to_string(),
        id: Some("T1".to_string()),
        child_of: None,
        name: Some("Renamed via submodule".to_string()),
        value: None,
        cost: None,
        acceptance: None,
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });

    let err = result.expect_err("mutation in submodule must be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("submodule"),
        "error should mention submodule: {msg}",
    );
    assert!(
        msg.contains(&parent.display().to_string()),
        "error should name the superproject path {parent:?}; got: {msg}",
    );
    assert!(
        msg.contains(&submodule.display().to_string()),
        "error should name the submodule cwd {submodule:?}; got: {msg}",
    );

    // Commit count in the submodule worktree is unchanged — the
    // mutation was refused before any write.
    assert_eq!(
        t24_commit_count(&submodule),
        commits_before,
        "no commit should land in the submodule when the mutation is refused",
    );

    // The bullseye.yaml inside the submodule is byte-identical to the
    // fixture: the refusal happened before any read-modify-write.
    let after = std::fs::read_to_string(submodule.join("bullseye.yaml")).unwrap();
    assert_eq!(
        after, T24_FIXTURE_YAML,
        "submodule's bullseye.yaml must be untouched after a refused mutation",
    );
}

/// Mutating `bullseye_put` against a repo with detached HEAD must be
/// refused with a clear error naming the detached state. A write on a
/// checkout with no branch would otherwise lose the work.
#[test]
fn t24_mutation_refused_in_detached_head() {
    use bullseye::config;
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    t24_set_git_env();

    let scratch = tempfile::tempdir().unwrap();
    let repo = scratch.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    t24_git_init(&repo);
    std::fs::write(repo.join("bullseye.yaml"), T24_FIXTURE_YAML).unwrap();
    t24_run_git(&repo, &["add", "bullseye.yaml"]);
    t24_run_git(&repo, &["commit", "-q", "-m", "init"]);
    // Detach HEAD by checking out the SHA directly.
    t24_run_git(&repo, &["checkout", "-q", "--detach", "HEAD"]);

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            bullseye::config::set_external_root_override(None);
        }
    }
    let _cleanup = Cleanup;

    let commits_before = t24_commit_count(&repo);

    let result = handle_put(PutTool {
        reason: None,
        cwd: repo.to_string_lossy().to_string(),
        id: Some("T1".to_string()),
        child_of: None,
        name: Some("Renamed via detached HEAD".to_string()),
        value: None,
        cost: None,
        acceptance: None,
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });

    let err = result.expect_err("mutation under detached HEAD must be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("detached"),
        "error should mention detached HEAD: {msg}",
    );
    assert!(
        msg.contains(&repo.display().to_string()),
        "error should name the repo path {repo:?}; got: {msg}",
    );
    assert!(
        msg.contains("checkout") || msg.contains("switch"),
        "error should suggest a branch checkout: {msg}",
    );

    assert_eq!(
        t24_commit_count(&repo),
        commits_before,
        "no commit should land when detached HEAD blocks the mutation",
    );
    let after = std::fs::read_to_string(repo.join("bullseye.yaml")).unwrap();
    assert_eq!(after, T24_FIXTURE_YAML, "yaml must be untouched");
}

/// Read-only operations (here: `bullseye_list`) must succeed even
/// inside a submodule replica — the 🎯T24 guard fires only on
/// mutating handlers, so research from inside a parent project's
/// submodule still works.
#[test]
fn t24_read_only_ops_unaffected_in_submodule() {
    use bullseye::config;
    use bullseye::handler::handle_list;
    use bullseye::tools::ListTool;

    t24_set_git_env();

    let scratch = tempfile::tempdir().unwrap();
    let parent = scratch.path().join("parent");
    let inner = scratch.path().join("inner-source");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::create_dir_all(&inner).unwrap();

    t24_git_init(&inner);
    std::fs::write(inner.join("bullseye.yaml"), T24_FIXTURE_YAML).unwrap();
    t24_run_git(&inner, &["add", "bullseye.yaml"]);
    t24_run_git(&inner, &["commit", "-q", "-m", "init inner"]);

    t24_git_init(&parent);
    std::fs::write(parent.join("README.md"), "parent\n").unwrap();
    t24_run_git(&parent, &["add", "README.md"]);
    t24_run_git(&parent, &["commit", "-q", "-m", "init parent"]);
    let inner_url = format!("file://{}", inner.display());
    t24_run_git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &inner_url,
            "sub",
        ],
    );
    t24_run_git(&parent, &["commit", "-q", "-m", "add submodule"]);

    let submodule = parent.join("sub");
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            bullseye::config::set_external_root_override(None);
        }
    }
    let _cleanup = Cleanup;

    // bullseye_list is read-only; it must succeed despite the cwd
    // sitting inside a submodule worktree.
    handle_list(ListTool {
        cwd: submodule.to_string_lossy().to_string(),
        filter: "active".to_string(),
    })
    .expect("read-only list should succeed inside a submodule");
}

// --- bullseye_subdivide (🎯T27.1) -----------------------------------------
