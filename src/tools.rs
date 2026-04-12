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
    /// Working directory to discover bullseye.yaml from.
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
    /// Working directory to discover bullseye.yaml from.
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
///
/// Named `put` after the HTTP idiom — the verb is familiar to anyone
/// who has worked with REST APIs and carries the right semantics
/// (idempotent create-or-replace). The pre-v0.12.0 name was `assert`,
/// which was confusing because in most programming contexts "assert"
/// means "verify a condition, crash if false" rather than "create or
/// update a resource". The upsert-style verb was the right idea;
/// "assert" was the wrong word for it.
#[mcp_tool(
    name = "bullseye_put",
    description = "Upsert a target: create if the ID doesn't exist, patch if it does. \
        Omit `id` to create a new target with an auto-assigned ID. \
        Provide `id` (e.g., 'T1.2') to create a sub-target at a specific ID or to patch an existing one. \
        On create, `name`, `value`, `cost`, and `acceptance` are required; on patch, all fields are optional. \
        Use `depends_on` to declare this target's blockers, or `blocks` (sugar) to inject this target into other targets' depends_on lists — handy when adding a new prerequisite above existing work."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct PutTool {
    /// Working directory to discover bullseye.yaml from.
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

    /// Mark a work target as *observable* — its completion produces
    /// something the human decision-maker can look at, acting as a
    /// checkpoint for repo-level prioritisation. Verify-kind targets
    /// are observable automatically; this flag only matters for
    /// work-kind targets. Defaults to unchanged on patch; omit (or
    /// pass `false`) when observability is not a property of the
    /// target. See 🎯T7 and `docs/mcp-triad.md` §9.
    #[serde(default)]
    pub observable: Option<bool>,

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
    /// Working directory to discover bullseye.yaml from.
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
    /// Working directory to discover bullseye.yaml from.
    pub cwd: String,
}

/// Trigger a rework cycle from a verify target.
#[mcp_tool(
    name = "bullseye_rework",
    description = "Trigger rework from a failed verification. Resets the rework target to converging, increments its retry count, and resets the verify target to identified. Returns an escalation warning if the retry budget is exhausted."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ReworkTool {
    /// Working directory to discover bullseye.yaml from.
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
    /// Working directory to discover bullseye.yaml from.
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
    /// Working directory to discover bullseye.yaml from.
    pub cwd: String,
}

/// Generate a Mermaid dependency graph.
#[mcp_tool(
    name = "bullseye_graph",
    description = "Generate a Mermaid dependency graph of active targets showing parent/child, gating, and depends-on relationships."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GraphTool {
    /// Working directory to discover bullseye.yaml from.
    pub cwd: String,
}

/// Initialise a new targets file with a starter template.
#[mcp_tool(
    name = "bullseye_init",
    description = "Create a starter bullseye.yaml with a sample target. Refuses to overwrite an existing file — use bullseye_put for repos that already have targets."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct InitTool {
    /// Working directory (project root) where bullseye.yaml will be created.
    pub cwd: String,

    /// Project name for the sample target context (e.g., "my-app").
    #[serde(default)]
    pub project_name: Option<String>,
}

/// Import targets from a markdown file into YAML.
#[mcp_tool(
    name = "bullseye_import",
    description = "Import targets from a markdown file into bullseye.yaml. Parses the markdown format produced by other repos' /cv skills. Tolerant of minor formatting variations. Refuses to overwrite an existing bullseye.yaml — use this for initial migration only. Requires an explicit `path` to the markdown source."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ImportTool {
    /// Working directory (project root).
    pub cwd: String,

    /// Path to the markdown file to import (required).
    #[serde(default)]
    pub path: Option<String>,

    /// If true, overwrite an existing bullseye.yaml (default: false).
    #[serde(default)]
    pub force: bool,
}

/// Session startup context for a project.
#[mcp_tool(
    name = "bullseye_startup_context",
    description = "Return a concise startup context for the current project: active target count, frontier targets ready for work, recently achieved targets, and any warnings (tunnels, validation errors). Designed for agent consumption at session start — pair with mnemo_recent_activity for full session context."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct StartupContextTool {
    /// Working directory to discover bullseye.yaml from.
    pub cwd: String,

    /// Number of days to look back for recently achieved targets (default: 14).
    #[serde(default)]
    pub recent_days: Option<u32>,
}

/// Cross-repo portfolio view with WSJF ranking.
#[mcp_tool(
    name = "bullseye_portfolio",
    description = "Discover all repos with targets under a workspace root and return a portfolio \
        summary ranked by aggregate WSJF score. Per-repo score: \
        sum(value_i / cost_i × momentum_i × enabler_boost_i) / frontier_size. \
        Enabler boost propagates downstream target value across repos via cross_enables edges. \
        Optionally accepts a `momentum` list ([{id, multiplier}, ...]) that scales per-target \
        WSJF contributions; caller supplies the multipliers (e.g. from mnemo_recent_activity). \
        Scans ~/work/ by default. Use this for cross-project prioritisation and global convergence assessment."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct PortfolioTool {
    /// Workspace root to scan (default: ~/work/).
    #[serde(default)]
    pub root: Option<String>,

    /// Maximum directory depth to scan (default: 5).
    #[serde(default)]
    pub max_depth: Option<u32>,

    /// Optional per-target momentum multipliers, as a list of
    /// `{id, multiplier}` entries. When provided, each frontier
    /// target's WSJF contribution is scaled by its listed multiplier
    /// (default 1.0 for targets not in the list). The caller
    /// (typically the /cv skill) computes these values from
    /// `mnemo_recent_activity` or any other external signal —
    /// bullseye never calls mnemo directly.
    #[serde(default)]
    pub momentum: Option<Vec<MomentumEntry>>,
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

/// Consolidated status overview: grouped targets, focus-ordered frontier, blocked, stale.
#[mcp_tool(
    name = "bullseye_summary",
    description = "Return a consolidated status overview in one call: active targets grouped by parent with rollup counts, frontier (unblocked) targets ordered by focus (value × momentum), blocked targets with blockers, and stale targets with inconsistent graph state. \
        Optionally accepts a `momentum` list ([{id, multiplier}, ...]) that scales each target's value before sorting the frontier; caller supplies the multipliers (e.g. from mnemo_recent_activity). Targets missing from the list default to 1.0 (no boost). \
        `frontier_details: true` expands each frontier entry with its full acceptance criteria, context, and edges — useful when you would otherwise round-trip `bullseye_get` on every frontier target. \
        Replaces multiple calls to list/frontier/validate/get for status assessment."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SummaryTool {
    /// Working directory to discover bullseye.yaml from.
    pub cwd: String,

    /// Optional per-target momentum multipliers, as a list of
    /// `{id, multiplier}` entries. When provided, each frontier
    /// target's value is multiplied by its listed multiplier (default
    /// 1.0 for targets not in the list) before sorting, so
    /// recently-active targets rise and stale ones sink. Targets with
    /// higher multipliers rank higher; values below 1.0 suppress.
    /// The caller (typically the /cv skill) computes these values
    /// from `mnemo_recent_activity` or any other external signal —
    /// bullseye never calls mnemo directly, so composition happens
    /// at the skill layer. Duplicate `id` entries use the last
    /// multiplier seen.
    #[serde(default)]
    pub momentum: Option<Vec<MomentumEntry>>,

    /// When true, expand each frontier entry with its full detail:
    /// acceptance criteria, context, depends_on, verifies, rework,
    /// and tags. Useful for callers that would otherwise round-trip
    /// `bullseye_get` on every frontier target. Default: false.
    #[serde(default)]
    pub frontier_details: Option<bool>,
}

/// Build an executable verification plan for a target's `checks`.
#[mcp_tool(
    name = "bullseye_verify",
    description = "Build an executable verification plan for a target's declared `checks`. \
        Bullseye does not call sawmill itself — MCP servers can't call each other — so this tool \
        returns a *plan*: an ordered list of sawmill tool invocations (check_conventions, query, \
        check_invariants) plus a report template the caller populates with pass/fail outcomes and \
        file/line-level failure detail. The agent (or the /cv skill) runs each planned check \
        against the sawmill MCP server, fills in the template, and presents the final report. \
        Errors if the target does not exist or has no `checks` field populated."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct VerifyTool {
    /// Working directory to discover bullseye.yaml from.
    pub cwd: String,

    /// Target ID (e.g., "T1", "T1.2") whose checks should be planned.
    pub id: String,
}

/// End-to-end convergence evaluation: invariants + unreleased fixes + targets + recommendation.
#[mcp_tool(
    name = "bullseye_convergence",
    description = "Answer \"what's the next most-valuable thing to work on?\" in a single tool call. \
        Runs the project's `make bullseye` (or `mk bullseye`) rule to check standing invariants, \
        scans git for unreleased bug-fix commits since the last tag, emits the full target summary \
        with frontier details inline, and computes a deterministic next-action recommendation \
        (\"Execute now\" / \"Blocked\"). \
        Consolidates the old /cv worker's many round-trips into one call. Requires a `bullseye` \
        target in Makefile or mkfile; returns setup instructions if missing."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ConvergenceTool {
    /// Working directory to discover bullseye.yaml and the build file from.
    pub cwd: String,

    /// Optional per-target momentum multipliers (same shape as
    /// `bullseye_summary`). Caller computes these from external signal;
    /// bullseye never calls mnemo directly.
    #[serde(default)]
    pub momentum: Option<Vec<MomentumEntry>>,

    /// When true, skip the `make bullseye` / `mk bullseye` invocation
    /// and omit the invariants check. Useful for a lightweight scan
    /// that just reports the target graph state. Default: false.
    #[serde(default)]
    pub skip_invariants: Option<bool>,
}

tool_box!(
    TargetTools,
    [
        ListTool,
        GetTool,
        PutTool,
        RetireTool,
        FrontierTool,
        ReworkTool,
        TunnelsTool,
        ValidateTool,
        GraphTool,
        InitTool,
        ImportTool,
        StartupContextTool,
        PortfolioTool,
        SummaryTool,
        VerifyTool,
        ConvergenceTool
    ]
);
