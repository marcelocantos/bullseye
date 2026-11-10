// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Process startup configuration. Defaults < YAML file < environment < CLI.
//! Loading is explicit at executable startup; library/test callers never read a personal file.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::config::Location;

pub const DEFAULT_ADDR: &str = "127.0.0.1:18743";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSettings {
    pub addr: Option<String>,
    pub t3_plugin: Option<bool>,
    pub data_dir: Option<PathBuf>,
    pub default_location: Option<String>,
    pub commit_mode: Option<bool>,
    pub integration_ref: Option<String>,
    pub cache_ttl_secs: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Overrides {
    pub config: Option<PathBuf>,
    pub no_config: bool,
    pub values: FileSettings,
}

#[derive(Debug)]
pub struct Settings {
    pub file: Option<PathBuf>,
    pub addr: String,
    pub t3_plugin: bool,
    pub data_dir: PathBuf,
    pub default_location: Option<Location>,
    pub commit_mode: bool,
    pub integration_ref: String,
    pub cache_ttl_secs: u64,
}

static PROCESS_SETTINGS: OnceLock<Settings> = OnceLock::new();

pub fn current() -> Option<&'static Settings> {
    PROCESS_SETTINGS.get()
}

pub fn install(settings: Settings) -> Result<(), String> {
    PROCESS_SETTINGS
        .set(settings)
        .map_err(|_| "startup settings already installed".into())
}

fn boolean(label: &str, value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "" | "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("{label}: expected true/false or 1/0")),
    }
}

/// Peel startup flags before a subcommand, or anywhere in `serve`'s argument list.
/// Never scan another command's values: e.g. a target's context can contain `--config`.
pub fn peel_args(
    args: &[String],
    overrides: &mut Overrides,
    prefix_only: bool,
) -> Result<Vec<String>, String> {
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let (flag, inline) = args[i]
            .split_once('=')
            .map_or((args[i].as_str(), None), |(k, v)| (k, Some(v)));
        match flag {
            "--no-config" | "--no-t3-plugin" | "--no-commit-mode" => {
                if inline.is_some() {
                    return Err(format!("{flag} takes no value"));
                }
                match flag {
                    "--no-config" => overrides.no_config = true,
                    "--no-t3-plugin" => overrides.values.t3_plugin = Some(false),
                    _ => overrides.values.commit_mode = Some(false),
                }
            }
            "--t3-plugin" | "--commit-mode" => {
                let value = inline
                    .map(|v| boolean(flag, v))
                    .transpose()?
                    .unwrap_or(true);
                if flag == "--t3-plugin" {
                    overrides.values.t3_plugin = Some(value);
                } else {
                    overrides.values.commit_mode = Some(value);
                }
            }
            "--config" | "--addr" | "--data-dir" | "--default-location" | "--integration-ref"
            | "--cache-ttl-secs" => {
                let value = if let Some(value) = inline {
                    value
                } else {
                    i += 1;
                    args.get(i)
                        .filter(|v| !v.starts_with("--"))
                        .map(String::as_str)
                        .ok_or_else(|| format!("{flag} requires a value"))?
                };
                if value.is_empty() {
                    return Err(format!("{flag} requires a non-empty value"));
                }
                match flag {
                    "--config" => overrides.config = Some(value.into()),
                    "--addr" => overrides.values.addr = Some(value.into()),
                    "--data-dir" => overrides.values.data_dir = Some(value.into()),
                    "--default-location" => overrides.values.default_location = Some(value.into()),
                    "--integration-ref" => overrides.values.integration_ref = Some(value.into()),
                    _ => {
                        overrides.values.cache_ttl_secs = Some(
                            value
                                .parse()
                                .map_err(|_| format!("{flag} expects a non-negative integer"))?,
                        )
                    }
                }
            }
            _ if prefix_only => {
                rest.extend_from_slice(&args[i..]);
                break;
            }
            _ => rest.push(args[i].clone()),
        }
        i += 1;
    }
    if overrides.no_config && overrides.config.is_some() {
        return Err("--config and --no-config are mutually exclusive".into());
    }
    Ok(rest)
}

fn path_from(value: &Path, base: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    if value.as_os_str().is_empty() {
        return Err("configuration path must not be empty".into());
    }
    let expanded = if value == Path::new("~") || value.starts_with("~/") {
        home.ok_or("HOME is required to expand ~ in a configuration path")?
            .join(value.strip_prefix("~").expect("tilde prefix"))
    } else {
        value.to_path_buf()
    };
    Ok(if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    })
}

/// Resolve with an injected environment for deterministic tests without process-global mutation.
pub fn resolve(
    overrides: Overrides,
    env: impl Fn(&str) -> Option<String>,
    cwd: &Path,
) -> Result<Settings, String> {
    let home = env("HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
    let explicit = overrides
        .config
        .clone()
        .or_else(|| env("BULLSEYE_CONFIG").map(PathBuf::from));
    let selected = if overrides.no_config {
        None
    } else if let Some(path) = explicit.as_ref() {
        Some(path_from(path, cwd, home.as_deref())?)
    } else {
        env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|h| h.join(".config")))
            .map(|root| root.join("bullseye/config.yaml"))
    };
    let mut file = FileSettings::default();
    let mut loaded = None;
    if let Some(path) = selected {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                file = serde_yaml_ng::from_str::<Option<FileSettings>>(&text)
                    .map_err(|e| {
                        format!("{}: invalid Bullseye configuration: {e}", path.display())
                    })?
                    .unwrap_or_default();
                loaded = Some(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && explicit.is_none() => {}
            Err(e) => {
                return Err(format!(
                    "cannot read Bullseye configuration {}: {e}",
                    path.display()
                ));
            }
        }
    }
    let cli = overrides.values;
    let choose =
        |cli: Option<String>, key: &str, file: Option<String>| cli.or_else(|| env(key)).or(file);
    let choose_bool = |cli: Option<bool>, key: &str, file: Option<bool>| -> Result<bool, String> {
        if let Some(v) = cli {
            return Ok(v);
        }
        env(key)
            .map(|v| boolean(key, &v))
            .transpose()
            .map(|v| v.or(file).unwrap_or(false))
    };
    let t3_plugin = choose_bool(cli.t3_plugin, "BULLSEYE_T3_PLUGIN", file.t3_plugin)?;
    let commit_mode = choose_bool(cli.commit_mode, "BULLSEYE_COMMIT", file.commit_mode)?;
    let addr = choose(cli.addr, "BULLSEYE_ADDR", file.addr).unwrap_or_else(|| DEFAULT_ADDR.into());
    let (host, _) = parse_addr(&addr)?;
    if t3_plugin {
        crate::t3_plugin::require_loopback(host)?;
    }
    let default_location = choose(
        cli.default_location,
        crate::config::DEFAULT_LOCATION_ENV,
        file.default_location,
    )
    .map(|value| Location::parse(value.trim()).map_err(|e| format!("default_location: {e}")))
    .transpose()?;
    let integration_ref = choose(
        cli.integration_ref,
        crate::commit_mode::INTEGRATION_REF_ENV,
        file.integration_ref,
    )
    .unwrap_or_else(|| crate::commit_mode::DEFAULT_REF.into());
    crate::commit_mode::integration_ref_allowed(&integration_ref)?;
    let cache_ttl_secs = if let Some(value) = cli.cache_ttl_secs {
        value
    } else if let Some(value) = env(crate::cache::TTL_ENV) {
        value
            .trim()
            .parse()
            .map_err(|_| format!("{} expects a non-negative integer", crate::cache::TTL_ENV))?
    } else {
        file.cache_ttl_secs.unwrap_or(crate::cache::TTL.as_secs())
    };
    // File-relative data directories are anchored to the file, never the daemon's working directory.
    let data_dir = if let Some(value) = cli.data_dir {
        path_from(&value, cwd, home.as_deref())?
    } else if let Some(value) = env("BULLSEYE_DATA_DIR") {
        path_from(Path::new(&value), cwd, home.as_deref())?
    } else if let Some(value) = file.data_dir {
        path_from(
            &value,
            loaded.as_ref().and_then(|p| p.parent()).unwrap_or(cwd),
            home.as_deref(),
        )?
    } else {
        home.as_ref()
            .ok_or("HOME or an explicit data_dir is required")?
            .join(".local/share/bullseye")
    };
    Ok(Settings {
        file: loaded,
        addr,
        t3_plugin,
        data_dir,
        default_location,
        commit_mode,
        integration_ref,
        cache_ttl_secs,
    })
}

pub fn load(overrides: Overrides) -> Result<Settings, String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine working directory: {e}"))?;
    resolve(overrides, |key| std::env::var(key).ok(), &cwd)
}

pub fn parse_addr(addr: &str) -> Result<(&str, u16), String> {
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| format!("addr expects HOST:PORT, got `{addr}`"))?;
    if host.is_empty()
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-_:[]".contains(&c))
    {
        return Err(format!("addr expects HOST:PORT, got `{addr}`"));
    }
    let port = port
        .parse()
        .map_err(|_| format!("addr expects HOST:PORT, got `{addr}` (invalid port)"))?;
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn every_setting_uses_file_then_env_then_cli_including_false_and_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = root.join(".config/bullseye/config.yaml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "addr: 127.0.0.1:18111\nt3_plugin: true\ndata_dir: ./file-data\ndefault_location: external\ncommit_mode: true\nintegration_ref: refs/heads/file\ncache_ttl_secs: 99\n").unwrap();
        let mut env = BTreeMap::from([("HOME", root.display().to_string())]);
        let settings = resolve(Overrides::default(), |key| env.get(key).cloned(), root).unwrap();
        assert_eq!(settings.file, Some(path.clone()));
        assert_eq!(settings.addr, "127.0.0.1:18111");
        assert!(settings.t3_plugin && settings.commit_mode);
        assert_eq!(
            settings.data_dir,
            path.parent().unwrap().join("./file-data")
        );
        assert_eq!(settings.default_location, Some(Location::External));
        assert_eq!(settings.integration_ref, "refs/heads/file");
        assert_eq!(settings.cache_ttl_secs, 99);
        for (key, value) in [
            ("BULLSEYE_ADDR", "127.0.0.1:18222"),
            ("BULLSEYE_T3_PLUGIN", "0"),
            ("BULLSEYE_DATA_DIR", "env-data"),
            ("BULLSEYE_DEFAULT_LOCATION", " in_repo "),
            ("BULLSEYE_COMMIT", "false"),
            ("BULLSEYE_INTEGRATION_REF", "refs/heads/env"),
            ("BULLSEYE_CACHE_TTL_SECS", "0"),
        ] {
            env.insert(key, value.into());
        }
        let settings = resolve(Overrides::default(), |key| env.get(key).cloned(), root).unwrap();
        assert_eq!(settings.addr, "127.0.0.1:18222");
        assert!(!settings.t3_plugin && !settings.commit_mode);
        assert_eq!(settings.data_dir, root.join("env-data"));
        assert_eq!(settings.default_location, Some(Location::InRepo));
        assert_eq!(settings.integration_ref, "refs/heads/env");
        assert_eq!(settings.cache_ttl_secs, 0);
        let cli = Overrides {
            values: FileSettings {
                addr: Some("127.0.0.1:18333".into()),
                t3_plugin: Some(true),
                data_dir: Some("cli-data".into()),
                default_location: Some("external".into()),
                commit_mode: Some(true),
                integration_ref: Some("refs/heads/cli".into()),
                cache_ttl_secs: Some(4),
            },
            ..Default::default()
        };
        let settings = resolve(cli, |key| env.get(key).cloned(), root).unwrap();
        assert_eq!(settings.addr, "127.0.0.1:18333");
        assert!(settings.t3_plugin && settings.commit_mode);
        assert_eq!(settings.data_dir, root.join("cli-data"));
        assert_eq!(settings.default_location, Some(Location::External));
        assert_eq!(settings.integration_ref, "refs/heads/cli");
        assert_eq!(settings.cache_ttl_secs, 4);
    }

    #[test]
    fn selection_and_errors_are_explicit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let file = root.join("selected.yaml");
        let env = |key: &str| match key {
            "HOME" => Some(root.display().to_string()),
            "BULLSEYE_CONFIG" => Some(file.display().to_string()),
            _ => None,
        };
        assert!(
            resolve(Overrides::default(), env, root)
                .unwrap_err()
                .contains("cannot read")
        );
        for content in ["t3_plugn: true\n", "t3_plugin: [oops]\n", "data_dir: [\n"] {
            std::fs::write(&file, content).unwrap();
            let error = resolve(Overrides::default(), env, root).unwrap_err();
            assert!(
                error.contains("selected.yaml") && error.contains("invalid Bullseye configuration"),
                "{error}"
            );
        }
        assert!(
            resolve(
                Overrides {
                    no_config: true,
                    ..Default::default()
                },
                env,
                root
            )
            .is_ok()
        );
        std::fs::write(&file, "").unwrap();
        assert!(resolve(Overrides::default(), env, root).is_ok());
        std::fs::write(&file, "addr: 0.0.0.0:18111\nt3_plugin: true\n").unwrap();
        assert!(resolve(Overrides::default(), env, root).is_err());
        assert!(
            resolve(
                Overrides {
                    values: FileSettings {
                        t3_plugin: Some(false),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                env,
                root
            )
            .is_ok()
        );
    }

    #[test]
    fn xdg_selection_and_relative_tilde_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let xdg = root.join("custom");
        std::fs::create_dir_all(xdg.join("bullseye")).unwrap();
        std::fs::write(xdg.join("bullseye/config.yaml"), "data_dir: ~/ledgers\n").unwrap();
        let env = |key: &str| match key {
            "HOME" => Some(root.display().to_string()),
            "XDG_CONFIG_HOME" => Some(xdg.display().to_string()),
            _ => None,
        };
        let result = resolve(Overrides::default(), env, root).unwrap();
        assert_eq!(result.data_dir, root.join("ledgers"));
        assert_eq!(result.file, Some(xdg.join("bullseye/config.yaml")));
        let result = resolve(
            Overrides::default(),
            |key| match key {
                "XDG_CONFIG_HOME" => Some("relative-ignored".into()),
                _ => env(key),
            },
            root,
        )
        .unwrap();
        assert!(result.file.is_none());
        assert_eq!(result.addr, DEFAULT_ADDR);
        let result = resolve(
            Overrides {
                config: Some("custom/bullseye/config.yaml".into()),
                ..Default::default()
            },
            env,
            root,
        )
        .unwrap();
        assert_eq!(result.data_dir, root.join("ledgers"));
    }

    #[test]
    fn startup_flags_preserve_command_values_and_support_negative_overrides() {
        let strings = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut overrides = Overrides::default();
        let rest = peel_args(
            &strings(&[
                "--config=x.yaml",
                "--commit-mode",
                "commit",
                "--context",
                "--no-config",
            ]),
            &mut overrides,
            true,
        )
        .unwrap();
        assert_eq!(rest, strings(&["commit", "--context", "--no-config"]));
        assert!(!overrides.no_config);
        assert_eq!(overrides.values.commit_mode, Some(true));
        peel_args(
            &strings(&[
                "--commit-mode=false",
                "--no-t3-plugin",
                "--addr",
                "localhost:18111",
            ]),
            &mut overrides,
            false,
        )
        .unwrap();
        assert_eq!(overrides.values.commit_mode, Some(false));
        assert_eq!(overrides.values.t3_plugin, Some(false));
        for args in [
            vec!["--config"],
            vec!["--addr", "--no-config"],
            vec!["--cache-ttl-secs=-1"],
            vec!["--no-config=false"],
            vec!["--config=x", "--no-config"],
        ] {
            assert!(peel_args(&strings(&args), &mut Overrides::default(), false).is_err());
        }
    }
}
