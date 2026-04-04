// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use rust_mcp_sdk::macros::{mcp_tool, JsonSchema};
use rust_mcp_sdk::tool_box;

/// List targets with optional filtering.
#[mcp_tool(
    name = "targets_list",
    description = "List convergence targets, sorted by weight (descending). Returns active targets by default."
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
    name = "targets_get",
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
    name = "targets_add",
    description = "Add a new convergence target. The server assigns the next available ID and validates the entry."
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

    /// Acceptance criteria — how to verify convergence.
    pub acceptance: Vec<String>,

    /// Why this target matters.
    #[serde(default)]
    pub context: String,

    /// Parent target ID (optional, for sub-targets).
    #[serde(default)]
    pub parent: Option<String>,

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
    name = "targets_update",
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
    name = "targets_retire",
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
    name = "targets_frontier",
    description = "Compute the frontier: active leaf targets with all dependencies satisfied. These are the targets that can be worked on right now, in parallel."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct FrontierTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,
}

/// Compute WSJF ranking with blocking analysis.
#[mcp_tool(
    name = "targets_rank",
    description = "Compute WSJF ranking of active targets with blocking analysis. Returns targets sorted by effective weight, split into unblocked and blocked."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RankTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,
}

/// Validate the targets file for schema conformance.
#[mcp_tool(
    name = "targets_validate",
    description = "Validate the targets file for schema conformance: ID format, references, cycles, required fields."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ValidateTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,
}

/// Generate a Mermaid dependency graph.
#[mcp_tool(
    name = "targets_graph",
    description = "Generate a Mermaid dependency graph of active targets showing parent/child, gating, and depends-on relationships."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GraphTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,
}

/// Render targets.md from the YAML source.
#[mcp_tool(
    name = "targets_render",
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

tool_box!(TargetTools, [
    ListTool,
    GetTool,
    AddTool,
    UpdateTool,
    RetireTool,
    FrontierTool,
    RankTool,
    ValidateTool,
    GraphTool,
    RenderTool
]);
