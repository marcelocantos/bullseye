// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Global target-ID allocation via git-history scan (🎯T28, 🎯T92).
//!
//! Auto-assigning the next free target ID by reading only the
//! in-memory `TargetsFile` produces collisions when two branches or
//! two parallel sessions each pick what they think is the next free
//! slot — each one sees only the state of its own current working
//! tree. Bullseye's global view includes every branch and remote the
//! local clone knows about, so the ID allocator consults git history
//! before picking.
//!
//! Implementation:
//!
//! - A grow-only set of every target ID ever **added** to
//!   `bullseye.yaml` is kept per repo, together with the ref tips
//!   (`git rev-parse --all`) that set was computed from (🎯T92).
//! - The first scan runs `git log -p --all --remotes`; later scans
//!   union `git log -p <current tips> --not <prior tips> -- <pathspec>`.
//! - The set and tips persist under `$BULLSEYE_DATA_DIR/id-history/`
//!   so a daemon restart does not repeat the full walk.
//! - Results are also memoised in-process keyed by the sorted tip
//!   list so unchanged refs answer without touching disk or git.
//! - Every worktree of the repo (`git worktree list`) also contributes
//!   the keys in its **working-tree** ledger, committed or not. A row
//!   filed on the main clone stays uncommitted until the work it tracks
//!   lands, and no ref contains it until then — so a sibling worktree
//!   allocating from history alone mints that same ID a second time.
//!   Observed in jevons (its 🎯T913): a fleet worker filed from its
//!   worktree and got the main ledger's newest, still-uncommitted ID.
//!   Each file's keys are memoised by mtime and length; this set is not
//!   grow-only — a row deleted from a working tree frees its ID.
//!
//! Accepted residual collision risk (T51 clone-scoped IDs backed out for
//! human ergonomics — short sequential `T{n}` restored):
//! - External-mode storage (shadow tree, no git repo): falls back to
//!   in-memory-only allocation — `historical_ids` returns an empty set.
//! - A git timeout or other scan failure **refuses** allocation
//!   (`id_history_scan_failed`) rather than guessing from live keys (Fable F6).
//! - Two machines / clones that allocate without fetching each other can
//!   still pick the same next `T{n}`; resolve by hand (or later policy
//!   such as even/odd developer ranges) if it becomes a major issue.
//! - Two worktrees allocating simultaneously: narrow race between the
//!   worktree scan and the write; flock is per yaml path, not
//!   cross-worktree. Sequential filings across worktrees are covered.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::bounded::{GIT_QUERY_TIMEOUT, GitQueryError, git_query_detailed};
use crate::cache;
use crate::config;

/// Process-global cache. Keyed by canonical repo-top path; value is
/// the sorted ref tips at scan time plus the grow-only ID set.
type CachedScan = (Instant, Vec<String>, HashSet<String>);

static CACHE: Mutex<Option<HashMap<PathBuf, CachedScan>>> = Mutex::new(None);

static ID_RE: OnceLock<Regex> = OnceLock::new();

/// Keys of one worktree's working-tree ledger, stamped with the
/// modification time and length they were read at.
type CachedLedgerKeys = (Instant, (std::time::SystemTime, u64), HashSet<String>);

static WORKTREE_CACHE: Mutex<Option<HashMap<PathBuf, CachedLedgerKeys>>> = Mutex::new(None);

/// One history scan per repo at a time so a burst of MCP writes does not
/// each spawn a full `git log -p --all --remotes` against the same object
/// store (🎯T101).
static REPO_HISTORY_LOCKS: Mutex<Option<HashMap<PathBuf, Arc<Mutex<()>>>>> = Mutex::new(None);

fn repo_history_lock(repo_top: &Path) -> Arc<Mutex<()>> {
    let key = repo_top
        .canonicalize()
        .unwrap_or_else(|_| repo_top.to_path_buf());
    let mut guard = REPO_HISTORY_LOCKS
        .lock()
        .expect("id_alloc repo history lock map poisoned");
    let map = guard.get_or_insert_with(HashMap::new);
    map.entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

static KEY_RE: OnceLock<Regex> = OnceLock::new();

thread_local! {
    static GIT_LOG_ARGS: RefCell<Vec<Vec<String>>> = const { RefCell::new(Vec::new()) };
}

fn id_re() -> &'static Regex {
    ID_RE.get_or_init(|| {
        // Match a YAML key like `+  T15:` or `+    T1.2:` in a `+`
        // diff line. Whitespace between `+` and the key is required
        // and uses `[^\S\n]` rather than `\s` so the `(?m)` mode's
        // line-anchored `^` doesn't get confused by embedded newlines.
        Regex::new(r"(?m)^\+[^\S\n]+(T\d+(?:\.\d+)*):").expect("id_alloc regex is well-formed")
    })
}

fn key_re() -> &'static Regex {
    KEY_RE.get_or_init(|| {
        // A target key line in a ledger file: indented `T15:` or
        // `T1.2:` with nothing after the colon. Requiring the bare key
        // keeps prose in a block scalar (`    T5: see above`) from
        // reserving a number.
        Regex::new(r"(?m)^[^\S\n]+(T\d+(?:\.\d+)*):[^\S\n]*$")
            .expect("id_alloc key regex is well-formed")
    })
}

const PERSIST_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct PersistedIdHistory {
    version: u32,
    ids: Vec<String>,
    scanned_tips: Vec<String>,
}

/// Why a historical-ID scan could not produce a trustworthy set.
#[derive(Debug, PartialEq, Eq)]
pub enum HistoricalIdsError {
    /// The path sits in a git repo, but git could not answer (timeout,
    /// missing binary, non-zero exit that is not "not a repository").
    /// Allocating from live keys alone would recycle an ID that exists
    /// only on another ref (🎯T28).
    ScanFailed { reason: String },
}

impl std::fmt::Display for HistoricalIdsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScanFailed { reason } => write!(
                f,
                "refusing to allocate a target ID — git history scan failed ({reason}). \
                 Retry when git can answer; inventing the next T{{n}} from live keys alone \
                 can recycle an ID that exists only on another ref (🎯T28)."
            ),
        }
    }
}

impl HistoricalIdsError {
    pub fn code(&self) -> crate::api::ErrorCode {
        crate::api::ErrorCode::IdHistoryScanFailed
    }
}

/// Every target ID that has ever appeared as a key in `yaml_path`
/// across every branch and remote the local clone knows about.
///
/// Returns an empty set when `yaml_path` is not inside a git repo
/// (external-mode shadow storage). Returns [`HistoricalIdsError`] when
/// the path *is* in a repo but git cannot answer — timeout, missing
/// binary, or a non-zero exit that is not "not a repository". Callers
/// that allocate must refuse rather than guess from live keys (🎯T28).
///
/// Memoised per-process keyed by the canonical repo-top path and the
/// sorted ref-tip list (🎯T92).
pub fn historical_ids(yaml_path: &Path) -> Result<HashSet<String>, HistoricalIdsError> {
    try_historical_ids_bounded(yaml_path, GIT_QUERY_TIMEOUT)
}

/// Same scan as [`historical_ids`], with an explicit bound so tests can
/// drive a short timeout.
pub fn try_historical_ids_bounded(
    yaml_path: &Path,
    timeout: Duration,
) -> Result<HashSet<String>, HistoricalIdsError> {
    scan_historical_ids(yaml_path, timeout)
}

fn scan_failed(reason: impl Into<String>) -> HistoricalIdsError {
    HistoricalIdsError::ScanFailed {
        reason: reason.into(),
    }
}

fn scan_historical_ids(
    yaml_path: &Path,
    timeout: Duration,
) -> Result<HashSet<String>, HistoricalIdsError> {
    let Some(parent) = yaml_path.parent() else {
        return Ok(HashSet::new());
    };
    let repo_top = match git_top_level_result(parent, timeout) {
        Ok(p) => p,
        Err(GitQueryError::NotARepo) => return Ok(HashSet::new()),
        Err(GitQueryError::Failed(reason)) => return Err(scan_failed(reason)),
    };
    let pathspec = relative_pathspec(yaml_path, &repo_top)
        .ok_or_else(|| scan_failed("could not relativize bullseye.yaml to the repo root"))?;

    // History is grow-only and cached by ref tips; the worktree ledgers
    // change without any ref moving, so they are read on every call.
    let mut ids = git_history_ids(&repo_top, &pathspec, timeout)?;
    ids.extend(worktree_ledger_ids(&repo_top, &pathspec, timeout)?);
    Ok(ids)
}

/// Every key ever added to `pathspec` on any ref (🎯T28, 🎯T92).
fn git_history_ids(
    repo_top: &Path,
    pathspec: &str,
    timeout: Duration,
) -> Result<HashSet<String>, HistoricalIdsError> {
    let repo_top = repo_top.to_path_buf();
    let scan_lock = repo_history_lock(&repo_top);
    let _scan_guard = scan_lock
        .lock()
        .expect("id_alloc repo history lock poisoned");

    let current_tips = match ref_tips(&repo_top, timeout) {
        Ok(t) => t,
        Err(GitQueryError::NotARepo) => return Ok(HashSet::new()),
        Err(GitQueryError::Failed(reason)) => return Err(scan_failed(reason)),
    };

    if let Some(cached) = cache_get(&repo_top, &current_tips) {
        return Ok(cached);
    }

    let persisted = load_persisted(&repo_top);
    let prior_tips = persisted
        .as_ref()
        .map(|p| p.scanned_tips.clone())
        .unwrap_or_default();
    let mut ids: HashSet<String> = persisted
        .as_ref()
        .map(|p| p.ids.iter().cloned().collect())
        .unwrap_or_default();

    if !prior_tips.is_empty() && prior_tips == current_tips {
        cache_put(repo_top, current_tips, ids.clone());
        return Ok(ids);
    }

    let full_scan = prior_tips.is_empty();
    if !full_scan {
        let prior_set: HashSet<&String> = prior_tips.iter().collect();
        let has_new_tips = current_tips.iter().any(|t| !prior_set.contains(t));
        if !has_new_tips {
            // Ref tips shrank (deleted refs only) — grow-only history is unchanged.
            save_persisted(&repo_top, &ids, &current_tips)?;
            cache_put(repo_top, current_tips, ids.clone());
            return Ok(ids);
        }
    }
    let body = match run_history_log(
        &repo_top,
        full_scan,
        &current_tips,
        &prior_tips,
        pathspec,
        timeout,
    ) {
        Ok(body) => body,
        Err(GitQueryError::NotARepo) => return Ok(HashSet::new()),
        Err(GitQueryError::Failed(reason)) => return Err(scan_failed(reason)),
    };

    for cap in id_re().captures_iter(&body) {
        ids.insert(cap[1].to_string());
    }

    save_persisted(&repo_top, &ids, &current_tips)?;
    cache_put(repo_top, current_tips, ids.clone());
    Ok(ids)
}

/// Every key in the working-tree ledger at `pathspec` of each worktree
/// of this repo, the caller's own included — committed or not.
///
/// A worktree whose ledger is absent (a bare entry, a directory deleted
/// without `git worktree prune`) contributes nothing. Failing to list
/// the worktrees, or to read a ledger that exists, refuses allocation
/// like a failed history scan: a guess from the rest could hand out the
/// very ID the unread file holds.
fn worktree_ledger_ids(
    repo_top: &Path,
    pathspec: &str,
    timeout: Duration,
) -> Result<HashSet<String>, HistoricalIdsError> {
    let listing = match git_query_detailed(
        repo_top,
        &["worktree", "list", "--porcelain", "-z"],
        timeout,
    ) {
        Ok(listing) => listing,
        Err(GitQueryError::NotARepo) => return Ok(HashSet::new()),
        Err(GitQueryError::Failed(reason)) => return Err(scan_failed(reason)),
    };

    let mut ids = HashSet::new();
    for field in listing.split('\0') {
        let Some(worktree) = field.strip_prefix("worktree ") else {
            continue;
        };
        let ledger = Path::new(worktree).join(pathspec);
        let keys = ledger_keys(&ledger).map_err(|e| {
            scan_failed(format!(
                "could not read worktree ledger {}: {e}",
                ledger.display()
            ))
        })?;
        ids.extend(keys);
    }
    Ok(ids)
}

/// Target keys in one ledger file, memoised by modification time and
/// length. A missing file has no keys.
fn ledger_keys(path: &Path) -> std::io::Result<HashSet<String>> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(e) => return Err(e),
    };
    let stamp = (meta.modified()?, meta.len());

    {
        let guard = WORKTREE_CACHE
            .lock()
            .expect("id_alloc worktree cache poisoned");
        if let Some((cached_at, cached_stamp, keys)) = guard.as_ref().and_then(|c| c.get(path))
            && *cached_stamp == stamp
            && !cache::expired(*cached_at)
        {
            return Ok(keys.clone());
        }
    }

    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(e) => return Err(e),
    };
    let keys: HashSet<String> = key_re()
        .captures_iter(&raw)
        .map(|cap| cap[1].to_string())
        .collect();

    let mut guard = WORKTREE_CACHE
        .lock()
        .expect("id_alloc worktree cache poisoned");
    let cache = guard.get_or_insert_with(HashMap::new);
    cache.retain(|_, (cached_at, _, _)| !cache::expired(*cached_at));
    cache.insert(path.to_path_buf(), (Instant::now(), stamp, keys.clone()));
    Ok(keys)
}

fn run_history_log(
    repo_top: &Path,
    full_scan: bool,
    current_tips: &[String],
    prior_tips: &[String],
    pathspec: &str,
    timeout: Duration,
) -> Result<String, GitQueryError> {
    let mut owned: Vec<String> = vec!["log".into(), "-p".into(), "--format=".into()];
    if full_scan {
        owned.push("--all".into());
        owned.push("--remotes".into());
    } else {
        let prior_set: HashSet<&String> = prior_tips.iter().collect();
        let new_tips: Vec<String> = current_tips
            .iter()
            .filter(|t| !prior_set.contains(t))
            .cloned()
            .collect();
        owned.extend(new_tips);
        owned.push("--not".into());
        owned.extend(prior_tips.iter().cloned());
    }
    owned.push("--".into());
    owned.push(pathspec.into());
    record_git_log_args_for_tests(&owned);
    let arg_refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    git_query_detailed(repo_top, &arg_refs, timeout)
}

fn record_git_log_args_for_tests(args: &[String]) {
    GIT_LOG_ARGS.with(|log| log.borrow_mut().push(args.to_vec()));
}

/// Sorted ref tips from `git rev-parse --all`.
fn ref_tips(repo_top: &Path, timeout: Duration) -> Result<Vec<String>, GitQueryError> {
    let refs = git_query_detailed(repo_top, &["rev-parse", "--all"], timeout)?;
    let mut tips: Vec<String> = refs
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    tips.sort();
    tips.dedup();
    Ok(tips)
}

fn cache_get(repo_top: &Path, tips: &[String]) -> Option<HashSet<String>> {
    let guard = CACHE.lock().expect("id_alloc cache poisoned");
    let cache = guard.as_ref()?;
    let (stamped, cached_tips, ids) = cache.get(repo_top)?;
    (cached_tips == tips && !cache::expired(*stamped)).then(|| ids.clone())
}

fn cache_put(repo_top: PathBuf, tips: Vec<String>, ids: HashSet<String>) {
    let mut guard = CACHE.lock().expect("id_alloc cache poisoned");
    let cache = guard.get_or_insert_with(HashMap::new);
    cache.retain(|_, (stamped, _, _)| !cache::expired(*stamped));
    cache.insert(repo_top, (Instant::now(), tips, ids));
}

fn relative_pathspec(yaml_path: &Path, repo_top: &Path) -> Option<String> {
    let canonical = yaml_path.canonicalize().ok()?;
    let stripped = canonical.strip_prefix(repo_top).ok()?;
    stripped.to_str().map(str::to_string)
}

fn git_top_level_result(dir: &Path, timeout: Duration) -> Result<PathBuf, GitQueryError> {
    let s = git_query_detailed(dir, &["rev-parse", "--show-toplevel"], timeout)?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(GitQueryError::Failed(
            "git rev-parse --show-toplevel was empty".into(),
        ));
    }
    Ok(PathBuf::from(trimmed))
}

fn persist_path(repo_top: &Path) -> PathBuf {
    let canonical = repo_top
        .canonicalize()
        .unwrap_or_else(|_| repo_top.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let key = format!("{:x}", hasher.finalize());
    config::external_root()
        .join("id-history")
        .join(format!("{key}.json"))
}

fn load_persisted(repo_top: &Path) -> Option<PersistedIdHistory> {
    let path = persist_path(repo_top);
    let raw = std::fs::read_to_string(&path).ok()?;
    let parsed: PersistedIdHistory = serde_json::from_str(&raw).ok()?;
    if parsed.version != PERSIST_VERSION {
        return None;
    }
    Some(parsed)
}

fn save_persisted(
    repo_top: &Path,
    ids: &HashSet<String>,
    tips: &[String],
) -> Result<(), HistoricalIdsError> {
    let path = persist_path(repo_top);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| scan_failed(format!("could not create id-history dir: {e}")))?;
    }
    let mut id_vec: Vec<String> = ids.iter().cloned().collect();
    id_vec.sort();
    let payload = PersistedIdHistory {
        version: PERSIST_VERSION,
        ids: id_vec,
        scanned_tips: tips.to_vec(),
    };
    let json = serde_json::to_string(&payload)
        .map_err(|e| scan_failed(format!("could not serialise id-history cache: {e}")))?;
    std::fs::write(&path, json)
        .map_err(|e| scan_failed(format!("could not write id-history cache: {e}")))?;
    Ok(())
}

fn clear_persisted_id_history() {
    let dir = config::external_root().join("id-history");
    if dir.is_dir() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Drop every cached entry and on-disk id-history store. Exposed for
/// integration tests that need a fresh scan. Production code does
/// **not** call this.
pub fn clear_cache_for_tests() {
    let mut guard = CACHE.lock().expect("id_alloc cache poisoned");
    if let Some(cache) = guard.as_mut() {
        cache.clear();
    }
    clear_persisted_id_history();
    GIT_LOG_ARGS.with(|log| log.borrow_mut().clear());
}

/// Drop only the in-process memoisation (🎯T92 persistence tests).
pub fn clear_process_cache_for_tests() {
    let mut guard = CACHE.lock().expect("id_alloc cache poisoned");
    if let Some(cache) = guard.as_mut() {
        cache.clear();
    }
}

/// Arguments passed to the most recent `git log` history scan on this
/// thread (tests only).
pub fn last_git_log_args_for_tests() -> Option<Vec<String>> {
    GIT_LOG_ARGS.with(|log| log.borrow().last().cloned())
}

/// How many `git log` history scans ran on this thread (tests only).
pub fn git_log_invocation_count_for_tests() -> usize {
    GIT_LOG_ARGS.with(|log| log.borrow().len())
}

/// Next auto top-level ID: short sequential `T{n}` over live keys ∪ git
/// history (🎯T28). Cross-machine uniqueness is not guaranteed — T51's
/// clone-scoped form was backed out for hand-typing ergonomics.
pub fn next_top_level_id(
    file: &crate::schema::TargetsFile,
    historical: &HashSet<String>,
) -> String {
    let in_memory = file.targets.keys().map(String::as_str);
    let from_history = historical.iter().map(String::as_str);
    let max_num = in_memory
        .chain(from_history)
        .filter_map(|k| {
            let num_str = k.strip_prefix('T')?;
            // Only plain top-level T{n}, not T1.2 or scoped leftovers.
            if num_str.contains('.') {
                None
            } else {
                num_str.parse::<u32>().ok()
            }
        })
        .max()
        .unwrap_or(0);
    format!("T{}", max_num + 1)
}

#[cfg(test)]
mod top_level_id_tests {
    use super::*;
    use crate::schema::TargetsFile;
    use std::collections::BTreeMap;

    fn empty_file() -> TargetsFile {
        TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
            load_parse_failures: Default::default(),
        }
    }

    #[test]
    fn empty_file_starts_at_t1() {
        let id = next_top_level_id(&empty_file(), &HashSet::new());
        assert_eq!(id, "T1");
    }

    #[test]
    fn skips_historical_slots() {
        let mut hist = HashSet::new();
        hist.insert("T1".into());
        hist.insert("T2".into());
        hist.insert("T3".into());
        let id = next_top_level_id(&empty_file(), &hist);
        assert_eq!(id, "T4");
    }

    #[test]
    fn advances_past_live_plain_t_ids() {
        let mut file = empty_file();
        file.targets.insert(
            "T5".into(),
            crate::schema::Target {
                name: "five".into(),
                status: crate::schema::Status::Identified,
                value: 0.0,
                cost: 0.0,
                actual_cost: None,
                set_aside_reason: None,
                attestation: None,
                acceptance: vec!["ok".into()],
                checks: vec![],
                context: String::new(),
                gates: vec![],
                depends_on: vec![],
                cross_depends: vec![],
                cross_enables: vec![],
                tags: vec![],
                strategy: None,
                origin: "test".into(),
                discovered: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                achieved: None,
                owned_by: None,
                postponed_until: None,
                postpone_predicate: None,
            },
        );
        let id = next_top_level_id(&file, &HashSet::new());
        assert_eq!(id, "T6");
    }

    #[test]
    fn scan_failure_uses_id_history_scan_failed_code() {
        assert_eq!(
            HistoricalIdsError::ScanFailed {
                reason: "timed out".into()
            }
            .code(),
            crate::api::ErrorCode::IdHistoryScanFailed
        );
    }
}
