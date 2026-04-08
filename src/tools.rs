// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::tool_box;

/// List targets with optional filtering.
#[mcp_tool(
    name = "bullseye_list",
    description = "List targets. Returns active targets by default."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// Filter: "active" (default), "achieved", or "all".
    #[serde(default = "default_filter")]
    pub filter: String,
}

/// Get a single target by ID.
#[mcp_tool(
    name = "bullseye_get",
    description = "Get a single target by ID (e.g., 'T1', 'T1.2'). Returns full detail including acceptance criteria and graph relationships."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// Target ID (e.g., "T1", "T1.2").
    pub id: String,
}

/// Add a new target.
#[mcp_tool(
    name = "bullseye_add",
    description = "Add a new target. The server assigns the next available ID and validates the entry."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct AddTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// Short assertion describing the desired state.
    pub name: String,

    /// User-scored value (Fibonacci: 1, 2, 3, 5, 8, 13, 20).
    pub value: f64,

    /// Agent-estimated cost (Fibonacci: 1, 2, 3, 5, 8, 13, 20).
    pub cost: f64,

    /// Acceptance criteria — how to verify the target is achieved.
    pub acceptance: Vec<String>,

    /// Why this target matters.
    #[serde(default)]
    pub context: String,

    /// Target kind: "work" (default) or "verify".
    #[serde(default)]
    pub kind: Option<String>,

    /// For verify targets: IDs of upstream targets this verifies.
    #[serde(default)]
    pub verifies: Vec<String>,

    /// Origin description (default: "manual").
    #[serde(default = "default_origin")]
    pub origin: String,

    /// Tags (e.g., ["visual"]).
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Update fields on an existing target.
#[mcp_tool(
    name = "bullseye_update",
    description = "Update one or more fields on an existing target. Only provided fields are changed."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct UpdateTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// Target ID to update.
    pub id: String,

    /// New status (identified, converging, achieved).
    #[serde(default)]
    pub status: Option<String>,

    /// New value score.
    #[serde(default)]
    pub value: Option<f64>,

    /// New cost estimate.
    #[serde(default)]
    pub cost: Option<f64>,

    /// New name/assertion.
    #[serde(default)]
    pub name: Option<String>,

    /// Replace acceptance criteria.
    #[serde(default)]
    pub acceptance: Option<Vec<String>>,

    /// Replace context.
    #[serde(default)]
    pub context: Option<String>,

    /// Replace tags.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Retire a target (move to achieved).
#[mcp_tool(
    name = "bullseye_retire",
    description = "Retire a target by marking it achieved with today's date. Optionally records actual cost for calibration."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RetireTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// Target ID to retire.
    pub id: String,

    /// Actual cost (Fibonacci scale) for calibration against the estimate.
    #[serde(default)]
    pub actual_cost: Option<f64>,
}

/// Compute the frontier: unblocked targets ready for work.
#[mcp_tool(
    name = "bullseye_frontier",
    description = "Compute the frontier: active leaf targets with all dependencies satisfied. These are the targets that can be worked on right now, in parallel."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct FrontierTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,
}

/// Trigger a rework cycle from a verify target.
#[mcp_tool(
    name = "bullseye_rework",
    description = "Trigger rework from a failed verification. Resets the rework target to converging, increments its retry count, and resets the verify target to identified. Returns an escalation warning if the retry budget is exhausted."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ReworkTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// The verify target ID that failed.
    pub id: String,

    /// Diagnosis: what went wrong (carried forward as context).
    #[serde(default)]
    pub diagnosis: String,
}

/// Detect tunnels: work targets far from verification.
#[mcp_tool(
    name = "bullseye_tunnels",
    description = "Detect tunnels: active work targets that have no verification checkpoint within 2 hops. Suggests where to insert verify targets to prevent agent drift."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct TunnelsTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// Maximum hops before flagging (default: 2).
    #[serde(default)]
    pub max_depth: Option<u32>,
}

/// Validate the targets file for schema conformance.
#[mcp_tool(
    name = "bullseye_validate",
    description = "Validate the targets file for schema conformance: ID format, references, cycles, required fields."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ValidateTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,
}

/// Generate a Mermaid dependency graph.
#[mcp_tool(
    name = "bullseye_graph",
    description = "Generate a Mermaid dependency graph of active targets showing parent/child, gating, and depends-on relationships."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GraphTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,
}

/// Initialise a new targets file with a starter template.
#[mcp_tool(
    name = "bullseye_init",
    description = "Create a starter docs/targets.yaml with a sample target. Refuses to overwrite an existing file — use bullseye_add for repos that already have targets."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct InitTool {
    /// Working directory (project root) where docs/targets.yaml will be created.
    pub cwd: String,

    /// Project name for the sample target context (e.g., "my-app").
    #[serde(default)]
    pub project_name: Option<String>,
}

/// Import targets from a markdown file into YAML.
#[mcp_tool(
    name = "bullseye_import",
    description = "Import targets from a markdown targets file (docs/targets.md) into docs/targets.yaml. Parses the markdown format produced by render.rs and other repos' /cv skills. Tolerant of minor formatting variations. Refuses to overwrite an existing targets.yaml — use this for initial migration only."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ImportTool {
    /// Working directory (project root) where docs/targets.md will be read.
    pub cwd: String,

    /// Path to the markdown file to import (default: docs/targets.md discovered by walking up from cwd).
    #[serde(default)]
    pub path: Option<String>,

    /// If true, overwrite an existing targets.yaml (default: false).
    #[serde(default)]
    pub force: bool,
}

/// Render targets.md from the YAML source.
#[mcp_tool(
    name = "bullseye_render",
    description = "Render docs/targets.md from docs/targets.yaml. The YAML is the source of truth; the markdown is a human-readable view. Mutation tools (add, update, retire) auto-render, so this is only needed for manual re-rendering."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RenderTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,
}

fn default_filter() -> String {
    "active".to_string()
}

fn default_origin() -> String {
    "manual".to_string()
}

tool_box!(
    TargetTools,
    [
        ListTool,
        GetTool,
        AddTool,
        UpdateTool,
        RetireTool,
        FrontierTool,
        ReworkTool,
        TunnelsTool,
        ValidateTool,
        GraphTool,
        RenderTool,
        InitTool,
        ImportTool
    ]
);
