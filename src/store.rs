// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use crate::config::{Location, external_root};
use crate::schema::{CURRENT_SCHEMA_VERSION, TargetsFile, migrate_gates_to_depends_on};

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

/// Load and parse a targets file.
///
/// Applies in-memory migration for legacy `gates` edges: they are folded
/// into `depends_on` on the gated target, then cleared. Every caller sees
/// a single-edge-type graph regardless of the on-disk format.
///
/// Enforces schema-version compatibility: if the file declares a
/// `schema_version` greater than [`CURRENT_SCHEMA_VERSION`], loading fails
/// with [`LoadError::VersionTooNew`] rather than silently misinterpreting
/// fields the current binary does not know about. Files without a
/// `schema_version` field are accepted as legacy v1 and the field is
/// filled in so the next save stamps it.
///
/// Errors are returned as a typed enum so callers can discriminate —
/// speculative callers like `bullseye_startup_context` tolerate I/O and
/// parse errors, while every caller must surface [`LoadError::VersionTooNew`].
pub fn load(path: &Path) -> Result<TargetsFile, LoadError> {
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

/// Write a targets file back to disk.
///
/// Always stamps `schema_version = CURRENT_SCHEMA_VERSION` on the
/// serialized output so that every bullseye-produced file is
/// self-describing. Callers that held an older (or missing) version
/// in memory will see it rewritten on save.
pub fn save(path: &Path, file: &TargetsFile) -> Result<(), String> {
    let mut stamped = file.clone();
    stamped.schema_version = Some(CURRENT_SCHEMA_VERSION);
    let content =
        serde_yaml_ng::to_string(&stamped).map_err(|e| format!("failed to serialize: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("failed to write {}: {e}", path.display()))
}
