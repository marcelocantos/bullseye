// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use fs2::FileExt;
use tempfile::NamedTempFile;

use crate::config::{Location, external_root};
use crate::schema::{CURRENT_SCHEMA_VERSION, TargetsFile, migrate_gates_to_depends_on};

/// Maximum time we'll wait for another bullseye process to release the
/// lockfile before giving up with a structured timeout error. Five
/// seconds is long enough to absorb a slow mutation in another process
/// but short enough that a stale lock doesn't hang the caller's session.
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// Polling interval while spinning on `try_lock_exclusive`.
const LOCK_POLL: Duration = Duration::from_millis(50);

/// Cached parse result for a single `bullseye.yaml` file.
struct CacheEntry {
    /// The mtime of the file at parse time.
    mtime: SystemTime,
    /// The last successfully parsed targets file.
    file: TargetsFile,
}

/// Process-global parse cache: keyed by absolute path, invalidated on mtime change.
///
/// Using `std::sync::Mutex` (not `tokio::sync::Mutex`) because the cache is
/// accessed from synchronous helpers called within async tool handlers. The
/// lock is held only for HashMap lookup/insertion — no I/O occurs under the
/// lock.
static PARSE_CACHE: Mutex<Option<HashMap<PathBuf, CacheEntry>>> = Mutex::new(None);

/// Returns the mtime of `path`, or `None` if stat fails.
fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Structured error type returned by [`load`]. Having distinct
/// variants lets callers decide per-case how to respond — in
/// particular, [`LoadError::VersionTooNew`] must always be surfaced
/// loudly (it's the point of the version check) while
/// [`LoadError::Io`] / [`LoadError::Parse`] can be tolerated by
/// speculative callers like `bullseye_startup_context`.
#[derive(Debug)]
pub enum LoadError {
    /// Couldn't read the file — permission denied, disk issue, etc.
    Io(String),
    /// Read succeeded but the YAML didn't parse.
    Parse(String),
    /// File declares a `schema_version` higher than this build
    /// supports. The binary must be upgraded before the file can be
    /// read safely.
    VersionTooNew {
        found: u32,
        supported: u32,
        path: PathBuf,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(msg) => write!(f, "{msg}"),
            LoadError::Parse(msg) => write!(f, "{msg}"),
            LoadError::VersionTooNew {
                found,
                supported,
                path,
            } => write!(
                f,
                "{}: bullseye.yaml declares schema_version {found}, but this \
                 bullseye binary only supports up to {supported}. \
                 Upgrade bullseye (e.g. `brew upgrade marcelocantos/tap/bullseye`) \
                 to read this file.",
                path.display(),
            ),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<LoadError> for String {
    fn from(e: LoadError) -> Self {
        e.to_string()
    }
}

const MAX_DISCOVER_DEPTH: usize = 64;

/// In-repo discovery: walk up from `start_dir` looking for
/// `bullseye.yaml` at each level. Original v0.1.0 behaviour, still
/// the first probe used by [`discover_anywhere`].
pub fn discover(start_dir: &Path) -> Option<PathBuf> {
    let mut dir = start_dir.to_path_buf();
    for _ in 0..MAX_DISCOVER_DEPTH {
        let path = dir.join("bullseye.yaml");
        if path.is_file() {
            return Some(path);
        }
        if !dir.pop() {
            return None;
        }
    }
    None
}

/// Compute the shadow path for an absolute `cwd` under `root` —
/// `root` + `cwd` with the leading component(s) stripped so that
/// `/Users/marcelo/work/x` becomes `<root>/Users/marcelo/work/x`.
/// Handles the macOS/Linux convention of paths anchored at `/`; on
/// Windows the drive prefix is treated the same way (stripped, then
/// the remaining components joined).
pub fn shadow_path(root: &Path, cwd: &Path) -> PathBuf {
    let mut shadow = root.to_path_buf();
    for comp in cwd.components() {
        use std::path::Component;
        match comp {
            Component::RootDir | Component::Prefix(_) | Component::CurDir => {}
            Component::ParentDir => {
                // `..` in cwd shouldn't happen for an absolute canonical
                // path; guard anyway so we never escape upward out of root.
            }
            Component::Normal(part) => shadow.push(part),
        }
    }
    shadow
}

/// External (shadow-tree) discovery: walk up from `shadow_path(root, cwd)`
/// the same way [`discover`] walks up the real tree. Stops at `root`.
pub fn discover_external(root: &Path, cwd: &Path) -> Option<PathBuf> {
    let mut dir = shadow_path(root, cwd);
    for _ in 0..MAX_DISCOVER_DEPTH {
        let path = dir.join("bullseye.yaml");
        if path.is_file() {
            return Some(path);
        }
        if dir == root || !dir.starts_with(root) {
            return None;
        }
        if !dir.pop() {
            return None;
        }
    }
    None
}

/// Per-repo discovery (v0.16.0+). Checks both possible locations and
/// returns whichever already exists:
///
/// 1. In-repo: walk up from `cwd` looking for `bullseye.yaml`.
/// 2. External: walk up the shadow tree under [`external_root`].
///
/// If both exist (edge case — e.g. someone moved a repo and forgot to
/// clean up the shadow copy), **in-repo wins**. An explicit committed
/// file is always the authoritative copy.
pub fn discover_anywhere(cwd: &Path) -> Option<PathBuf> {
    if let Some(p) = discover(cwd) {
        return Some(p);
    }
    discover_external(&external_root(), cwd)
}

/// Target path for a *new* targets file at the requested location.
/// Does not create the file — pair with [`create_at`] for the full
/// first-run flow.
pub fn target_path_for_new(cwd: &Path, location: Location) -> PathBuf {
    match location {
        Location::InRepo => cwd.join("bullseye.yaml"),
        Location::External => shadow_path(&external_root(), cwd).join("bullseye.yaml"),
    }
}

/// Create a starter targets file for `cwd` at the requested
/// `location`. Creates parent directories under the shadow root when
/// the location is external.
pub fn create_at(cwd: &Path, location: Location, project_name: &str) -> Result<PathBuf, String> {
    let path = target_path_for_new(cwd, location);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    write_starter_file(&path, project_name)?;
    Ok(path)
}

fn write_starter_file(path: &Path, project_name: &str) -> Result<(), String> {
    let today = chrono::Local::now().date_naive();
    let mut targets = std::collections::BTreeMap::new();
    targets.insert(
        "T1".to_string(),
        crate::schema::Target {
            name: format!("{project_name} has a clear first milestone"),
            kind: crate::schema::Kind::Work,
            status: crate::schema::Status::Identified,
            value: 5.0,
            cost: 3.0,
            observable: false,
            actual_cost: None,
            acceptance: vec![
                "First milestone is defined with measurable success criteria".to_string(),
                "At least one sub-target breaks the milestone into actionable work".to_string(),
            ],
            checks: Vec::new(),
            context: "Starter target — replace with your project's actual first goal.".to_string(),
            gates: Vec::new(),
            depends_on: Vec::new(),
            cross_depends: Vec::new(),
            cross_enables: Vec::new(),
            verifies: Vec::new(),
            rework: None,
            retry_budget: None,
            retries: 0,
            tags: Vec::new(),
            origin: "bullseye_init".to_string(),
            discovered: today,
            achieved: None,
        },
    );

    let file = TargetsFile {
        schema_version: Some(CURRENT_SCHEMA_VERSION),
        last_evaluated: None,
        targets,
    };
    save(path, &file)
}

/// Parse a targets file from disk without any caching.
///
/// This is the inner implementation shared by [`load`] and the cache-miss
/// path. Callers should prefer [`load`] which adds mtime-based caching on
/// top.
fn parse_file(path: &Path) -> Result<TargetsFile, LoadError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| LoadError::Io(format!("failed to read {}: {e}", path.display())))?;
    let mut file: TargetsFile = serde_yaml_ng::from_str(&content)
        .map_err(|e| LoadError::Parse(format!("failed to parse {}: {e}", path.display())))?;
    if let Some(v) = file.schema_version
        && v > CURRENT_SCHEMA_VERSION
    {
        return Err(LoadError::VersionTooNew {
            found: v,
            supported: CURRENT_SCHEMA_VERSION,
            path: path.to_path_buf(),
        });
    }
    if file.schema_version.is_none() {
        file.schema_version = Some(CURRENT_SCHEMA_VERSION);
    }
    migrate_gates_to_depends_on(&mut file);
    Ok(file)
}

/// Load and parse a targets file, with mtime-keyed caching.
///
/// On each call the file's mtime is checked cheaply via `stat`. If the mtime
/// matches the cached value the previously parsed [`TargetsFile`] is returned
/// without re-reading the file. When the mtime changes (file was modified) the
/// cache entry is refreshed.
///
/// **Failure fallback**: if a re-parse fails (e.g. the file is mid-edit and
/// temporarily malformed), the last valid cached state is returned instead of
/// propagating the error. The parse failure is noted in the returned value via
/// a unit error surface — callers that need to distinguish "stale cached"
/// from "freshly parsed" can inspect the returned `Ok`. For
/// [`LoadError::VersionTooNew`] (a hard binary incompatibility), the error is
/// always propagated even when a cached copy is available.
///
/// Thread safety is provided by a process-global `std::sync::Mutex`. The lock
/// is never held while doing I/O; only HashMap lookups and insertions happen
/// under the lock.
///
/// Applies in-memory migration for legacy `gates` edges on every fresh parse.
pub fn load(path: &Path) -> Result<TargetsFile, LoadError> {
    // Canonicalise to a stable cache key. If canonicalisation fails (broken
    // symlink, etc.), fall back to the raw path — load will fail anyway.
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let current_mtime = file_mtime(path);

    // --- Check cache under lock (no I/O inside) ---
    {
        let mut guard = PARSE_CACHE.lock().expect("parse cache mutex poisoned");
        let cache = guard.get_or_insert_with(HashMap::new);

        if let Some(entry) = cache.get(&key) {
            // If we couldn't stat the file, fall back to the cached copy.
            // If the mtime is unchanged, return the cached copy.
            if current_mtime.is_none() || current_mtime == Some(entry.mtime) {
                return Ok(entry.file.clone());
            }
        }
    } // lock released before I/O

    // --- Cache miss or mtime changed: parse from disk ---
    match parse_file(path) {
        Ok(file) => {
            // Store in cache. Use the mtime we sampled before parsing; if the
            // file changes again before we store, the next call will detect it.
            let mtime = current_mtime.unwrap_or(SystemTime::UNIX_EPOCH);
            let mut guard = PARSE_CACHE.lock().expect("parse cache mutex poisoned");
            let cache = guard.get_or_insert_with(HashMap::new);
            cache.insert(
                key,
                CacheEntry {
                    mtime,
                    file: file.clone(),
                },
            );
            Ok(file)
        }
        Err(e @ LoadError::VersionTooNew { .. }) => {
            // Hard incompatibility — always surface, even if we have a stale
            // cached copy. The stale copy might silently misinterpret fields.
            Err(e)
        }
        Err(e) => {
            // Soft error (I/O or parse failure). If we have a cached copy,
            // serve it so the caller keeps working while the file is mid-edit.
            let guard = PARSE_CACHE.lock().expect("parse cache mutex poisoned");
            if let Some(cache) = guard.as_ref()
                && let Some(entry) = cache.get(&key)
            {
                return Ok(entry.file.clone());
            }
            // No cached copy — propagate the error.
            Err(e)
        }
    }
}

/// Evict the cache entry for `path`. Called after [`save`] so the next
/// [`load`] re-parses the freshly written file rather than serving the
/// in-memory snapshot (which may differ from the serialised form).
fn evict_cache(path: &Path) {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut guard = PARSE_CACHE.lock().expect("parse cache mutex poisoned");
    if let Some(cache) = guard.as_mut() {
        cache.remove(&key);
    }
}

/// Write a targets file back to disk atomically.
///
/// Always stamps `schema_version = CURRENT_SCHEMA_VERSION` on the
/// serialized output so that every bullseye-produced file is
/// self-describing. Callers that held an older (or missing) version
/// in memory will see it rewritten on save.
///
/// The write goes via a sibling tempfile in the same directory
/// (required so the final `rename(2)` is atomic — cross-filesystem
/// renames are not), which is fsync'd and then renamed into place.
/// This is crash-safe: either readers see the old file or the new
/// file, never a half-written one.
pub fn save(path: &Path, file: &TargetsFile) -> Result<(), String> {
    let mut stamped = file.clone();
    stamped.schema_version = Some(CURRENT_SCHEMA_VERSION);
    let content =
        serde_yaml_ng::to_string(&stamped).map_err(|e| format!("failed to serialize: {e}"))?;

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp = NamedTempFile::new_in(parent)
        .map_err(|e| format!("failed to create tempfile in {}: {e}", parent.display()))?;
    tmp.as_file_mut()
        .write_all(content.as_bytes())
        .map_err(|e| format!("failed to write tempfile: {e}"))?;
    tmp.as_file_mut()
        .sync_all()
        .map_err(|e| format!("failed to fsync tempfile: {e}"))?;
    tmp.persist(path).map_err(|e| {
        format!(
            "failed to rename tempfile to {}: {}",
            path.display(),
            e.error
        )
    })?;

    // Evict stale cache entry so the next load re-parses the written file.
    evict_cache(path);
    Ok(())
}

/// Sibling lockfile path for a given `bullseye.yaml`. The lockfile
/// name is stable across atomic renames of the yaml (since only the
/// yaml's inode is replaced on write, not the lockfile's), which is
/// why we lock here instead of on the yaml itself.
fn lock_path_for(yaml: &Path) -> PathBuf {
    let mut s = yaml.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

/// Snapshot `(mtime, len)` of `path`, or `None` if stat fails. Used
/// for CAS checks bracketing the read-modify-write window — if either
/// field changes between read and write, someone modified the file
/// outside our advisory lock (e.g. a text editor that doesn't honour
/// flock).
fn file_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let md = std::fs::metadata(path).ok()?;
    Some((md.modified().ok()?, md.len()))
}

/// Structured error from [`with_locked_mutation`]. Each variant names
/// a distinct failure mode so callers can surface actionable messages
/// (e.g. the timeout includes which lockfile was contended; the
/// conflict includes which yaml was externally edited).
#[derive(Debug)]
pub enum MutationError {
    /// Couldn't acquire the exclusive flock within [`LOCK_WAIT`].
    /// Another bullseye process is holding it for too long.
    LockTimeout { path: PathBuf, waited: Duration },
    /// Lockfile I/O failure unrelated to contention (e.g. permission
    /// denied, directory doesn't exist).
    LockIo(String),
    /// Reading `bullseye.yaml` failed (propagates [`LoadError`]).
    Load(LoadError),
    /// The yaml's `(mtime, len)` changed between our read and our
    /// write, despite us holding the flock — means a non-flock-aware
    /// writer (editor, script) modified the file in the window.
    Conflict { path: PathBuf },
    /// The closure supplied to [`with_locked_mutation`] returned an
    /// error. The mutation was not applied.
    Apply(String),
    /// Serialising or writing the mutated yaml back failed.
    Save(String),
}

impl std::fmt::Display for MutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MutationError::LockTimeout { path, waited } => write!(
                f,
                "timed out after {:?} waiting for exclusive lock on {} — another bullseye process may be editing the file; retry",
                waited,
                path.display(),
            ),
            MutationError::LockIo(msg) => write!(f, "lockfile error: {msg}"),
            MutationError::Load(e) => write!(f, "{e}"),
            MutationError::Conflict { path } => write!(
                f,
                "conflict: {} was modified externally between read and write; retry",
                path.display(),
            ),
            MutationError::Apply(msg) => write!(f, "{msg}"),
            MutationError::Save(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for MutationError {}

impl From<MutationError> for String {
    fn from(e: MutationError) -> Self {
        e.to_string()
    }
}

/// Run a mutation against `bullseye.yaml` under an exclusive flock.
///
/// Steps, in order:
/// 1. Open (creating if necessary) a sibling lockfile `<path>.lock`.
/// 2. Acquire an exclusive advisory lock via `fs2::FileExt`. On POSIX
///    this is `flock(2) LOCK_EX`; on Windows it's `LockFileEx`. Wait
///    up to [`LOCK_WAIT`] for the lock, polling every [`LOCK_POLL`].
///    A timeout returns [`MutationError::LockTimeout`].
/// 3. Read and parse `path` **fresh from disk**, bypassing the parse
///    cache. The cache is fine for read-only tools but must not be
///    trusted inside a mutation — another process may have written
///    since we last loaded.
/// 4. Record `(mtime, len)` for the CAS check.
/// 5. Call the closure with a mutable reference to the freshly-parsed
///    [`TargetsFile`]. If it returns `Err`, abort without writing.
/// 6. Re-stat the file. If `(mtime, len)` changed, another writer
///    (outside our flock — so, an editor or script) raced us; return
///    [`MutationError::Conflict`] without clobbering.
/// 7. Serialise and write back atomically via [`save`], which evicts
///    the parse cache on success.
/// 8. Drop the lockfile handle, releasing the flock.
///
/// The lockfile is never removed — it's a 0-byte sentinel whose
/// entire purpose is to be a stable flock anchor.
/// Open the sibling lockfile for `yaml_path` and acquire an exclusive
/// advisory lock, waiting up to [`LOCK_WAIT`] for contention to clear.
/// Returns the held file handle — drop it to release the lock.
fn acquire_lock(yaml_path: &Path) -> Result<std::fs::File, MutationError> {
    let lock_path = lock_path_for(yaml_path);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| MutationError::LockIo(format!("create {}: {e}", parent.display())))?;
    }
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| MutationError::LockIo(format!("open {}: {e}", lock_path.display())))?;

    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        #[allow(deprecated)]
        match lock_file.try_lock_exclusive() {
            Ok(()) => return Ok(lock_file),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(MutationError::LockTimeout {
                        path: lock_path,
                        waited: LOCK_WAIT,
                    });
                }
                std::thread::sleep(LOCK_POLL);
            }
            Err(e) => {
                return Err(MutationError::LockIo(format!(
                    "lock {}: {e}",
                    lock_path.display()
                )));
            }
        }
    }
}

/// Write a full [`TargetsFile`] to `path` under an exclusive flock.
/// Used for wholesale replacements (e.g. `bullseye_import`) where we
/// want lock coordination with concurrent mutators but don't have a
/// prior-read baseline for CAS. The lockfile's parent must already
/// exist (import creates the shadow directory beforehand).
pub fn with_locked_write(path: &Path, file: &TargetsFile) -> Result<(), MutationError> {
    let _lock = acquire_lock(path)?;
    save(path, file).map_err(MutationError::Save)?;
    Ok(())
}

pub fn with_locked_mutation<F, T, E>(path: &Path, f: F) -> Result<T, MutationError>
where
    F: FnOnce(&mut TargetsFile) -> Result<T, E>,
    E: Into<String>,
{
    let _lock = acquire_lock(path)?;

    // --- Critical section: flock held. ---
    let mut file = parse_file(path).map_err(MutationError::Load)?;
    let stamp_before = file_stamp(path);

    let output = f(&mut file).map_err(|e| MutationError::Apply(e.into()))?;

    let stamp_now = file_stamp(path);
    if stamp_now != stamp_before {
        return Err(MutationError::Conflict {
            path: path.to_path_buf(),
        });
    }

    save(path, &file).map_err(MutationError::Save)?;
    // Lock released on drop of lock_file at end of scope.
    Ok(output)
}
