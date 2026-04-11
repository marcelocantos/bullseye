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

/// Upsert a target: create if it doesn't exist, patch if it does.
///
/// Unified replacement for the old add/update split. `id` is optional;
/// omit it to create a new target with an auto-assigned ID, provide it
/// to upsert at a specific ID (useful for sub-targets like T1.2). When
/// the target already exists, only the provided fields are changed.
#[mcp_tool(
    name = "bullseye_assert",
    description = "Upsert a target: create if the ID doesn't exist, patch if it does. \
        Omit `id` to create a new target with an auto-assigned ID. \
        Provide `id` (e.g., 'T1.2') to create a sub-target at a specific ID or to patch an existing one. \
        On create, `name`, `value`, `cost`, and `acceptance` are required; on patch, all fields are optional. \
        Use `depends_on` to declare this target's blockers, or `blocks` (sugar) to inject this target into other targets' depends_on lists — handy when adding a new prerequisite above existing work."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct AssertTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// Target ID. Omit to create a new target with an auto-assigned ID.
    /// Provide to upsert at a specific ID (creates if missing, patches if existing).
    #[serde(default)]
    pub id: Option<String>,

    /// Short assertion describing the desired state. Required on create.
    #[serde(default)]
    pub name: Option<String>,

    /// User-scored value (Fibonacci: 1, 2, 3, 5, 8, 13, 20). Required on create.
    #[serde(default)]
    pub value: Option<f64>,

    /// Agent-estimated cost (Fibonacci: 1, 2, 3, 5, 8, 13, 20). Required on create.
    #[serde(default)]
    pub cost: Option<f64>,

    /// Acceptance criteria — how to verify the target is achieved. Required on create.
    #[serde(default)]
    pub acceptance: Option<Vec<String>>,

    /// Why this target matters.
    #[serde(default)]
    pub context: Option<String>,

    /// Target kind: "work" (default) or "verify". Only settable on create.
    #[serde(default)]
    pub kind: Option<String>,

    /// Status: "identified", "converging", or "achieved".
    #[serde(default)]
    pub status: Option<String>,

    /// IDs of targets this one depends on (must be achieved first).
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,

    /// Sugar: IDs of targets that should gain this target as a dependency.
    /// The handler appends this target's ID to each listed target's `depends_on`.
    /// Lets you declare "I am a new prerequisite for X, Y" at creation time
    /// without a separate patch on X and Y.
    #[serde(default)]
    pub blocks: Option<Vec<String>>,

    /// For verify targets: IDs of upstream targets this verifies.
    #[serde(default)]
    pub verifies: Option<Vec<String>>,

    /// Origin description (default: "manual" on create; unchanged on patch).
    #[serde(default)]
    pub origin: Option<String>,

    /// Tags (e.g., ["visual"]).
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
    description = "Create a starter docs/targets.yaml with a sample target. Refuses to overwrite an existing file — use bullseye_assert for repos that already have targets."
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

/// Session startup context for a project.
#[mcp_tool(
    name = "bullseye_startup_context",
    description = "Return a concise startup context for the current project: active target count, frontier targets ready for work, recently achieved targets, and any warnings (tunnels, validation errors). Designed for agent consumption at session start — pair with mnemo_recent_activity for full session context."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct StartupContextTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// Number of days to look back for recently achieved targets (default: 14).
    #[serde(default)]
    pub recent_days: Option<u32>,
}

/// Cross-repo portfolio view.
#[mcp_tool(
    name = "bullseye_portfolio",
    description = "Discover all repos with targets under a workspace root and return a portfolio summary: per-repo active/frontier/achieved counts and frontier target names. Scans ~/work/ by default. Use this for cross-project prioritisation and global convergence assessment."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct PortfolioTool {
    /// Workspace root to scan (default: ~/work/).
    #[serde(default)]
    pub root: Option<String>,

    /// Maximum directory depth to scan (default: 5).
    #[serde(default)]
    pub max_depth: Option<u32>,
}

fn default_filter() -> String {
    "active".to_string()
}

/// A single entry in the `momentum` parameter of [`SummaryTool`].
///
/// Pair of `target_id → multiplier`. The parameter is a list of
/// these rather than a JSON object keyed by target ID because the
/// rust-mcp-sdk `JsonSchema` derive does not emit Draft 2020-12
/// compliant schema for keyed-map types (it falls back to
/// `type: "unknown"`, which the Anthropic API rejects on tool-list
/// submission). A list of objects with scalar fields always produces
/// valid schema.
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct MomentumEntry {
    /// Target ID the multiplier applies to (e.g. `"T1"`, `"T1.2"`).
    pub id: String,
    /// Multiplier applied to the target's WSJF score during ranking.
    /// `1.0` is identity (no boost), values `> 1.0` promote, values
    /// `< 1.0` suppress.
    pub multiplier: f64,
}

/// Consolidated status overview: grouped targets, frontier, blocked, stale, WSJF ranking.
#[mcp_tool(
    name = "bullseye_summary",
    description = "Return a consolidated status overview in one call: active targets grouped by parent with rollup counts, frontier (unblocked) targets, blocked targets with blockers, stale targets with inconsistent graph state, and top-N WSJF-ranked targets. \
        Optionally accepts a `momentum` list ([{id, multiplier}, ...]) that scales each target's WSJF score before ranking; this lets a caller fold recency/frequency from mnemo_recent_activity (or any external signal) into the ordering without bullseye having to know about the source. Targets missing from the list default to 1.0 (no boost). \
        Replaces multiple calls to list/frontier/validate for status assessment."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SummaryTool {
    /// Working directory to discover targets.yaml from.
    pub cwd: String,

    /// How many WSJF-ranked targets to include (default: 5).
    #[serde(default)]
    pub top_n: Option<u32>,

    /// Optional per-target momentum multipliers, as a list of
    /// `{id, multiplier}` entries. When provided, each target's WSJF
    /// score is multiplied by its listed multiplier (default 1.0 for
    /// targets not in the list) before ranking. Targets with higher
    /// multipliers rise; values below 1.0 suppress stale targets.
    /// The caller (typically the /cv skill) computes these values
    /// from `mnemo_recent_activity` or any other external signal —
    /// bullseye never calls mnemo directly, so composition happens
    /// at the skill layer. The multiplier formula itself is the
    /// caller's responsibility and is therefore tunable without
    /// touching bullseye. Duplicate `id` entries use the last
    /// multiplier seen.
    #[serde(default)]
    pub momentum: Option<Vec<MomentumEntry>>,
}

tool_box!(
    TargetTools,
    [
        ListTool,
        GetTool,
        AssertTool,
        RetireTool,
        FrontierTool,
        ReworkTool,
        TunnelsTool,
        ValidateTool,
        GraphTool,
        RenderTool,
        InitTool,
        ImportTool,
        StartupContextTool,
        PortfolioTool,
        SummaryTool
    ]
);
