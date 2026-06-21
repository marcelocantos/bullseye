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

    /// Filter: "active" (default), "achieved", "set_aside", or "all".
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
        On create, `name` and `acceptance` are required; all other fields are optional. \
        `value` and `cost` are portfolio-scope metadata used for cross-repo WSJF ranking (weekly-plus horizon) — \
        they are NOT consumed by repo-level frontier ordering, which uses `depends_on` shape (unblocking fanout) only. \
        Omit value/cost when working at repo scope; supply them only when you also intend to participate in \
        cross-repo portfolio prioritisation and have a meaningful estimate. \
        Targets are uniform — there is no `kind` discriminator. Whether the pass signal comes from CI, a human \
        ack, or a smoke test is described in the acceptance criteria themselves (free text), not encoded as a \
        node type. On patch, all fields are optional. \
        Use `depends_on` to declare this target's blockers, or `blocks` (sugar) to inject this target into other \
        targets' depends_on lists — handy when adding a new prerequisite above existing work. \
        \
        Before filing a target whose acceptance reads as multi-phase prose (\"do X, then Y, then check Z\"), \
        check `docs/shapes.md` in this repo for the named graph-shape patterns (diamond, fan-out, chain, \
        choke-point, spike-then-decide, contract-first, migration) and propose the decomposition before \
        committing to a single node. The mistake to avoid is encoding a subgraph in one node's prose."
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

    /// Portfolio-scope: user-scored value (Fibonacci: 1, 2, 3, 5, 8, 13, 20).
    /// Consumed by cross-repo WSJF ranking only. Not used by repo-level
    /// frontier ordering (which uses checkpoint distance instead). Omit
    /// when working at repo scope; 0.0 is stored and means "not set".
    #[serde(default)]
    pub value: Option<f64>,

    /// Portfolio-scope: agent-estimated cost (Fibonacci: 1, 2, 3, 5, 8, 13, 20).
    /// Consumed by cross-repo WSJF ranking only. Not used by repo-level
    /// frontier ordering (which uses checkpoint distance instead). Omit
    /// when working at repo scope; 0.0 is stored and means "not set".
    #[serde(default)]
    pub cost: Option<f64>,

    /// Acceptance criteria — how to verify the target is achieved. Required on create.
    #[serde(default)]
    pub acceptance: Option<Vec<String>>,

    /// Why this target matters.
    #[serde(default)]
    pub context: Option<String>,

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

/// Set a target aside (parked / deferred / wont_fix) with a documented
/// rationale. Distinct from retirement — the target is not delivered,
/// but it is removed from the active set and unblocks its dependents
/// just like an achieved target. The free-text reason carries the
/// nuance (parked indefinitely, deferred to a later milestone,
/// rejected outright); the schema deliberately doesn't fix a taxonomy.
/// See 🎯T18.
#[mcp_tool(
    name = "bullseye_set_aside",
    description = "Set a target aside (parked / deferred / wont_fix) with a required rationale. \
        The status moves to `set_aside`, the reason is recorded on the target, and the target is \
        removed from the active set / frontier — its dependents are unblocked the same way an \
        achieved target unblocks them. Use this instead of `bullseye_retire` when the target was \
        not delivered: parked indefinitely, deferred to a later milestone, or actively rejected \
        as won't-fix. The reason is free text — it should explain the disposition (e.g. \"parked \
        pending design discussion in 🎯T42\", \"deferred to v2.0\", \"won't fix — superseded by \
        🎯T57's redesign\"). Distinct from `bullseye_retire`, which is achievement-only."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SetAsideTool {
    /// Working directory to discover bullseye.yaml from.
    pub cwd: String,

    /// Target ID to set aside.
    pub id: String,

    /// Rationale for the disposition. Required and must be non-empty
    /// after trimming. Carries the parked / deferred / wont_fix
    /// nuance the schema doesn't taxonomise.
    pub reason: String,
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

/// Re-open a previously-retired target.
#[mcp_tool(
    name = "bullseye_revert",
    description = "Re-open a previously-retired target: status moves Achieved → Converging, the \
        achieved date is cleared, and the supplied reason is appended to the target's context with \
        today's date so the audit trail survives in-place. Use when a retirement turns out to have \
        been wrong: regression detected, an acceptance criterion was missed, or external evidence \
        contradicts the achievement. Refuses to revert a target that is not currently achieved — \
        to resume a set-aside target use `bullseye_put` with `status: identified`; to move an active \
        target backwards, patch its status directly."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RevertTool {
    /// Working directory to discover bullseye.yaml from.
    pub cwd: String,

    /// Target ID to revert.
    pub id: String,

    /// Reason for the revert — what changed since retirement. Appended
    /// to the target's context as a `Reverted YYYY-MM-DD: <reason>`
    /// line so the audit trail survives. Required and must be
    /// non-empty after trimming.
    pub reason: String,
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
    description = "Create a starter bullseye.yaml with a sample target. \
        Requires a `location` argument that is either \"in_repo\" (file is \
        committed into the repo, for repos you own where the team uses bullseye) \
        or \"external\" (file is stored in a shadow tree under \
        ~/.local/share/bullseye/ mirroring the cwd's absolute path — for \
        read-only repos, or personal use of bullseye). Refuses to create a \
        file if one already exists at either location."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct InitTool {
    /// Working directory (project root) where bullseye.yaml will be
    /// created (in-repo location) or whose absolute path will be
    /// mirrored under the external shadow root.
    pub cwd: String,

    /// Where to store the new bullseye.yaml: "in_repo" or "external".
    /// Ask the user if you don't know. This is a per-repo choice, not
    /// a machine-wide one — pick it once per repo at init time.
    pub location: String,

    /// Project name for the sample target context (e.g., "my-app").
    #[serde(default)]
    pub project_name: Option<String>,
}

/// Import targets from a markdown file into YAML.
#[mcp_tool(
    name = "bullseye_import",
    description = "Import targets from a markdown file into bullseye.yaml. Parses \
        the markdown format produced by other repos' /cv skills. Tolerant of minor \
        formatting variations. Requires `path` (the markdown source) and \
        `location` (\"in_repo\" or \"external\" — same semantics as bullseye_init). \
        Refuses to overwrite an existing bullseye.yaml in either location unless \
        `force: true`."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ImportTool {
    /// Working directory (project root).
    pub cwd: String,

    /// Path to the markdown file to import (required).
    #[serde(default)]
    pub path: Option<String>,

    /// Where to write the resulting bullseye.yaml: "in_repo" or "external".
    /// Same semantics as bullseye_init's location field.
    pub location: String,

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

/// One new child target in a `bullseye_subdivide` call.
///
/// `id` is optional — when omitted the child is auto-assigned the next
/// free sub-target slot under the parent (e.g. parent `T15` → `T15.1`,
/// `T15.2`, …). Supply `id` explicitly to place the child at a
/// top-level slot or at a specific sub-target ID.
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SubdivisionChild {
    /// Optional explicit target ID. Omit to auto-assign as the next
    /// sub-target of the parent.
    #[serde(default)]
    pub id: Option<String>,

    /// Short assertion describing the child's desired state.
    pub name: String,

    /// Acceptance criteria — how to verify the child is achieved.
    pub acceptance: Vec<String>,

    /// Optional context paragraph carrying the why / discovery story.
    #[serde(default)]
    pub context: Option<String>,

    /// Optional tags.
    #[serde(default)]
    pub tags: Option<Vec<String>>,

    /// Optional dependencies the child blocks on, in addition to any
    /// implicit edges added by the subdivide mode.
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
}

/// Split a parent target into one or more children in a single call,
/// with one of three dependent-rewiring modes.
#[mcp_tool(
    name = "bullseye_subdivide",
    description = "Split a parent target into one or more children in a single call, rewiring \
        the parent's existing dependents according to `mode`. Use this when work inside a target \
        proves bigger than scoped and you want to spawn sub-work without losing the dependency \
        edges that point at the parent. \
        \
        See `docs/shapes.md` for the named graph-shape patterns (diamond, fan-out, chain, \
        choke-point, spike-then-decide, contract-first, migration) and which subdivide call \
        shape produces each. Diamond decomposition in particular leans on `retire` + `tail` — \
        the diamond's tail is the convergence node, and dependents should rewire to that single \
        node rather than the whole subgraph. \
        \
        Modes (safest default first): \
        - `add`: parent untouched; every existing dependent of the parent gains the new children \
          as additional `depends_on` entries alongside the parent. Strictly tightens the graph, \
          destroys no information. \
        - `aggregate`: parent becomes a converging umbrella — each new child is appended to the \
          parent's own `depends_on`, and the parent moves to `converging` if previously \
          `identified`. Dependents continue to depend on the parent only; the parent retires once \
          all its children retire. \
        - `retire`: parent transitions to `achieved` with today's date (its content is preserved); \
          every dependent's `depends_on` is rewired by replacing the parent ID with the new child \
          IDs (or with `tail`, when supplied — see below). Use when the parent's original \
          acceptance is met and the new children carry spillover work the original scope didn't \
          anticipate, or to reshape into a named pattern (diamond, contract-first, …). \
        \
        Each child requires `name` and `acceptance`. Child IDs default to auto-assigned \
        sub-target slots of the parent (e.g. parent `T15` → `T15.1`, `T15.2`). Supply explicit \
        `id` values on children you intend to reference from `tail` or from other children's \
        `depends_on`. Refuses to operate on terminal parents (achieved or set_aside) — revert or \
        re-open them first. \
        \
        `retire_reason` is optional and only consumed in `retire` mode; when supplied it's \
        appended to the parent's context as a `Subdivided YYYY-MM-DD: <reason>` audit line. \
        \
        `tail` is optional and only consumed in `retire` mode. When omitted, dependents are \
        rewired to depend on every new child (functionally redundant but noisy when the children \
        form a connected subgraph). When supplied, dependents are rewired to depend on only the \
        listed tail children — the load-bearing case for diamond decomposition. Tail entries must \
        match the explicit `id` of one of the children in this call."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SubdivideTool {
    /// Working directory to discover bullseye.yaml from.
    pub cwd: String,

    /// ID of the parent target to subdivide. Must exist and must not
    /// be in a terminal status (achieved / set_aside).
    pub parent: String,

    /// `add` (default, safest), `aggregate`, or `retire`. See the tool
    /// description for full semantics.
    pub mode: String,

    /// One or more new child targets to create. Must be non-empty.
    pub children: Vec<SubdivisionChild>,

    /// Optional rationale appended to the parent's context as
    /// `Subdivided YYYY-MM-DD: <reason>`. Only consumed in `retire`
    /// mode; ignored otherwise.
    #[serde(default)]
    pub retire_reason: Option<String>,

    /// Optional subset of child IDs that should receive the rewired
    /// dependent edges in `retire` mode. When omitted, every new
    /// child receives the dependents (current default). When
    /// supplied, only the listed children do — useful for shapes
    /// whose tail node converges multiple internal branches
    /// (diamond, contract-first integration node, migration verify
    /// node). Tail entries must match a child's explicit `id`.
    /// Rejected outside `retire` mode.
    #[serde(default)]
    pub tail: Option<Vec<String>>,
}

/// Resolve a partial repo reference to an absolute repo root (🎯T29).
#[mcp_tool(
    name = "bullseye_resolve",
    description = "Resolve a partial repo reference to an absolute path so subsequent tool calls \
        can pass it as `cwd` without the agent eyeballing portfolio output. \
        \
        Accepts: a leaf name (e.g. `spyder`), an `org/repo` fragment (e.g. \
        `marcelocantos/spyder`), or a full host+org+repo path. Absolute paths pass through \
        after a sanity check that `bullseye.yaml` exists there. \
        \
        Matching is suffix-on-path-components against the same workspace scan \
        `bullseye_portfolio` uses (default `~/work/`, override with `workspace_root`). \
        Ambiguous references — a reference matching two or more repos — return a \
        structured error listing every candidate; bullseye never silently picks one. \
        Zero matches return a not-found error suggesting `bullseye_portfolio` for the \
        directory of available repos. \
        \
        Scan results are memoised per-process, so repeated calls in a session don't \
        re-walk the workspace."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ResolveTool {
    /// Repo reference to resolve. Leaf name, partial `org/repo`,
    /// `host/org/repo`, or absolute path.
    pub reference: String,

    /// Workspace root to scan (default: `~/work/`). Same default as
    /// `bullseye_portfolio`.
    #[serde(default)]
    pub workspace_root: Option<String>,
}

/// Default `horizon` for `bullseye_sync_priorities`.
fn default_horizon() -> String {
    "today".to_string()
}

/// Default directory walk depth for `bullseye_sync_priorities`.
fn default_max_depth() -> u32 {
    5
}

/// Mirror GitHub issues into bullseye targets and reflect target
/// lifecycle back to issues, via the `gh` CLI. The MCP-surface twin of
/// the `bullseye github sync` subcommand — every capability is reachable
/// from both surfaces.
#[mcp_tool(
    name = "bullseye_github_sync",
    description = "Mirror GitHub issues into bullseye targets and reflect target lifecycle back to issues, using the gh CLI (authentication is the caller's existing gh session — no token is stored). Open issues become GHI-<n> targets (the issue number is the ID); achieving/setting-aside/reverting a mirrored target closes/reopens its issue. The MCP twin of `bullseye github sync`."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GithubSyncTool {
    /// Working directory to discover bullseye.yaml from and run gh in.
    pub cwd: String,

    /// Repo "owner/name" to sync (default: the checkout at cwd).
    #[serde(default)]
    pub repo: Option<String>,

    /// Only mirror issues carrying this label.
    #[serde(default)]
    pub label: Option<String>,

    /// Only mirror issues assigned to this user ("@me" for the authed user).
    #[serde(default)]
    pub assignee: Option<String>,

    /// Mirror issues in only; do not push target lifecycle back to GitHub.
    #[serde(default)]
    pub pull_only: bool,

    /// Push target lifecycle only; do not mirror issues in.
    #[serde(default)]
    pub push_only: bool,

    /// Show what would change without writing or calling gh mutators.
    #[serde(default)]
    pub dry_run: bool,
}

/// Scan the workspace and upsert the portfolio frontier into a SQLite
/// table. The MCP-surface twin of the `bullseye sync-priorities`
/// subcommand.
#[mcp_tool(
    name = "bullseye_sync_priorities",
    description = "Scan the workspace for repos with bullseye.yaml, compute the portfolio frontier, and upsert each frontier target into a SQLite `targets_priorities` table (stale rows pruned in the same transaction). The MCP twin of `bullseye sync-priorities`."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SyncPrioritiesTool {
    /// SQLite path (default: $BULLSEYE_DATA_DIR/priorities.db).
    #[serde(default)]
    pub db: Option<String>,

    /// Workspace root to scan (default: ~/work).
    #[serde(default)]
    pub root: Option<String>,

    /// Value written to the `horizon` column (default: "today").
    #[serde(default = "default_horizon")]
    pub horizon: String,

    /// Directory walk depth (default: 5).
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
}

tool_box!(
    TargetTools,
    [
        ListTool,
        GetTool,
        PutTool,
        RetireTool,
        RevertTool,
        SetAsideTool,
        SubdivideTool,
        FrontierTool,
        ValidateTool,
        GraphTool,
        InitTool,
        ImportTool,
        ResolveTool,
        StartupContextTool,
        PortfolioTool,
        SummaryTool,
        VerifyTool,
        ConvergenceTool,
        GithubSyncTool,
        SyncPrioritiesTool
    ]
);
