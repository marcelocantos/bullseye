// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

// Shared by every tests/core/* module: fixtures, mutation-test helpers,
// and the crate-wide names the original core_test.rs imported once at
// file scope. Re-exported here (not just `use`d) so a glob import
// (`use super::support::*;`) brings them into scope in every submodule
// exactly as they were in scope throughout the pre-split file.
pub use std::path::PathBuf;

pub use bullseye::graph;
pub use bullseye::schema::{RetryPolicy, Status, Strategy, TargetsFile};
pub use bullseye::store;

pub fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn load_fixture() -> TargetsFile {
    let path = fixture_path().join("bullseye.yaml");
    store::load(&path).unwrap()
}

pub fn write_project(tmp: &std::path::Path, makefile: &str, targets_yaml: &str) {
    use std::io::Write;
    write!(
        std::fs::File::create(tmp.join("bullseye.yaml")).unwrap(),
        "{targets_yaml}"
    )
    .unwrap();
    write!(
        std::fs::File::create(tmp.join("Makefile")).unwrap(),
        "{makefile}"
    )
    .unwrap();
}

/// Extract the concatenated text payload from an MCP `CallToolResult`,
/// panicking if any content block is not a `TextContent`. Used by the
/// handler-level end-to-end tests that drive `handle_convergence`
/// directly rather than calling `convergence::convergence`.
pub fn text_from_call_result(result: rust_mcp_sdk::schema::CallToolResult) -> String {
    use rust_mcp_sdk::schema::ContentBlock;
    result
        .content
        .into_iter()
        .map(|block| match block {
            ContentBlock::TextContent(t) => t.text,
            other => panic!("expected TextContent, got {other:?}"),
        })
        .collect::<Vec<_>>()
        .join("")
}

pub const SIMPLE_TARGETS_YAML: &str = r#"
schema_version: 5
targets:
  T1:
    name: Primary deliverable
    status: identified
    value: 8
    cost: 3
    acceptance:
      - Produces the primary artifact
      - Tests cover the happy path
    context: The highest-value thing in the project.
    discovered: 2026-04-01
  T1.v:
    name: Verify primary deliverable
    status: identified
    value: 1
    cost: 1
    acceptance:
      - T1 passes
    depends_on:
      - T1
    discovered: 2026-04-01
  T2:
    name: Secondary polish
    status: identified
    value: 3
    cost: 2
    acceptance:
      - Rough edges smoothed
    discovered: 2026-04-01
"#;

/// `make bullseye` that is green except for a dirty-tree check that
/// ignores `bullseye.yaml` at any in-repo path (🎯T73). `$$` is
/// Makefile escaping so the shell sees `$(git …)` and `'bullseye\.yaml$'`.
/// This fixture *fails* on non-ledger dirt — used to prove custom hooks
/// that still hard-fail continue to block. The recommended skeleton
/// warns instead (`DIRTY_WARN_MAKEFILE`).
pub const YAML_IGNORING_MAKEFILE: &str = r#"bullseye:
	@test -z "$$(git status --porcelain | grep -vE 'bullseye\.yaml$$')" && echo "✓ clean tree" || \
	 (echo "✗ dirty tree:"; git status --short | grep -vE 'bullseye\.yaml$$'; exit 1)
"#;

/// Recommended dirty-tree policy: loud warning, exit 0. Agents decide
/// whether to park leftover WIP or continue on this session's work.
pub const DIRTY_WARN_MAKEFILE: &str = r#"bullseye:
	@dirty=$$(git status --porcelain | grep -vE 'bullseye\.yaml$$' || true); \
	if [ -z "$$dirty" ]; then echo "✓ working tree clean"; \
	else \
	  echo "⚠  DIRTY WORKING TREE"; \
	  echo "Warning only — invariants still pass (exit 0)."; \
	  echo "$$dirty"; \
	fi
"#;

/// Set up `tmp` as a git repo whose `pre-commit` hook blocks forever,
/// with `bullseye.yaml` dirty — the shape that hung convergence in
/// 🎯T62 when step 0 auto-committed. 🎯T73 removed that step; the
/// helper still dirties the ledger so tests can prove yaml dirt does
/// not invoke `git commit`.
#[cfg(unix)]
pub fn write_project_with_blocking_pre_commit_hook(tmp: &std::path::Path, makefile: &str) {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(tmp)
            .args(args)
            .output()
            .expect("git invocation failed");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    write_project(tmp, makefile, SIMPLE_TARGETS_YAML);
    git(&["init", "-q", "-b", "master"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "init"]);

    let hooks = tmp.join(".git/blocking-hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexec sleep 3600\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);

    // Dirty the ledger so yaml dirt is present (🎯T73: must not block).
    use std::io::Write;
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(tmp.join("bullseye.yaml"))
            .unwrap(),
        "# touched"
    )
    .unwrap();
}

/// Initialise a git repo at `dir` with a non-ledger commit so HEAD is
/// not a yaml-only commit (🎯T73 fixtures).
pub fn t73_git_repo_with_readme(dir: &std::path::Path) {
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git invocation failed");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q", "-b", "master"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "commit.gpgsign", "false"]);
    let empty = dir.join(".git/empty-hooks");
    std::fs::create_dir_all(&empty).unwrap();
    git(&["config", "core.hooksPath", empty.to_str().unwrap()]);
    std::fs::write(dir.join("README.md"), "# project\n").unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "-q", "-m", "init"]);
}

pub fn t73_commit_count(dir: &std::path::Path) -> usize {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap_or(0)
}

pub fn t73_git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Write a minimal valid bullseye.yaml to a path.
pub fn write_yaml(path: &std::path::Path, target_name: &str) {
    use std::io::Write;
    write!(
        std::fs::File::create(path).unwrap(),
        "schema_version: 1\ntargets:\n  T1:\n    name: {target_name}\n    \
         status: identified\n    value: 3\n    cost: 2\n    acceptance:\n      \
         - done\n    discovered: 2026-04-15\n"
    )
    .unwrap();
}

/// Run `git -C <dir> <args>` and panic on failure with captured stderr.
/// Used by the 🎯T24 integration tests to set up parent + submodule
/// repos and to flip HEAD into a detached state. Identity / hooks
/// config is set locally so commits work in CI without a global
/// gitconfig.
pub fn t24_run_git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git invocation failed");
    assert!(
        out.status.success(),
        "git {args:?} failed in {dir:?}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Initialise a git repo at `dir` with stable identity + an empty
/// hooks dir so the developer's global pre-commit hooks don't fire.
pub fn t24_git_init(dir: &std::path::Path) {
    t24_run_git(dir, &["init", "-q", "-b", "master"]);
    t24_run_git(dir, &["config", "user.email", "test@example.com"]);
    t24_run_git(dir, &["config", "user.name", "Test"]);
    t24_run_git(dir, &["config", "commit.gpgsign", "false"]);
    let empty = dir.join(".git/empty-hooks");
    std::fs::create_dir_all(&empty).unwrap();
    t24_run_git(dir, &["config", "core.hooksPath", empty.to_str().unwrap()]);
}

/// Set the standard env vars `git commit` requires when no system
/// gitconfig is available (CI). The tests' per-repo `user.name` /
/// `user.email` config is enough on most platforms, but
/// `git submodule add` runs a sub-command in the child working tree
/// before our config takes effect — these env vars carry through.
pub fn t24_set_git_env() {
    // Safety: tests run sequentially within one process here, but
    // `cargo test` parallelises across tests by default. We set these
    // env vars defensively even though per-repo `user.*` config is
    // also populated; they're idempotent and process-local.
    unsafe {
        std::env::set_var("GIT_AUTHOR_NAME", "Test");
        std::env::set_var("GIT_AUTHOR_EMAIL", "test@example.com");
        std::env::set_var("GIT_COMMITTER_NAME", "Test");
        std::env::set_var("GIT_COMMITTER_EMAIL", "test@example.com");
    }
}

/// Commit count on `HEAD` for `repo`, or 0 if `HEAD` is unborn.
pub fn t24_commit_count(repo: &std::path::Path) -> usize {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap_or(0)
}

pub const T24_FIXTURE_YAML: &str = r#"schema_version: 1
targets:
  T1:
    name: Example target
    status: identified
    value: 1
    cost: 1
    acceptance:
      - it works
    origin: manual
    discovered: 2026-01-01
"#;

/// Stand up a fresh tempdir with three targets in a chain so each
/// subdivide test starts from the same shape:
///
///   T1 (identified, no deps)
///   T2 (identified, depends_on: [T1])
///   T3 (identified, depends_on: [T1])
///
/// T1 has two dependents. Subdivision against T1 in any mode is
/// observable as a change in how T2/T3 wire to whatever replaces T1.
pub fn subdivide_fixture() -> (tempfile::TempDir, tempfile::TempDir, String) {
    use bullseye::config::{self, Location};

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "subdivide-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    // The starter file already seeded T1; add T2 and T3 depending on T1.
    let mut file = store::load(&path).unwrap();
    let today = chrono::Local::now().date_naive();
    for id in ["T2", "T3"] {
        file.targets.insert(
            id.to_string(),
            bullseye::schema::Target {
                name: format!("Dependent {id}"),
                status: bullseye::schema::Status::Identified,
                value: 0.0,
                cost: 0.0,
                actual_cost: None,
                attestation: None,
                set_aside_reason: None,
                acceptance: vec!["done".to_string()],
                checks: vec![],
                context: String::new(),
                gates: vec![],
                depends_on: vec!["T1".to_string()],
                cross_depends: vec![],
                cross_enables: vec![],
                tags: vec![],
                strategy: None,
                origin: "test".to_string(),
                discovered: today,
                achieved: None,
                owned_by: None,
                postponed_until: None,
                postpone_predicate: None,
            },
        );
    }
    store::save(&path, &file).unwrap();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    (tmp, shadow_tmp, cwd)
}

pub fn child_spec(name: &str, acceptance: &[&str]) -> bullseye::tools::SubdivisionChild {
    bullseye::tools::SubdivisionChild {
        id: None,
        name: name.to_string(),
        acceptance: acceptance.iter().map(|s| s.to_string()).collect(),
        context: None,
        tags: None,
        depends_on: None,
    }
}

/// Serialise the 🎯T28 id_alloc tests. They share the process-global
/// `id_alloc::CACHE`, so running them in parallel produces flaky
/// races where one test's `clear_cache_for_tests()` invalidates
/// another's expected memoisation. Each test acquires this mutex at
/// entry and holds it for the whole body; the harness still
/// parallelises against unrelated tests.
pub static T28_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn t28_lock() -> std::sync::MutexGuard<'static, ()> {
    T28_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Initialise a git repo with stable identity so commits don't depend
/// on the developer's git config or wall clock for these tests.
pub fn t28_git_init(dir: &std::path::Path) {
    use std::process::Command;
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git invocation failed");
        if !out.status.success() {
            panic!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    };
    run(&["init", "-q", "-b", "master"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    let empty = dir.join(".git/empty-hooks");
    std::fs::create_dir_all(&empty).unwrap();
    run(&["config", "core.hooksPath", empty.to_str().unwrap()]);
}

pub fn t28_git(dir: &std::path::Path, args: &[&str]) {
    use std::process::Command;
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git invocation failed");
    if !out.status.success() {
        panic!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Set up a git repo with `bullseye.yaml` already committed on master
/// and a branch `feature` that adds an extra target. Returns the path
/// to the yaml on master (which does NOT contain `feature`'s target).
/// Clears the id_alloc cache so each test sees a fresh scan.
pub fn t28_repo_with_branched_id() -> tempfile::TempDir {
    use bullseye::config::{self, Location};
    bullseye::id_alloc::clear_cache_for_tests();

    let tmp = tempfile::tempdir().unwrap();
    t28_git_init(tmp.path());

    // Seed: starter file (T1 only). The store::create_at helper writes
    // a fresh bullseye.yaml with a `T1` target.
    let path = store::create_at(tmp.path(), Location::InRepo, "t28-test").unwrap();
    let _ = config::set_external_root_override; // reference to keep import
    t28_git(tmp.path(), &["add", "bullseye.yaml"]);
    t28_git(tmp.path(), &["commit", "-q", "-m", "init bullseye.yaml"]);

    // Branch off, add T2 and T3, commit, switch back. Master's
    // bullseye.yaml ends with just T1 on disk; T2 and T3 only live on
    // the feature branch's commit history.
    t28_git(tmp.path(), &["checkout", "-q", "-b", "feature"]);
    {
        let mut file = store::load(&path).unwrap();
        let today = chrono::Local::now().date_naive();
        for id in ["T2", "T3"] {
            file.targets.insert(
                id.to_string(),
                bullseye::schema::Target {
                    name: format!("Feature-branch target {id}"),
                    status: bullseye::schema::Status::Identified,
                    value: 0.0,
                    cost: 0.0,
                    actual_cost: None,
                    attestation: None,
                    set_aside_reason: None,
                    acceptance: vec!["done".to_string()],
                    checks: vec![],
                    context: String::new(),
                    gates: vec![],
                    depends_on: vec![],
                    cross_depends: vec![],
                    cross_enables: vec![],
                    tags: vec![],
                    strategy: None,
                    origin: "test".to_string(),
                    discovered: today,
                    achieved: None,
                    owned_by: None,
                    postponed_until: None,
                    postpone_predicate: None,
                },
            );
        }
        store::save(&path, &file).unwrap();
    }
    t28_git(tmp.path(), &["add", "bullseye.yaml"]);
    t28_git(
        tmp.path(),
        &["commit", "-q", "-m", "feature: add T2 and T3"],
    );
    t28_git(tmp.path(), &["checkout", "-q", "master"]);

    bullseye::id_alloc::clear_cache_for_tests();
    tmp
}

/// Build a fake workspace under `root` with the given repo paths. Each
/// path becomes `<root>/<path>/bullseye.yaml`. Used to exercise the
/// resolver without touching `~/work/`.
pub fn t29_workspace(root: &std::path::Path, repos: &[&str]) {
    for repo in repos {
        let dir = root.join(repo);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("bullseye.yaml"),
            "schema_version: 5\ntargets: {}\n",
        )
        .unwrap();
    }
}

/// A `CommitTool` with every optional field cleared, so each test sets
/// only what it exercises.
pub fn t64_commit(cwd: &str, op: &str) -> bullseye::tools::CommitTool {
    bullseye::tools::CommitTool {
        cwd: cwd.to_string(),
        op: op.to_string(),
        id: None,
        child_of: None,
        name: None,
        value: None,
        cost: None,
        acceptance: None,
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
        actual_cost: None,
        attestation: None,
        reason: None,
        postponed_until: None,
        postpone_predicate: None,
        parent: None,
        mode: None,
        children: None,
        retire_reason: None,
        tail: None,
        owner: None,
    }
}

/// Create a temp repo with a starter `bullseye.yaml` (target `T1`) and
/// an isolated external shadow root. Returns the tempdirs (which must
/// outlive the test body), the file path, and the cwd string.
pub fn t64_repo(label: &str) -> (tempfile::TempDir, tempfile::TempDir, PathBuf, String) {
    use bullseye::config::{self, Location};
    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, label).unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow.path().to_path_buf()));
    (tmp, shadow, path, cwd)
}

pub const T64_BRICKED_YAML: &str = r#"
schema_version: 5
targets:
  T1:
    name: Healthy target
    status: identified
    value: 0
    cost: 0
    acceptance:
      - It works
    discovered: 2026-08-01
  T2:
    name: Target with illegal residue
    status: achieved
    value: 0
    cost: 0
    acceptance:
      - It worked
    set_aside_reason: duplicate of T288
    attestation: shipped in 1.2.3
    discovered: 2026-08-01
    achieved: 2026-08-09
"#;

pub const T64_ONE_INVALID_YAML: &str = r#"
schema_version: 5
targets:
  T1:
    name: Healthy and ready
    status: converging
    value: 0
    cost: 0
    acceptance:
      - It works
    discovered: 2026-08-01
  T2:
    name: Dangling dependency
    status: converging
    value: 0
    cost: 0
    acceptance:
      - It works too
    depends_on:
      - T99
    discovered: 2026-08-01
"#;

/// RAII helper: isolate the external shadow root to a tempdir so the
/// tests don't touch the developer's real `~/.local/share/bullseye`,
/// and cleanly restore on drop.
pub struct ShadowFixture {
    _tmp: tempfile::TempDir,
}

impl ShadowFixture {
    pub fn with_root(root: &std::path::Path) -> Self {
        bullseye::config::set_external_root_override(Some(root.to_path_buf()));
        // Caller owns the tempdir; this holder just flips the override back.
        ShadowFixture {
            _tmp: tempfile::tempdir().unwrap(),
        }
    }
}

impl Drop for ShadowFixture {
    fn drop(&mut self) {
        bullseye::config::set_external_root_override(None);
    }
}
