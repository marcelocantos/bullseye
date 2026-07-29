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
//! Cross-clone / cross-machine allocation (🎯T51) layers a clone-scoped
//! prefix `T{scope}.{seq}` on top of the history scan: `scope` mixes a
//! durable machine tag with the absolute path of this clone's
//! `bullseye.yaml`, so two independent clones or worktrees that never
//! fetch each other still produce disjoint IDs (including two checkouts
//! on the same machine).
//! - External-mode storage (shadow tree, no git repo): falls back to
//!   in-memory-only allocation — `historical_ids` returns an empty
//!   set and callers behave identically to pre-T28.
//! - Two worktrees of the same repo allocating simultaneously: the
//!   per-`bullseye.yaml` flock doesn't serialise across worktrees
//!   (each has its own yaml file and so its own lock). The race
//!   window is "between the scan and the commit of the first
//!   worktree's tool call", typically milliseconds. Much narrower
//!   than pre-T28, where the failure mode required nothing more than
//!   "two sessions on different branches".

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use regex::Regex;

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

    let out = Command::new("git")
        .arg("-C")
        .arg(&repo_top)
        .args(["log", "-p", "--all", "--remotes", "--format=", "--"])
        .arg(&pathspec)
        .output();
    let body_bytes = match out {
        Ok(o) if o.status.success() => o.stdout,
        _ => return HashSet::new(),
    };
    let body = String::from_utf8_lossy(&body_bytes);

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
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
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

/// Durable per-machine node tag (🎯T51). Stored under the bullseye data
/// dir so external-mode and in-repo clones on the same machine share it,
/// while distinct machines almost surely differ. Mixed with the clone
/// path in [`clone_scope_tag`] so same-machine dual clones still diverge.
pub fn machine_node_tag() -> u32 {
    use std::io::Write;
    let dir = crate::config::external_root();
    let path = dir.join("node_id");
    if let Ok(s) = std::fs::read_to_string(&path) {
        let trimmed = s.trim();
        if let Ok(n) = u32::from_str_radix(trimmed, 16) {
            return n;
        }
        if let Ok(n) = trimmed.parse::<u32>() {
            return n;
        }
    }
    // Fresh tag: mix time + address bits for uniqueness without rand crate.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let tag = (nanos ^ (nanos >> 17) ^ 0xA5A5_5A5A) as u32;
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
    {
        let _ = writeln!(f, "{tag:08x}");
    }
    tag
}

/// FNV-1a 64-bit over raw bytes (no_std-friendly, no extra deps).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Clone/worktree scope tag (🎯T51): mixes the durable machine tag with
/// the absolute path of this clone's `bullseye.yaml`. Two independent
/// clones or worktrees on the **same machine** get different scopes even
/// without fetching each other; two processes on the same path share
/// the scope and advance `{seq}` under live keys + git history.
pub fn clone_scope_tag(yaml_path: &Path) -> u32 {
    let machine = machine_node_tag();
    let abs = std::fs::canonicalize(yaml_path).unwrap_or_else(|_| yaml_path.to_path_buf());
    let mut buf = Vec::with_capacity(16 + abs.as_os_str().len());
    buf.extend_from_slice(&machine.to_le_bytes());
    buf.extend_from_slice(abs.to_string_lossy().as_bytes());
    // Keep full u32 space (avoid % 1e6 birthday collisions across paths).
    fnv1a64(&buf) as u32
}

/// Next auto top-level ID: `T{scope}.{seq}` (🎯T51 global allocation).
///
/// `yaml_path` is the path of the `bullseye.yaml` being mutated — used
/// only to derive the clone/worktree scope, not read again.
pub fn next_global_top_level_id(
    yaml_path: &Path,
    file: &crate::schema::TargetsFile,
    historical: &HashSet<String>,
) -> String {
    let scope = clone_scope_tag(yaml_path);
    let prefix = format!("T{scope}.");
    let in_memory = file.targets.keys().map(String::as_str);
    let from_history = historical.iter().map(String::as_str);
    let max_seq = in_memory
        .chain(from_history)
        .filter_map(|k| {
            let suffix = k.strip_prefix(prefix.as_str())?;
            // Only the top-level slot under this scope (no T{scope}.1.2).
            if suffix.contains('.') {
                None
            } else {
                suffix.parse::<u32>().ok()
            }
        })
        .max()
        .unwrap_or(0);
    format!("T{scope}.{}", max_seq + 1)
}

#[cfg(test)]
mod t51_tests {
    use super::*;
    use crate::schema::TargetsFile;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn empty_file() -> TargetsFile {
        TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        }
    }

    #[test]
    fn two_clone_paths_produce_disjoint_id_spaces() {
        // Real dual-clone proof: two distinct filesystem paths (as two
        // independent clones/worktrees would have) must allocate
        // different first IDs under the same machine tag + empty history.
        let a_dir = tempfile::tempdir().unwrap();
        let b_dir = tempfile::tempdir().unwrap();
        let a_yaml = a_dir.path().join("bullseye.yaml");
        let b_yaml = b_dir.path().join("bullseye.yaml");
        // Touch so canonicalize can succeed when possible.
        std::fs::write(&a_yaml, "schema_version: 5\ntargets: {}\n").unwrap();
        std::fs::write(&b_yaml, "schema_version: 5\ntargets: {}\n").unwrap();

        let file = empty_file();
        let hist = HashSet::new();
        let id_a = next_global_top_level_id(&a_yaml, &file, &hist);
        let id_b = next_global_top_level_id(&b_yaml, &file, &hist);
        assert_ne!(
            id_a, id_b,
            "same-machine dual clones must not share T{{scope}}.1; got {id_a} and {id_b}"
        );
        assert!(id_a.starts_with('T') && id_a.contains('.'), "{id_a}");
        assert!(id_b.starts_with('T') && id_b.contains('.'), "{id_b}");
        // Same path twice → same first id (seq starts at 1 for empty file).
        let id_a2 = next_global_top_level_id(&a_yaml, &file, &hist);
        assert_eq!(id_a, id_a2, "same clone path must be stable for empty file");
    }

    #[test]
    fn same_clone_advances_seq_under_live_keys() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = dir.path().join("bullseye.yaml");
        std::fs::write(&yaml, "schema_version: 5\ntargets: {}\n").unwrap();
        let hist = HashSet::new();
        let mut file = empty_file();
        let first = next_global_top_level_id(&yaml, &file, &hist);
        file.targets.insert(
            first.clone(),
            crate::schema::Target {
                name: "one".into(),
                status: crate::schema::Status::Identified,
                value: 0.0,
                cost: 0.0,
                actual_cost: None,
                set_aside_reason: None,
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
        let second = next_global_top_level_id(&yaml, &file, &hist);
        assert_ne!(first, second);
        // Same scope prefix, higher seq.
        let prefix = first.rsplit_once('.').unwrap().0;
        assert!(
            second.starts_with(&format!("{prefix}.")),
            "expected same scope prefix {prefix}, got {second}"
        );
    }

    #[test]
    fn clone_scope_tag_differs_by_path() {
        let a = PathBuf::from("/tmp/bullseye-clone-a/bullseye.yaml");
        let b = PathBuf::from("/tmp/bullseye-clone-b/bullseye.yaml");
        assert_ne!(clone_scope_tag(&a), clone_scope_tag(&b));
    }
}
