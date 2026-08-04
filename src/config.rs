// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Per-repo storage resolution (v0.16.0+) and create-time defaults (🎯T61).
//!
//! **Discovery** looks in two places and uses whichever already exists:
//!
//! - **In-repo**: walk up from `cwd` looking for `bullseye.yaml`.
//! - **External (shadow tree)**: walk up from the shadow path
//!   `<external_root>/<absolute cwd>` looking for `bullseye.yaml`.
//!
//! If both exist, **in-repo wins**. Discovery never consults
//! [`default_location`].
//!
//! `external_root()` is `~/.local/share/bullseye/` by default, override
//! via `BULLSEYE_DATA_DIR` (primarily for tests/sandboxes). The shadow
//! tree mirrors the absolute `cwd` path — purely path-driven, no
//! git-remote or host/org/repo assumptions.
//!
//! **Creation** of a *new* `bullseye.yaml` resolves location as:
//! 1. Per-call `location` argument (overrides everything).
//! 2. Server default from [`default_location`] (`--default-location` /
//!    `BULLSEYE_DEFAULT_LOCATION`) — create paths only.
//! 3. Otherwise the locked [`LOCATION_PROMPT`].

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const DEFAULT_DATA_SUBDIR: &str = ".local/share/bullseye";

/// Env var for the server-level create default (🎯T61). Create-path only.
pub const DEFAULT_LOCATION_ENV: &str = "BULLSEYE_DEFAULT_LOCATION";

thread_local! {
    /// Thread-local override for [`external_root`]. Set in tests so
    /// each test thread sees its own shadow tree without racing on
    /// `std::env::set_var` (which Rust 2024 marks `unsafe` because it
    /// is not thread-safe). Production code never reads this — it
    /// falls through to the env-var / HOME default.
    static DATA_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };

    /// Thread-local override for [`default_location`] (🎯T61 tests).
    /// `None` = consult process / env; `Some(None)` = force no default;
    /// `Some(Some(loc))` = force that location.
    static DEFAULT_LOCATION_OVERRIDE: RefCell<Option<Option<Location>>> =
        const { RefCell::new(None) };
}

/// Process-level create default set from CLI `--default-location` at
/// server startup. Set at most once per process.
static PROCESS_DEFAULT_LOCATION: OnceLock<Location> = OnceLock::new();

/// Install a thread-local override for the external shadow root.
/// Tests call this at setup; production code never does.
pub fn set_external_root_override(dir: Option<PathBuf>) {
    DATA_DIR_OVERRIDE.with(|o| *o.borrow_mut() = dir);
}

/// Install a thread-local override for the create-time default location
/// (🎯T61). `None` clears the override; `Some(None)` forces "no default"
/// even if env/process would supply one; `Some(Some(loc))` forces `loc`.
pub fn set_default_location_override(loc: Option<Option<Location>>) {
    DEFAULT_LOCATION_OVERRIDE.with(|o| *o.borrow_mut() = loc);
}

/// Record the process-level create default from CLI
/// (`--default-location`). Idempotent: the first call wins (OnceLock).
/// Returns `Err` if `s` is not a valid location string.
pub fn set_process_default_location(s: &str) -> Result<(), String> {
    let loc = Location::parse(s)?;
    let _ = PROCESS_DEFAULT_LOCATION.set(loc);
    Ok(())
}

/// Where the first-time prompt is surfaced to the user. This wording
/// is returned verbatim by tools that need a location decision (today:
/// create tools when neither a per-call `location` nor a server default
/// is set, and read-side tools when no targets file exists under cwd).
pub const LOCATION_PROMPT: &str = "Create bullseye.yaml for this repo where?\n\
    - in_repo — commit bullseye.yaml into the repo (you own it, team uses bullseye).\n\
    - external — shadow tree under ~/.local/share/bullseye/ (read-only repo, or personal use of bullseye).\n\
    Call `bullseye_init` with `location: in_repo` or `location: external`.\n\
    Or set a server default: `--default-location external|in_repo` / env `BULLSEYE_DEFAULT_LOCATION`.";

/// Per-repo storage location. Supplied to create tools on first write;
/// discovery afterwards is automatic from filesystem state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    InRepo,
    External,
}

impl Location {
    pub fn as_str(&self) -> &'static str {
        match self {
            Location::InRepo => "in_repo",
            Location::External => "external",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "in_repo" => Ok(Location::InRepo),
            "external" => Ok(Location::External),
            other => Err(format!(
                "unknown location: {other} (use in_repo or external)"
            )),
        }
    }
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/Users/marcelo"))
}

/// External shadow root. Precedence: thread-local override (tests) →
/// `BULLSEYE_DATA_DIR` env var → `$HOME/.local/share/bullseye`.
pub fn external_root() -> PathBuf {
    if let Some(dir) = DATA_DIR_OVERRIDE.with(|o| o.borrow().clone()) {
        return dir;
    }
    if let Ok(dir) = std::env::var("BULLSEYE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    home_dir().join(DEFAULT_DATA_SUBDIR)
}

/// Server-level default for **creating** a new `bullseye.yaml` only
/// (🎯T61). Never consulted by discovery.
///
/// Precedence: thread-local override (tests) → process CLI
/// (`set_process_default_location`) → env [`DEFAULT_LOCATION_ENV`] →
/// `None` (caller must supply `location` or surface [`LOCATION_PROMPT`]).
pub fn default_location() -> Option<Location> {
    if let Some(forced) = DEFAULT_LOCATION_OVERRIDE.with(|o| *o.borrow()) {
        return forced;
    }
    if let Some(loc) = PROCESS_DEFAULT_LOCATION.get() {
        return Some(*loc);
    }
    match std::env::var(DEFAULT_LOCATION_ENV) {
        Ok(s) => Location::parse(s.trim()).ok(),
        Err(_) => None,
    }
}

/// Resolve where to **create** a new targets file.
///
/// 1. Non-empty explicit `location` argument wins.
/// 2. Else [`default_location`] if set.
/// 3. Else error carrying [`LOCATION_PROMPT`].
pub fn resolve_create_location(explicit: Option<&str>) -> Result<Location, String> {
    match explicit.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Location::parse(s).map_err(|e| format!("{e}\n\n{LOCATION_PROMPT}")),
        None => default_location().ok_or_else(|| LOCATION_PROMPT.to_string()),
    }
}

/// Expand a leading `~/` in a user-supplied path. Anything else is
/// returned unchanged.
pub fn expand_tilde(path: &Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~") {
        home_dir().join(rest)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_parse() {
        assert_eq!(Location::parse("in_repo").unwrap(), Location::InRepo);
        assert_eq!(Location::parse("external").unwrap(), Location::External);
        assert!(Location::parse("").is_err());
        assert!(Location::parse("EXTERNAL").is_err());
    }

    #[test]
    fn tilde_expansion_identity_on_absolute() {
        let absolute = expand_tilde(Path::new("/explicit/path"));
        assert_eq!(absolute, PathBuf::from("/explicit/path"));
    }

    #[test]
    fn resolve_create_location_explicit_wins() {
        set_default_location_override(Some(Some(Location::External)));
        assert_eq!(
            resolve_create_location(Some("in_repo")).unwrap(),
            Location::InRepo
        );
        set_default_location_override(None);
    }

    #[test]
    fn resolve_create_location_uses_default() {
        set_default_location_override(Some(Some(Location::External)));
        assert_eq!(resolve_create_location(None).unwrap(), Location::External);
        set_default_location_override(None);
    }

    #[test]
    fn resolve_create_location_prompt_without_default() {
        set_default_location_override(Some(None));
        let err = resolve_create_location(None).unwrap_err();
        assert!(err.contains("Create bullseye.yaml"));
        set_default_location_override(None);
    }
}
