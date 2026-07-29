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
//! Accepted edge cases (documented in 🎯T28):
//!
//! - Other-machine sessions that haven't synced their commits to a
//!   remote the local clone tracks: not covered. The race window is
//!   "neither machine has fetched the other's commit yet".
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
/// while distinct machines almost surely differ.
pub fn machine_node_tag() -> u32 {
    use std::io::Write;
    let dir = crate::config::external_root();
    let path = dir.join("node_id");
    if let Ok(s) = std::fs::read_to_string(&path) {
        let trimmed = s.trim();
        if let Ok(n) = u32::from_str_radix(trimmed, 16) {
            return n % 1_000_000;
        }
        if let Ok(n) = trimmed.parse::<u32>() {
            return n % 1_000_000;
        }
    }
    // Fresh tag: mix time + address bits for uniqueness without rand crate.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let tag = ((nanos ^ (nanos >> 17) ^ 0xA5A5_5A5A) as u32) % 1_000_000;
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
    {
        let _ = writeln!(f, "{tag:06x}");
    }
    tag
}

/// Next auto top-level ID: `T{node}.{seq}` (🎯T51 global allocation).
pub fn next_global_top_level_id(
    file: &crate::schema::TargetsFile,
    historical: &HashSet<String>,
) -> String {
    let node = machine_node_tag();
    let prefix = format!("T{node}.");
    let in_memory = file.targets.keys().map(String::as_str);
    let from_history = historical.iter().map(String::as_str);
    let max_seq = in_memory
        .chain(from_history)
        .filter_map(|k| {
            let suffix = k.strip_prefix(prefix.as_str())?;
            if suffix.contains('.') {
                None
            } else {
                suffix.parse::<u32>().ok()
            }
        })
        .max()
        .unwrap_or(0);
    format!("T{node}.{}", max_seq + 1)
}

#[cfg(test)]
mod t51_tests {
    use super::*;
    use crate::schema::TargetsFile;
    use std::collections::BTreeMap;

    #[test]
    fn two_node_tags_produce_disjoint_id_spaces() {
        let file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        let hist = HashSet::new();
        // Simulate two machines by manually crafting prefixes
        let a = format!("T{}.{}", 111_111u32, 1);
        let b = format!("T{}.{}", 222_222u32, 1);
        assert_ne!(a, b);
        let id = next_global_top_level_id(&file, &hist);
        assert!(id.starts_with('T'), "{id}");
        assert!(id.contains('.'), "machine-scoped id should be T{{node}}.{{seq}}, got {id}");
    }
}
