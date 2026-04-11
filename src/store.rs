// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use crate::schema::{TargetsFile, migrate_gates_to_depends_on};

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

/// Create an empty targets file at `start_dir/docs/targets.yaml`.
/// Creates the `docs/` directory if it doesn't exist.
pub fn create_default(start_dir: &Path) -> Result<PathBuf, String> {
    let docs = start_dir.join("docs");
    std::fs::create_dir_all(&docs)
        .map_err(|e| format!("failed to create {}: {e}", docs.display()))?;
    let path = docs.join("targets.yaml");
    let file = TargetsFile {
        last_evaluated: None,
        targets: Default::default(),
    };
    save(&path, &file)?;
    Ok(path)
}

/// Create a starter targets file with a sample target at `start_dir/docs/targets.yaml`.
/// Creates the `docs/` directory if it doesn't exist.
pub fn create_starter(start_dir: &Path, project_name: &str) -> Result<PathBuf, String> {
    let docs = start_dir.join("docs");
    std::fs::create_dir_all(&docs)
        .map_err(|e| format!("failed to create {}: {e}", docs.display()))?;
    let path = docs.join("targets.yaml");

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
            actual_cost: None,
            acceptance: vec![
                "First milestone is defined with measurable success criteria".to_string(),
                "At least one sub-target breaks the milestone into actionable work".to_string(),
            ],
            context: "Starter target — replace with your project's actual first goal.".to_string(),
            gates: Vec::new(),
            depends_on: Vec::new(),
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
        last_evaluated: None,
        targets,
    };
    save(&path, &file)?;
    Ok(path)
}

/// Load and parse a targets file.
///
/// Applies in-memory migration for legacy `gates` edges: they are folded
/// into `depends_on` on the gated target, then cleared. Every caller sees
/// a single-edge-type graph regardless of the on-disk format.
pub fn load(path: &Path) -> Result<TargetsFile, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let mut file: TargetsFile = serde_yaml_ng::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
    migrate_gates_to_depends_on(&mut file);
    Ok(file)
}

/// Write a targets file back to disk.
pub fn save(path: &Path, file: &TargetsFile) -> Result<(), String> {
    let content =
        serde_yaml_ng::to_string(file).map_err(|e| format!("failed to serialize: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("failed to write {}: {e}", path.display()))
}
