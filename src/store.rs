// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use crate::schema::TargetsFile;

/// Discover the targets file by walking up from `start_dir`.
/// Checks `docs/targets.yaml`, then `targets.yaml` at each level.
/// Maximum directory levels to traverse upward when searching for targets.yaml.
const MAX_DISCOVER_DEPTH: usize = 64;

pub fn discover(start_dir: &Path) -> Option<PathBuf> {
    let mut dir = start_dir.to_path_buf();
    for _ in 0..MAX_DISCOVER_DEPTH {
        for candidate in &["docs/targets.yaml", "targets.yaml"] {
            let path = dir.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
    None
}

/// Load and parse a targets file.
pub fn load(path: &Path) -> Result<TargetsFile, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_yaml_ng::from_str(&content).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

/// Write a targets file back to disk.
pub fn save(path: &Path, file: &TargetsFile) -> Result<(), String> {
    let content = serde_yaml_ng::to_string(file).map_err(|e| format!("failed to serialize: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("failed to write {}: {e}", path.display()))
}
