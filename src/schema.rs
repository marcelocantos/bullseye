// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Top-level targets file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetsFile {
    /// Git SHA at which targets were last evaluated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluated: Option<String>,

    /// All targets keyed by ID (e.g., "T1", "T1.1").
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
}

/// A single convergence target.
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

    /// Parent target ID (e.g., "T1" for sub-target "T1.1").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,

    /// Targets this one enables (gating relationships).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GateEdge>,

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

/// A gating relationship: this target enables `target` with the given
/// criticality (fraction of the gated target's value that depends on
/// this gate). Defaults to 1.0 (hard prerequisite).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateEdge {
    pub target: String,
    #[serde(default = "default_criticality")]
    pub criticality: f64,
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

fn default_criticality() -> f64 {
    1.0
}

fn default_origin() -> String {
    "manual".to_string()
}

impl Target {
    /// Computed weight = value / cost (minimum 1).
    pub fn weight(&self) -> f64 {
        if self.cost > 0.0 {
            (self.value / self.cost).max(1.0)
        } else {
            self.value.max(1.0)
        }
    }

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
