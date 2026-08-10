// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Global target-ID allocation via git-history scan (🎯T28).
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
//! - One `git log -p --all --remotes --format= -- <pathspec>` call
//!   surfaces every revision that ever touched `bullseye.yaml` across
//!   every ref the clone has fetched.
//! - The diff body is grepped for `+\s+T<N>(\.<M>)*:` patterns —
//!   every target key ever **added** to the file, even if later
//!   deleted. IDs are intentionally never recycled.
//! - Results are memoised per-process keyed by the repo's top-level
//!   path so a single session's many puts/subdivides pay the scan
//!   cost once.
//!
//! Accepted residual collision risk (T51 clone-scoped IDs backed out for
//! human ergonomics — short sequential `T{n}` restored):
//! - External-mode storage (shadow tree, no git repo): falls back to
//!   in-memory-only allocation — `historical_ids` returns an empty set.
//! - Two machines / clones that allocate without fetching each other can
//!   still pick the same next `T{n}`; resolve by hand (or later policy
//!   such as even/odd developer ranges) if it becomes a major issue.
//! - Two worktrees allocating simultaneously: narrow race between scan
//!   and commit; flock is per yaml path, not cross-worktree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use regex::Regex;

use crate::bounded::{GIT_QUERY_TIMEOUT, git_query};

/// Process-global cache. Keyed by canonical repo-top path; value is
/// the set of every target ID ever added to that repo's
/// `bullseye.yaml`. Filled lazily on first call per repo.
static CACHE: Mutex<Option<HashMap<PathBuf, HashSet<String>>>> = Mutex::new(None);

static ID_RE: OnceLock<Regex> = OnceLock::new();

fn id_re() -> &'static Regex {
    ID_RE.get_or_init(|| {
        // Match a YAML key like `+  T15:` or `+    T1.2:` in a `+`
        // diff line. Whitespace between `+` and the key is required
        // and uses `[^\S\n]` rather than `\s` so the `(?m)` mode's
        // line-anchored `^` doesn't get confused by embedded newlines.
        Regex::new(r"(?m)^\+[^\S\n]+(T\d+(?:\.\d+)*):").expect("id_alloc regex is well-formed")
    })
}

/// Every target ID that has ever appeared as a key in `yaml_path`
/// across every branch and remote the local clone knows about.
///
/// Returns an empty set when:
/// - `yaml_path` is not inside a git repo (external-mode shadow
///   storage falls into this case),
/// - the git invocation fails for any reason (missing binary,
///   permissions, etc.).
///
/// Callers should union this set with the live in-memory target keys
/// when picking the next free ID — the historical scan deliberately
/// does **not** include uncommitted in-memory state.
///
/// Memoised per-process keyed by the canonical repo-top path.
pub fn historical_ids(yaml_path: &Path) -> HashSet<String> {
    let Some(parent) = yaml_path.parent() else {
        return HashSet::new();
    };
    let Some(repo_top) = git_top_level(parent) else {
        return HashSet::new();
    };

    if let Some(cached) = cache_get(&repo_top) {
        return cached;
    }

    let Some(pathspec) = relative_pathspec(yaml_path, &repo_top) else {
        return HashSet::new();
    };

    let Some(body) = git_query(
        &repo_top,
        &[
            "log",
            "-p",
            "--all",
            "--remotes",
            "--format=",
            "--",
            &pathspec,
        ],
        GIT_QUERY_TIMEOUT,
    ) else {
        return HashSet::new();
    };

    let mut ids: HashSet<String> = HashSet::new();
    for cap in id_re().captures_iter(&body) {
        ids.insert(cap[1].to_string());
    }

    cache_put(repo_top, ids.clone());
    ids
}

fn cache_get(repo_top: &Path) -> Option<HashSet<String>> {
    let guard = CACHE.lock().expect("id_alloc cache poisoned");
    let cache = guard.as_ref()?;
    cache.get(repo_top).cloned()
}

fn cache_put(repo_top: PathBuf, ids: HashSet<String>) {
    let mut guard = CACHE.lock().expect("id_alloc cache poisoned");
    let cache = guard.get_or_insert_with(HashMap::new);
    cache.insert(repo_top, ids);
}

fn relative_pathspec(yaml_path: &Path, repo_top: &Path) -> Option<String> {
    let canonical = yaml_path.canonicalize().ok()?;
    let stripped = canonical.strip_prefix(repo_top).ok()?;
    stripped.to_str().map(str::to_string)
}

fn git_top_level(dir: &Path) -> Option<PathBuf> {
    let s = git_query(dir, &["rev-parse", "--show-toplevel"], GIT_QUERY_TIMEOUT)?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// Drop every cached entry. Exposed for integration tests that need
/// to verify the scan picks up state from a freshly-mutated repo. The
/// production cache lives for the process lifetime; production code
/// does **not** call this.
pub fn clear_cache_for_tests() {
    let mut guard = CACHE.lock().expect("id_alloc cache poisoned");
    if let Some(cache) = guard.as_mut() {
        cache.clear();
    }
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
}
