// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Per-repo storage resolution (v0.16.0+).
//!
//! Bullseye no longer has a machine-wide storage-mode config. Instead,
//! discovery looks in two places and uses whichever already exists:
//!
//! - **In-repo**: walk up from `cwd` looking for `bullseye.yaml`.
//! - **External (shadow tree)**: walk up from the shadow path
//!   `<external_root>/<absolute cwd>` looking for `bullseye.yaml`.
//!
//! `external_root()` is `~/.local/share/bullseye/` by default, override
//! via `BULLSEYE_DATA_DIR` (primarily for tests/sandboxes). The shadow
//! tree mirrors the absolute `cwd` path — purely path-driven, no
//! git-remote or host/org/repo assumptions.
//!
//! Creation still needs an explicit choice between in_repo and external.
//! That choice is passed as a `location` argument to `bullseye_init` —
//! no config file is consulted. See [`Location`].

use std::cell::RefCell;
use std::path::{Path, PathBuf};

const DEFAULT_DATA_SUBDIR: &str = ".local/share/bullseye";

thread_local! {
    /// Thread-local override for [`external_root`]. Set in tests so
    /// each test thread sees its own shadow tree without racing on
    /// `std::env::set_var` (which Rust 2024 marks `unsafe` because it
    /// is not thread-safe). Production code never reads this — it
    /// falls through to the env-var / HOME default.
    static DATA_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Install a thread-local override for the external shadow root.
/// Tests call this at setup; production code never does.
pub fn set_external_root_override(dir: Option<PathBuf>) {
    DATA_DIR_OVERRIDE.with(|o| *o.borrow_mut() = dir);
}

/// Where the first-time prompt is surfaced to the user. This wording
/// is returned verbatim by tools that need a location decision (today:
/// `bullseye_init` when called without `location`, and read-side tools
/// when no targets file exists anywhere under the cwd).
pub const LOCATION_PROMPT: &str = "Create bullseye.yaml for this repo where?\n\
    - in_repo — commit bullseye.yaml into the repo (you own it, team uses bullseye).\n\
    - external — shadow tree under ~/.local/share/bullseye/ (read-only repo, or personal use of bullseye).\n\
    Call `bullseye_init` with `location: in_repo` or `location: external`.";

/// Per-repo storage location. Supplied to `bullseye_init` on first
/// creation; discovery afterwards is automatic from filesystem state,
/// so this value is never persisted in a separate config file.
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
}
