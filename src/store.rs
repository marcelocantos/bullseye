// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use crate::config::{Config, Mode};
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

/// Discover the targets file by walking up from `start_dir`.
/// Looks for `bullseye.yaml` at each level.
const MAX_DISCOVER_DEPTH: usize = 64;

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
/// Windows we treat a drive prefix the same way (strip the prefix,
/// join the rest).
fn shadow_path(root: &Path, cwd: &Path) -> PathBuf {
    let mut shadow = root.to_path_buf();
    for comp in cwd.components() {
        use std::path::Component;
        match comp {
            Component::RootDir | Component::Prefix(_) => {}
            Component::CurDir => {}
            Component::ParentDir => {
                // `..` in cwd shouldn't happen for an absolute canonical
                // path, but guard anyway — treat as no-op rather than
                // escaping upward out of root.
            }
            Component::Normal(part) => shadow.push(part),
        }
    }
    shadow
}

/// Shadow-mode discovery: walk up the shadow tree from
/// `shadow_path(root, cwd)` the same way [`discover`] walks up the
/// real tree. Stops at `root` — we never walk above it.
pub fn discover_external(root: &Path, cwd: &Path) -> Option<PathBuf> {
    let start = shadow_path(root, cwd);
    let mut dir = start;
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

/// Config-aware discovery. Returns `Some(path)` when a targets file
/// exists for `cwd` under the configured storage mode.
pub fn discover_with_config(cwd: &Path, cfg: &Config) -> Option<PathBuf> {
    match cfg.storage.mode {
        Mode::InRepo => discover(cwd),
        Mode::External => discover_external(&cfg.effective_root(), cwd),
    }
}

/// Target path for a *new* targets file for `cwd` under the configured
/// storage mode. Does not create the file — callers pair this with
/// [`save`] or [`create_default_with_config`].
pub fn target_path_for_new(cwd: &Path, cfg: &Config) -> PathBuf {
    match cfg.storage.mode {
        Mode::InRepo => cwd.join("bullseye.yaml"),
        Mode::External => shadow_path(&cfg.effective_root(), cwd).join("bullseye.yaml"),
    }
}

/// Config-aware companion to [`create_default`]. Creates parent dirs
/// under the configured root when in external mode.
pub fn create_default_with_config(cwd: &Path, cfg: &Config) -> Result<PathBuf, String> {
    let path = target_path_for_new(cwd, cfg);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let file = TargetsFile {
        schema_version: Some(CURRENT_SCHEMA_VERSION),
        last_evaluated: None,
        targets: Default::default(),
    };
    save(&path, &file)?;
    Ok(path)
}

/// Config-aware companion to [`create_starter`].
pub fn create_starter_with_config(
    cwd: &Path,
    cfg: &Config,
    project_name: &str,
) -> Result<PathBuf, String> {
    let path = target_path_for_new(cwd, cfg);
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

/// Create an empty targets file at `start_dir/bullseye.yaml` (in-repo
/// mode). Prefer [`create_default_with_config`] in new code.
pub fn create_default(start_dir: &Path) -> Result<PathBuf, String> {
    let path = start_dir.join("bullseye.yaml");
    let file = TargetsFile {
        schema_version: Some(CURRENT_SCHEMA_VERSION),
        last_evaluated: None,
        targets: Default::default(),
    };
    save(&path, &file)?;
    Ok(path)
}

/// Create a starter targets file at `start_dir/bullseye.yaml` (in-repo
/// mode). Prefer [`create_starter_with_config`] in new code.
pub fn create_starter(start_dir: &Path, project_name: &str) -> Result<PathBuf, String> {
    let path = start_dir.join("bullseye.yaml");
    write_starter_file(&path, project_name)?;
    Ok(path)
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
