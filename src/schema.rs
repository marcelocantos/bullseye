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
///   `bullseye_summary`.
/// - `Some(2)` → v0.19.0+ format: the work-target `observable: true`
///   flag is renamed to `showcase: true` (with semantic strengthening
///   — retirement of a showcase target now requires a `demonstration`
///   string recorded via `bullseye_retire`). The legacy field name
///   continues to deserialise via `#[serde(alias = "observable")]`,
///   so older files load cleanly and are rewritten under the new name
///   on next save. The gates-field and parent-field migrations from
///   v0.4.0 and v0.8.0 continue to run transparently on load.
/// - `Some(3)` → 🎯T18: introduces the `set_aside` status (parked /
///   deferred / wont_fix) with a required `set_aside_reason` field.
///   Set-aside targets are terminal for graph traversal (they unblock
///   dependents) but distinct from achieved in summary output. Older
///   binaries reading a v3 file fail at load with a "schema_version
///   too new" error rather than silently dropping the new variant.
/// - `Some(4)` → 🎯T23: the `showcase` construct is removed entirely.
///   The boolean field on `Target` (and its legacy `observable` alias)
///   is gone, the `demonstration` field is gone, and `bullseye_put` /
///   `bullseye_retire` no longer accept the corresponding parameters.
///   Only verify-kind targets are checkpoints now. Pre-v4 YAML files
///   continue to load — `showcase` and `demonstration` keys are
///   ignored on parse via serde's default behaviour and are stripped
///   on the next save. Older binaries reading a v4 file fail at load
///   with a "schema_version too new" error.
/// - `Some(5)` → 🎯T25: the `kind` enum is removed entirely, along
///   with the `verifies` / `rework` / `retry_budget` / `retries`
///   fields and the whole checkpoint / tunnel apparatus that grew up
///   around `kind: verify`. Targets are now uniform: every node has
///   acceptance criteria, every node retires when those are met,
///   regardless of whether the pass signal comes from CI, a human ack,
///   or a smoke test (the signal source is free text on the
///   acceptance, not a node-type discriminator). Failed retirements
///   route through the new `bullseye_revert` operation. Pre-v5 YAML
///   files load unchanged — `kind`, `verifies`, `rework`,
///   `retry_budget`, and `retries` keys are silently dropped by serde
///   on parse and stripped on the next save (one-shot migration,
///   same shape as the v3→v4 showcase removal). Older binaries
///   reading a v5 file fail at load with a "schema_version too new"
///   error.
pub const CURRENT_SCHEMA_VERSION: u32 = 5;

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

    /// Optional globs of paths that constitute the **shipped surface**
    /// for this repo (🎯T42). When set, `bullseye_convergence` only
    /// treats fix commits as release-blocking when their diff since the
    /// last tag intersects one of these prefixes/globs. Absent → all
    /// fix commits count (legacy behaviour).
    ///
    /// Matching is path-prefix oriented: each entry is compared as a
    /// string prefix of paths from `git show --name-only` (trailing `/`
    /// optional). Examples: `src/`, `dist/`, `include/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub release_surface: Vec<String>,

    /// All targets keyed by ID (e.g., "T1", "T1.1").
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
}

/// Ownership exclusion — this target is driven by someone else (🎯T43).
///
/// Distinct from [`Status::SetAside`]: status stays active
/// (identified/converging), dependents remain blocked, and the target
/// is excluded only from *this* owner's frontier / next-action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedBy {
    /// Who is driving the work (free text handle / name).
    pub owner: String,
    /// Why this owner exclusion applies (required non-empty when set).
    pub reason: String,
}

/// Convergence strategy for a target — how the executor daemon should
/// drive this target toward its desired state without human intervention.
///
/// The schema is intentionally minimal: `command` and `trigger` are
/// the only required fields. The executor daemon (crosshair/bullseye-agent,
/// a separate repo) interprets all fields; bullseye itself only parses,
/// validates, and persists them. See 🎯T15.2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Strategy {
    /// Shell command (or path to script) the executor runs to attempt
    /// convergence. Must be non-empty.
    pub command: String,

    /// When to run — free-form trigger expression the executor parses.
    /// Examples: `"cron:0 * * * *"`, `"fswatch:/path/to/watch"`,
    /// `"on_wake"`, `"manual"`. Must be non-empty.
    pub trigger: String,

    /// Hard ceiling per attempt, expressed as a human duration
    /// (`"30m"`, `"2h"`). The executor kills the command when this
    /// elapses. Optional; omit to let the executor use its default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,

    /// Retry and backoff policy. Optional; omit to use executor defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
}

/// Retry and backoff policy for a [`Strategy`].
///
/// Both fields are optional and interpreted by the executor daemon.
/// Bullseye stores them verbatim without semantic validation beyond
/// structural presence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of consecutive attempts before the executor
    /// circuit-breaks. Optional; executor uses its own default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,

    /// Backoff algorithm and parameters — free-form string the executor
    /// parses. Examples: `"exponential"`, `"linear:30m"`.
    /// Optional; executor uses its own default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff: Option<String>,
}

/// A single target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    /// Short assertion describing the desired state.
    pub name: String,

    /// Current status.
    pub status: Status,

    /// Portfolio-scope value on Fibonacci scale (1, 2, 3, 5, 8, 13, 20).
    ///
    /// **Optional.** Omit / store `0.0` for unscored repo-scope work.
    /// Consumed only by cross-repo portfolio WSJF ranking — *not* by
    /// repo-level frontier ordering (which uses `depends_on` fanout).
    pub value: f64,

    /// Portfolio-scope cost on Fibonacci scale.
    ///
    /// **Optional.** Same scoping rule as [`Target::value`]: omit or
    /// `0.0` when unscored; portfolio ranking skips zero-cost terms.
    pub cost: f64,

    /// Actual cost recorded on retirement, for calibration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_cost: Option<f64>,

    /// Rationale recorded when the target's status is set to
    /// [`Status::SetAside`] — the free-text "why we decided not to
    /// pursue this" note. Required (non-empty) whenever
    /// `status == set_aside`; omitted for all other statuses.
    /// Validation flags missing or empty reasons.
    ///
    /// Free text by design: the prose carries the parked / deferred
    /// / wont_fix nuance without requiring the schema to commit to a
    /// fixed taxonomy. See 🎯T18.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_aside_reason: Option<String>,

    /// How to verify the desired state is achieved.
    pub acceptance: Vec<String>,

    /// Optional executable acceptance checks.
    ///
    /// Each entry names a sawmill verification primitive. Bullseye
    /// never calls sawmill directly (MCP servers can't call each
    /// other), so these are consumed by `bullseye_verify`, which
    /// returns a structured plan for the agent (or `/cv` skill) to
    /// execute against sawmill and feed results back into a report.
    ///
    /// See `docs/mcp-triad.md` §1 for the phasing: today's sawmill
    /// supports `convention` and `query`; `invariant` is reserved
    /// for phase 2 once sawmill T19 lands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<Check>,

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

    /// Cross-repo dependencies: capabilities or targets in other repos
    /// that this target depends on. Advisory only — does NOT block
    /// frontier computation (the dependency lives outside this repo's
    /// target graph, so bullseye can't authoritatively know when it's
    /// satisfied). Surfaced in `bullseye_portfolio` so the human sees
    /// the coupling.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cross_depends: Vec<CrossEdge>,

    /// Cross-repo enablers: targets or capabilities in other repos
    /// that this target unblocks. Used for value propagation — targets
    /// with non-empty `cross_enables` get a priority boost in the
    /// portfolio view since completing them unblocks work elsewhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cross_enables: Vec<CrossEdge>,

    /// Freeform tags (e.g., "visual").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Convergence strategy — how the executor daemon should drive this
    /// target toward its desired state. Parsed and persisted by bullseye;
    /// executed by the separate executor daemon (see 🎯T15.2). Absent
    /// on most targets; executor-driven targets set this to declare their
    /// automation contract alongside their acceptance criteria.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<Strategy>,

    /// How this target was created.
    #[serde(default = "default_origin")]
    pub origin: String,

    /// Date the target was discovered.
    pub discovered: NaiveDate,

    /// Date the target was achieved (filled on retirement).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub achieved: Option<NaiveDate>,

    /// When set, this target is owned by another agent/human and is
    /// excluded from the local frontier without unblocking dependents
    /// (🎯T43). Cleared with `bullseye_commit op=unassign`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<OwnedBy>,
}

/// A cross-repo edge — a link from a target in this repo to a target
/// or capability in another repo. Used for both `cross_depends` (this
/// target needs something from another repo) and `cross_enables`
/// (completing this target unblocks work in another repo).
///
/// Either `target` (a specific target ID like `"T1.4"`) or `capability`
/// (a free-form description of an API or feature) must be set — both
/// are optional in the struct so callers can pick the shape that fits
/// the reality of the reference, but [`crate::graph::validate`]
/// rejects edges where neither is set. Setting both is allowed for
/// edges that point at a specific target that also provides a named
/// capability.
///
/// Cross-repo refs are intentionally not validated against a scanned
/// portfolio: the reference may point at a repo the user hasn't asked
/// bullseye to scan, or at a target that doesn't yet exist. Dangling
/// refs stay valid; they just don't light up any cross-checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossEdge {
    /// Repository identifier (e.g. `"marcelocantos/jevon"`). Required;
    /// `bullseye_portfolio` uses this to group edges and to match
    /// against discovered repos.
    pub repo: String,

    /// Specific target ID in the referenced repo (e.g. `"T1.4"`).
    /// Optional — leave unset when the reference is to a named
    /// capability rather than a concrete target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,

    /// Free-form capability name (e.g. `"Manager API"`). Optional —
    /// leave unset when the reference is to a specific target ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,

    /// Human-readable note explaining the edge. Optional, free-form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl CrossEdge {
    /// Human-readable reference: `target` if present, else `capability`,
    /// else `"?"`. Used for rendering and error messages.
    pub fn reference(&self) -> String {
        match (&self.target, &self.capability) {
            (Some(t), Some(c)) => format!("🎯{t} ({c})"),
            (Some(t), None) => format!("🎯{t}"),
            (None, Some(c)) => c.clone(),
            (None, None) => "?".to_string(),
        }
    }
}

/// Legacy gate edge from older targets files. The edge is upstream→downstream
/// (A gates B means A enables B), which is the inverse of `depends_on`.
/// Criticality was a soft-blocking weight that no downstream logic ever
/// consumed; it is discarded on migration.
#[derive(Debug, Clone, PartialEq, Deserialize)]
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
#[serde(rename_all = "snake_case")]
pub enum Status {
    Identified,
    Converging,
    Achieved,
    /// Terminal disposition for a target the user has decided not to
    /// pursue (parked indefinitely, deferred, or actively rejected).
    /// Distinct from `Achieved`: the target was not delivered, but it
    /// is no longer a candidate for work and unblocks its dependents
    /// just like an achieved target. The motivation lives in
    /// [`Target::set_aside_reason`], which is required (non-empty)
    /// whenever this status is set. See 🎯T18.
    SetAside,
}

impl Status {
    /// Has this target reached a terminal disposition (achieved or
    /// set aside)? Terminal targets unblock their dependents and are
    /// excluded from the active set / frontier. Distinct from
    /// "achieved" — a set-aside target is terminal but not achieved.
    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Achieved | Status::SetAside)
    }
}

fn default_origin() -> String {
    "manual".to_string()
}

impl Target {
    /// Whether this target is active — neither achieved nor set
    /// aside. Active targets are the ones the frontier can surface
    /// and that count toward "still in flight" reporting.
    pub fn is_active(&self) -> bool {
        !self.status.is_terminal()
    }
}

/// A single executable check attached to a target.
///
/// Serialized form matches `docs/mcp-triad.md` §1 exactly: each list
/// entry is a single-key map where the key names the variant.
///
/// ```yaml
/// checks:
///   - convention: no-platform-ifdefs
///   - query:
///       kind: preprocessor_directive
///       pattern: "ifdef|ifndef|if defined"
///       exclude_path: src/platform/
///       expect: 0
///   - invariant: platform-isolation
/// ```
///
/// This is the external-tagging shape serde produces for an enum by
/// default; `rename_all = "lowercase"` gives us the doc-accurate
/// discriminator names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Check {
    /// Reference a named sawmill convention via `check_conventions`.
    Convention {
        /// Name of the sawmill convention to run.
        convention: String,
    },
    /// Run a structural sawmill `query` and assert a result count.
    Query {
        /// Query parameters the sawmill `query` tool accepts.
        query: QueryCheck,
    },
    /// Reference a named sawmill structural invariant (phase 2,
    /// sawmill 🎯T19).
    Invariant {
        /// Name of the sawmill invariant to run.
        invariant: String,
    },
}

/// Parameters for a `query`-kind check. Mirrors the fields sawmill's
/// `query` tool accepts today: a node kind, an optional regex pattern,
/// an optional path-exclusion glob, and an expected match count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryCheck {
    /// Tree-sitter node kind sawmill should look for (e.g.
    /// `preprocessor_directive`).
    pub kind: String,
    /// Optional regex or substring the matching nodes must contain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Optional path glob to exclude from the search (e.g. a directory
    /// where the pattern is allowed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_path: Option<String>,
    /// Expected number of matches; the check passes when the observed
    /// count equals this number.
    pub expect: u32,
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

    /// Achieved targets only — strictly `Status::Achieved`. Set-aside
    /// targets are terminal but not achieved and are excluded here.
    pub fn achieved(&self) -> BTreeMap<&str, &Target> {
        self.targets
            .iter()
            .filter(|(_, t)| t.status == Status::Achieved)
            .map(|(id, t)| (id.as_str(), t))
            .collect()
    }

    /// Set-aside targets only — `Status::SetAside`. Rendered in their
    /// own group so reviewers can see what was decided not to do and
    /// why, separate from the achievements record.
    pub fn set_aside(&self) -> BTreeMap<&str, &Target> {
        self.targets
            .iter()
            .filter(|(_, t)| t.status == Status::SetAside)
            .map(|(id, t)| (id.as_str(), t))
            .collect()
    }
}
