// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Current schema version written by this build of bullseye.
///
/// Bumped whenever a breaking schema change lands (new required field,
/// edge-type rework, semantic change to an existing field). Callers
/// that need format stability should read this constant instead of
/// hard-coding the number.
///
/// Versions in the wild:
/// - `None` on disk → legacy pre-v0.9.0 file; treated as v1 on load.
/// - `Some(1)` → v0.9.0+ format: single `depends_on` edge type,
///   `verifies`/`rework` edges, optional `momentum` parameter on
///   `bullseye_summary`. The gates-field and parent-field migrations
///   from v0.4.0 and v0.8.0 continue to run transparently on load,
///   so older on-disk files are still accepted.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Top-level targets file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetsFile {
    /// Schema version. Written by bullseye on every save so that an
    /// older binary loading a file produced by a newer binary can
    /// detect the mismatch and refuse to proceed rather than
    /// silently misinterpreting new fields. Absent on legacy files
    /// from before v0.9.0; those are treated as v1 at load time and
    /// upgraded in place on the next save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,

    /// Git SHA at which targets were last evaluated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluated: Option<String>,

    /// All targets keyed by ID (e.g., "T1", "T1.1").
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
}

/// A single target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    /// Short assertion describing the desired state.
    pub name: String,

    /// Target kind: work (default) or verify.
    #[serde(default, skip_serializing_if = "is_work")]
    pub kind: Kind,

    /// Current status.
    pub status: Status,

    /// User-scored value on Fibonacci scale (1, 2, 3, 5, 8, 13, 20).
    /// Required for leaf targets. Interior targets derive value from
    /// the graph, but the file stores the computed result.
    pub value: f64,

    /// Agent-estimated cost on Fibonacci scale.
    pub cost: f64,

    /// Actual cost recorded on retirement, for calibration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_cost: Option<f64>,

    /// How to verify the desired state is achieved.
    pub acceptance: Vec<String>,

    /// Why this target matters, how it was discovered.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub context: String,

    /// Transient field: legacy `gates` edges read from old targets files.
    /// Always empty after [`migrate_gates_to_depends_on`] runs; never serialized.
    /// Retained only so old YAMLs deserialize without error.
    #[serde(default, skip_serializing)]
    pub gates: Vec<LegacyGateEdge>,

    /// Targets that must be achieved before work on this one begins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,

    /// For verify targets: which upstream targets this verifies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verifies: Vec<String>,

    /// For verify targets: the upstream target to re-enter on failure.
    /// When verification fails, the rework target is reset to converging
    /// and the verify target carries forward a diagnosis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rework: Option<String>,

    /// Maximum number of rework cycles before escalation.
    /// Only meaningful on targets that are rework destinations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_budget: Option<u32>,

    /// Current retry count (incremented each time rework re-enters this target).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retries: u32,

    /// Freeform tags (e.g., "visual").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// How this target was created.
    #[serde(default = "default_origin")]
    pub origin: String,

    /// Date the target was discovered.
    pub discovered: NaiveDate,

    /// Date the target was achieved (filled on retirement).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub achieved: Option<NaiveDate>,
}

/// Legacy gate edge from older targets files. The edge is upstream→downstream
/// (A gates B means A enables B), which is the inverse of `depends_on`.
/// Criticality was a soft-blocking weight that no downstream logic ever
/// consumed; it is discarded on migration.
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyGateEdge {
    pub target: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub criticality: Option<f64>,
}

/// Fold legacy `gates` edges into `depends_on` on the field-owning target.
///
/// Although the docstring on the old field said "targets this one enables",
/// real-world data across every repo that used it treated `Gates: 🎯X` as
/// **"I am gated by X"** — the natural English reading. Verified against
/// concrete examples (e.g. pairdroid T4 "installable via Homebrew" had
/// `Gates: T1, T3`, which can only mean T4 depends on T1 and T3, since
/// homebrew installation can't possibly be a prerequisite of the app code
/// it packages). Under that reading, `t.gates = [X]` is equivalent to
/// `t.depends_on += [X]`, with the criticality weight discarded.
///
/// Called from [`crate::store::load`] and from markdown import, so every
/// in-memory `TargetsFile` the rest of the codebase sees has `gates` empty.
pub fn migrate_gates_to_depends_on(file: &mut TargetsFile) {
    for target in file.targets.values_mut() {
        let gates = std::mem::take(&mut target.gates);
        for gate in gates {
            if !target.depends_on.contains(&gate.target) {
                target.depends_on.push(gate.target);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Identified,
    Converging,
    Achieved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Kind {
    #[default]
    Work,
    Verify,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

fn is_work(kind: &Kind) -> bool {
    *kind == Kind::Work
}

fn default_origin() -> String {
    "manual".to_string()
}

impl Target {
    /// Whether this target is active (not achieved).
    pub fn is_active(&self) -> bool {
        self.status != Status::Achieved
    }
}

impl TargetsFile {
    /// Active targets only.
    pub fn active(&self) -> BTreeMap<&str, &Target> {
        self.targets
            .iter()
            .filter(|(_, t)| t.is_active())
            .map(|(id, t)| (id.as_str(), t))
            .collect()
    }

    /// Achieved targets only.
    pub fn achieved(&self) -> BTreeMap<&str, &Target> {
        self.targets
            .iter()
            .filter(|(_, t)| !t.is_active())
            .map(|(id, t)| (id.as_str(), t))
            .collect()
    }
}
