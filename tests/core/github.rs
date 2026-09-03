// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

/// The github-sync MCP handler inverts `pull_only` / `push_only` into the
/// `pull` / `push` enables (default: both directions on).
#[test]
fn github_sync_handler_inverts_pull_push_flags() {
    use bullseye::handler::github_args_for;
    use bullseye::tools::GithubSyncTool;

    let tool = |pull_only: bool, push_only: bool| GithubSyncTool {
        cwd: "/tmp/x".to_string(),
        repo: Some("o/r".to_string()),
        label: None,
        assignee: None,
        pull_only,
        push_only,
        dry_run: false,
    };

    let both = github_args_for(&tool(false, false));
    assert!(both.pull && both.push, "default: both directions on");
    assert_eq!(both.repo.as_deref(), Some("o/r"));

    let pull_only = github_args_for(&tool(true, false));
    assert!(pull_only.pull && !pull_only.push);

    let push_only = github_args_for(&tool(false, true));
    assert!(!push_only.pull && push_only.push);
}

/// The sync-priorities MCP handler runs the same scan + upsert as the CLI
/// subcommand, writing the portfolio frontier into the SQLite table.
#[cfg(feature = "sqlite")]
#[test]
fn sync_priorities_handler_writes_frontier() {
    use bullseye::handler::handle_sync_priorities;
    use bullseye::tools::SyncPrioritiesTool;

    let root = tempfile::TempDir::new().unwrap();
    // A repo under the workspace root with one unblocked (frontier) target.
    let repo = root.path().join("acme");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("bullseye.yaml"),
        "schema_version: 5\ntargets:\n  T1:\n    name: ship it\n    status: identified\n    \
         value: 0\n    cost: 0\n    acceptance: [done]\n    discovered: 2026-06-21\n",
    )
    .unwrap();
    let db = root.path().join("priorities.db");

    let result = handle_sync_priorities(SyncPrioritiesTool {
        db: Some(db.to_string_lossy().into_owned()),
        root: Some(root.path().to_string_lossy().into_owned()),
        horizon: "today".to_string(),
        max_depth: 5,
    })
    .expect("sync-priorities handler should succeed");

    let out = text_from_call_result(result);
    assert!(
        out.contains("upserted"),
        "handler should report a priorities sync; got: {out}"
    );
    assert!(
        db.exists(),
        "the priorities SQLite db should have been created at {}",
        db.display()
    );
}

// --- 🎯T45 core API surface ------------------------------------------------
