// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Git-backed ledger writes for commit mode (🎯T100.1).
//!
//! Stages `bullseye.yaml` bytes in a private index, builds a commit on top
//! of the current ref tip, and moves the ref with an expected-old CAS.
//! A moved ref surfaces as [`WriteError::Conflict`] for retry — never as a
//! silent clobber or false success.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bounded::{
    GIT_QUERY_TIMEOUT, GitQueryError, git_hash_object, git_query_detailed, git_run_detailed,
};

/// Default integration ref for commit mode (local master per 🎯T104).
pub const DEFAULT_REF: &str = "refs/heads/master";

/// Opt-in env var (🎯T100 / 🎯T100.4). When truthy, ledger persists via ref CAS.
pub const COMMIT_ENV: &str = "BULLSEYE_COMMIT";

/// Integration ref override (default [`DEFAULT_REF`]).
pub const INTEGRATION_REF_ENV: &str = "BULLSEYE_INTEGRATION_REF";

/// Matches handler `apply_request` conflict retries (🎯T100.2).
pub const PERSIST_CONFLICT_ATTEMPTS: usize = 8;

/// Ledger path inside the git tree.
pub const LEDGER_PATH: &str = "bullseye.yaml";

// Per-thread test override so parallel `cargo test` workers do not fight over
// `BULLSEYE_COMMIT` (🎯T100.2).
thread_local! {
    static PERSIST_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Process-wide opt-in from `bullseye serve --commit-mode` (🎯T100.4).
/// Env [`COMMIT_ENV`] is the default source; the flag forces commit mode on
/// for the whole server process.
static PROCESS_COMMIT_MODE: OnceLock<bool> = OnceLock::new();

/// Record commit mode from CLI `--commit-mode` at process startup (idempotent).
pub fn set_process_commit_mode(enabled: bool) {
    let _ = PROCESS_COMMIT_MODE.set(enabled);
}

/// Whether commit-mode persist is active for this process.
pub fn persist_enabled() -> bool {
    if let Some(v) = PERSIST_OVERRIDE.get() {
        return v;
    }
    if PROCESS_COMMIT_MODE.get() == Some(&true) {
        return true;
    }
    env_truthy(std::env::var(COMMIT_ENV).ok().as_deref())
}

/// One-line summary for `serve` startup logging (🎯T100.4).
pub fn startup_status_line() -> String {
    if !persist_enabled() {
        return "commit mode: off (set BULLSEYE_COMMIT=1 or --commit-mode to enable)".to_string();
    }
    format!(
        "commit mode: on (integration ref {})",
        integration_ref_name()
    )
}

/// Integration ref must be a movable git ref name, not a detached commit SHA (🎯T100.4).
pub fn integration_ref_allowed(ref_name: &str) -> Result<(), String> {
    let trimmed = ref_name.trim();
    if trimmed.is_empty() {
        return Err("integration ref name is empty".into());
    }
    if !trimmed.starts_with("refs/") {
        return Err(format!(
            "integration ref `{trimmed}` must be a full ref name starting with \
             refs/ (e.g. {DEFAULT_REF}), not a bare branch name or commit SHA"
        ));
    }
    Ok(())
}

/// Verify the integration ref exists in `repo` before commit-mode writes.
pub fn integration_ref_resolves(repo: &Path, ref_name: &str) -> Result<(), WriteError> {
    integration_ref_allowed(ref_name).map_err(WriteError::Git)?;
    let spec = format!("{ref_name}^{{commit}}");
    git_query_detailed(repo, &["rev-parse", "--verify", &spec], GIT_QUERY_TIMEOUT)
        .map_err(git_err)?;
    Ok(())
}

/// Force commit-mode on/off on this thread (integration tests only).
#[doc(hidden)]
pub fn set_persist_override_for_tests(enabled: Option<bool>) {
    PERSIST_OVERRIDE.set(enabled);
}

/// Ref name for commit-mode writes (env override or [`DEFAULT_REF`]).
pub fn integration_ref_name() -> String {
    std::env::var(INTEGRATION_REF_ENV).unwrap_or_else(|_| DEFAULT_REF.to_string())
}

fn env_truthy(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some("1") | Some("true") | Some("yes"))
}

/// Outcome of one commit-mode write attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteResult {
    pub commit_sha: String,
    pub parent_sha: String,
}

/// Why a single write attempt did not land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// `update-ref` rejected the move because the ref advanced since read.
    Conflict,
    /// Any other git or I/O failure.
    Git(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Conflict => write!(f, "ref moved since read (retriable conflict)"),
            WriteError::Git(msg) => write!(f, "{msg}"),
        }
    }
}

/// Read the object id currently pointed to by `ref_name`.
pub fn ref_tip(repo: &Path, ref_name: &str) -> Result<String, WriteError> {
    git_query_detailed(repo, &["rev-parse", ref_name], GIT_QUERY_TIMEOUT)
        .map(|s| s.trim().to_string())
        .map_err(git_err)
}

/// One attempt: stage `ledger_bytes` at [`LEDGER_PATH`] and move `ref_name`.
///
/// `index_file` must be a worker-owned `GIT_INDEX_FILE` path (not the shared
/// worktree index). The file is created on first use and may be reused across
/// retries in the same writer.
pub fn write_ledger_at_ref(
    repo: &Path,
    ref_name: &str,
    ledger_bytes: &[u8],
    message: &str,
    index_file: &Path,
) -> Result<WriteResult, WriteError> {
    let parent = ref_tip(repo, ref_name)?;
    let index_env = [("GIT_INDEX_FILE", index_path(index_file))];

    git_run_detailed(repo, &index_env, &["read-tree", &parent], GIT_QUERY_TIMEOUT)
        .map_err(git_err)?;

    let blob = git_hash_object(repo, ledger_bytes, GIT_QUERY_TIMEOUT).map_err(git_err)?;
    let cacheinfo = format!("100644,{blob},{LEDGER_PATH}");
    git_run_detailed(
        repo,
        &index_env,
        &["update-index", "--add", "--cacheinfo", &cacheinfo],
        GIT_QUERY_TIMEOUT,
    )
    .map_err(git_err)?;

    let tree = git_run_detailed(repo, &index_env, &["write-tree"], GIT_QUERY_TIMEOUT)
        .map_err(git_err)?
        .trim()
        .to_string();

    let commit = git_run_detailed(
        repo,
        &index_env,
        &["commit-tree", &tree, "-p", &parent, "-m", message],
        GIT_QUERY_TIMEOUT,
    )
    .map_err(git_err)?
    .trim()
    .to_string();

    // Expected-old CAS via update-ref (same guarantee as --stdin old-value line).
    match cas_update_ref(repo, ref_name, &commit, &parent) {
        Ok(()) => Ok(WriteResult {
            commit_sha: commit,
            parent_sha: parent,
        }),
        Err(WriteError::Conflict) => Err(WriteError::Conflict),
        Err(e) => Err(e),
    }
}

/// Retry `write_ledger_at_ref` on [`WriteError::Conflict`] up to `max_attempts`.
pub fn write_ledger_at_ref_with_retry(
    repo: &Path,
    ref_name: &str,
    ledger_bytes: &[u8],
    message: &str,
    max_attempts: usize,
) -> Result<WriteResult, WriteError> {
    let index_file = private_index_path(repo);
    let mut last_conflict = false;
    for attempt in 0..max_attempts {
        match write_ledger_at_ref(repo, ref_name, ledger_bytes, message, &index_file) {
            Ok(r) => return Ok(r),
            Err(WriteError::Conflict) => {
                last_conflict = true;
                if attempt + 1 == max_attempts {
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    if last_conflict {
        Err(WriteError::Conflict)
    } else {
        Err(WriteError::Git("write exhausted without attempt".into()))
    }
}

/// One private index per `write_ledger_at_ref_with_retry` call so concurrent
/// writers in the same process never fight over the same `GIT_INDEX_FILE`.
fn private_index_path(repo: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    repo.join(".git")
        .join(format!("index-commit-mode-{}-{}", std::process::id(), seq))
}

fn index_path(index_file: &Path) -> &str {
    index_file
        .to_str()
        .expect("private index path must be UTF-8")
}

fn cas_update_ref(
    repo: &Path,
    ref_name: &str,
    commit: &str,
    parent: &str,
) -> Result<(), WriteError> {
    match git_run_detailed(
        repo,
        &[],
        &["update-ref", ref_name, commit, parent],
        GIT_QUERY_TIMEOUT,
    ) {
        Ok(_) => Ok(()),
        Err(GitQueryError::Failed(msg)) if is_ref_conflict(&msg) => Err(WriteError::Conflict),
        Err(e) => Err(git_err(e)),
    }
}

fn is_ref_conflict(stderr: &str) -> bool {
    stderr.contains("cannot lock ref")
        || stderr.contains("reference is not")
        || stderr.contains("expected")
        || stderr.contains("is at")
}

fn git_err(e: GitQueryError) -> WriteError {
    WriteError::Git(e.to_string())
}

/// Path to the ledger file relative to `repo` root (POSIX separators).
pub fn ledger_relpath(repo: &Path, ledger_abs: &Path) -> Result<String, WriteError> {
    let canon_repo = repo
        .canonicalize()
        .map_err(|e| WriteError::Git(format!("canonicalize repo: {e}")))?;
    let canon_ledger = ledger_abs
        .canonicalize()
        .map_err(|e| WriteError::Git(format!("canonicalize ledger: {e}")))?;
    match canon_ledger.strip_prefix(&canon_repo) {
        Ok(p) => Ok(p.to_string_lossy().replace('\\', "/")),
        Err(_) => Err(WriteError::Git(
            "ledger path is outside git toplevel".into(),
        )),
    }
}

/// Read ledger bytes at `ref_name` (e.g. `git show refs/heads/master:path`).
pub fn read_ledger_at_ref(
    repo: &Path,
    ref_name: &str,
    ledger_abs: &Path,
) -> Result<String, WriteError> {
    let rel = ledger_relpath(repo, ledger_abs)?;
    let spec = format!("{ref_name}:{rel}");
    git_run_detailed(repo, &[], &["show", &spec], GIT_QUERY_TIMEOUT).map_err(git_err)
}

/// After a commit-mode write, mirror the ref tip into the working tree.
pub fn refresh_worktree_ledger(
    repo: &Path,
    ref_name: &str,
    ledger_abs: &Path,
) -> Result<(), WriteError> {
    let rel = ledger_relpath(repo, ledger_abs)?;
    git_run_detailed(
        repo,
        &[],
        &["checkout", ref_name, "--", &rel],
        GIT_QUERY_TIMEOUT,
    )
    .map(|_| ())
    .map_err(git_err)
}
