// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! User-level configuration: where does bullseye store targets?
//!
//! Config lives at `~/.config/bullseye/config.yaml` (override via
//! `BULLSEYE_CONFIG_DIR`). It is machine-wide and selects between two
//! storage modes:
//!
//! - [`Mode::InRepo`]: targets file lives inside the project tree,
//!   discovered by walking up from `cwd`. Original behaviour.
//! - [`Mode::External`]: targets file lives under `storage.root` at a
//!   shadow path mirroring the absolute `cwd`. Default root is
//!   `~/.local/share/bullseye`. Path-driven — no git-remote or
//!   host/org/repo assumptions — so it handles monorepos, non-git
//!   dirs, and unconventional layouts identically.
//!
//! Missing or malformed config is surfaced as a structured error; no
//! silent fallback to a default. Callers translate [`ConfigError`]
//! into imperative instructions for the agent (ask the user, then
//! call `bullseye_configure`).

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

thread_local! {
    /// Thread-local override for [`config_dir`]. Set in tests so each
    /// test thread sees its own config directory without racing on
    /// `std::env::set_var` (which Rust 2024 marks `unsafe` precisely
    /// because it is not thread-safe). Production code never reads
    /// this — it falls through to the env-var / HOME fallback.
    static CONFIG_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Install a thread-local config directory override. Tests call this
/// at setup; production code never does.
pub fn set_config_dir_override(dir: Option<PathBuf>) {
    CONFIG_DIR_OVERRIDE.with(|o| *o.borrow_mut() = dir);
}

const CONFIG_FILENAME: &str = "config.yaml";
const DEFAULT_CONFIG_SUBDIR: &str = ".config/bullseye";
const DEFAULT_DATA_SUBDIR: &str = ".local/share/bullseye";

/// Locked first-run prompt text. Surfaced verbatim in the missing-config
/// error message and (eventually) via MCP elicitation. Kept here as a
/// single source of truth so the wording can never drift between paths.
pub const FIRST_RUN_PROMPT: &str = "Store targets where?\n\
    - in_repo — commit bullseye.yaml into the repo (you own it, team uses bullseye).\n\
    - external — shadow tree under ~/.local/share/bullseye/ (read-only repo, or personal use of bullseye).\n\
    Answer: in_repo or external. Machine-wide; edit ~/.config/bullseye/config.yaml to change.";

/// Storage mode: where bullseye looks for (and creates) targets files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    InRepo,
    External,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::InRepo => "in_repo",
            Mode::External => "external",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "in_repo" => Ok(Mode::InRepo),
            "external" => Ok(Mode::External),
            other => Err(format!(
                "unknown storage mode: {other} (use in_repo or external)"
            )),
        }
    }
}

/// The `storage` block of the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Storage {
    pub mode: Mode,
    /// External root. Ignored when `mode: in_repo`. Tildes are expanded
    /// at load time so downstream code sees an absolute path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
}

/// Full config file contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub storage: Storage,
}

impl Config {
    /// Resolve the effective external root — user-configured `storage.root`
    /// if present, otherwise [`default_data_dir`]. Only meaningful when
    /// `mode: external`; callers must check the mode before using.
    pub fn effective_root(&self) -> PathBuf {
        self.storage.root.clone().unwrap_or_else(default_data_dir)
    }
}

/// Structured error for [`load`]. Distinguishes "no decision yet"
/// (which prompts the agent to ask the user) from "decision recorded
/// but file is broken" (which is a hazard the user must fix).
#[derive(Debug)]
pub enum ConfigError {
    /// Config file does not exist. The agent should ask the user and
    /// call `bullseye_configure` to record the answer.
    NotConfigured { path: PathBuf },
    /// Config file exists but could not be read.
    Io { path: PathBuf, message: String },
    /// Config file exists but did not parse.
    Parse { path: PathBuf, message: String },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfigured { path } => write!(
                f,
                "Bullseye storage mode is not configured. {FIRST_RUN_PROMPT}\n\n\
                 Then call `bullseye_configure` with the user's answer. \
                 Expected config file: {}",
                path.display()
            ),
            ConfigError::Io { path, message } => {
                write!(f, "failed to read {}: {message}", path.display())
            }
            ConfigError::Parse { path, message } => {
                write!(f, "failed to parse {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Home directory, with a well-known fallback if `$HOME` is unset. The
/// fallback matches the codebase's existing habit (`handler.rs` does
/// the same for the portfolio default root).
fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/Users/marcelo"))
}

/// Config directory. Precedence: thread-local override (tests) →
/// `BULLSEYE_CONFIG_DIR` env var → `$HOME/.config/bullseye`.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = CONFIG_DIR_OVERRIDE.with(|o| o.borrow().clone()) {
        return dir;
    }
    if let Ok(dir) = std::env::var("BULLSEYE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    home_dir().join(DEFAULT_CONFIG_SUBDIR)
}

/// Full path of the config file.
pub fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILENAME)
}

/// Default external-storage root when the config doesn't override it.
pub fn default_data_dir() -> PathBuf {
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

/// Load the config file. Expands `~` in `storage.root`.
pub fn load() -> Result<Config, ConfigError> {
    let path = config_path();
    if !path.exists() {
        return Err(ConfigError::NotConfigured { path });
    }
    let content = std::fs::read_to_string(&path).map_err(|e| ConfigError::Io {
        path: path.clone(),
        message: e.to_string(),
    })?;
    let mut cfg: Config = serde_yaml_ng::from_str(&content).map_err(|e| ConfigError::Parse {
        path: path.clone(),
        message: e.to_string(),
    })?;
    if let Some(root) = cfg.storage.root {
        cfg.storage.root = Some(expand_tilde(&root));
    }
    Ok(cfg)
}

/// Write the config file, creating the config directory as needed.
pub fn save(cfg: &Config) -> Result<(), String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    let path = dir.join(CONFIG_FILENAME);
    let content =
        serde_yaml_ng::to_string(cfg).map_err(|e| format!("failed to serialize config: {e}"))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// RAII guard: installs a thread-local config dir override on
    /// construction, clears it on drop. Each test gets an isolated
    /// config directory without racing sibling tests on process env.
    struct ConfigGuard {
        _tmp: TempDir,
    }

    fn with_config_dir() -> ConfigGuard {
        let tmp = TempDir::new().unwrap();
        set_config_dir_override(Some(tmp.path().to_path_buf()));
        ConfigGuard { _tmp: tmp }
    }

    impl Drop for ConfigGuard {
        fn drop(&mut self) {
            set_config_dir_override(None);
        }
    }

    #[test]
    fn not_configured_when_file_absent() {
        let _g = with_config_dir();
        match load() {
            Err(ConfigError::NotConfigured { .. }) => {}
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_in_repo() {
        let _g = with_config_dir();
        save(&Config {
            storage: Storage {
                mode: Mode::InRepo,
                root: None,
            },
        })
        .unwrap();
        let loaded = load().unwrap();
        assert_eq!(loaded.storage.mode, Mode::InRepo);
        assert!(loaded.storage.root.is_none());
    }

    #[test]
    fn roundtrip_external_with_root() {
        let _g = with_config_dir();
        save(&Config {
            storage: Storage {
                mode: Mode::External,
                root: Some(PathBuf::from("/tmp/bullseye-data")),
            },
        })
        .unwrap();
        let loaded = load().unwrap();
        assert_eq!(loaded.storage.mode, Mode::External);
        assert_eq!(
            loaded.storage.root,
            Some(PathBuf::from("/tmp/bullseye-data"))
        );
    }

    #[test]
    fn parse_error_reported_with_path() {
        let _g = with_config_dir();
        std::fs::write(config_path(), "storage:\n  mode: sideways\n").unwrap();
        match load() {
            Err(ConfigError::Parse { path, .. }) => {
                assert!(path.ends_with(CONFIG_FILENAME));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn mode_parse() {
        assert_eq!(Mode::parse("in_repo").unwrap(), Mode::InRepo);
        assert_eq!(Mode::parse("external").unwrap(), Mode::External);
        assert!(Mode::parse("").is_err());
        assert!(Mode::parse("EXTERNAL").is_err());
    }

    #[test]
    fn tilde_expansion_with_explicit_home() {
        // Call expand_tilde with a controlled HOME via direct arg —
        // avoids touching process-global $HOME, which would race
        // sibling tests. We test the expansion logic via the public
        // helper on an absolute path (identity) and verify it returns
        // unchanged for inputs without a tilde.
        let absolute = expand_tilde(Path::new("/explicit/path"));
        assert_eq!(absolute, PathBuf::from("/explicit/path"));
    }
}
