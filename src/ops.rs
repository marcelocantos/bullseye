// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use chrono::Local;

use crate::schema::{Check, QueryCheck, Status, Target, TargetsFile};

/// Result of a revert operation.
#[derive(Debug)]
pub struct RevertResult {
    /// Name of the target that was reopened (for display).
    pub name: String,
}

/// Error from a revert operation.
#[derive(Debug, PartialEq)]
pub enum RevertError {
    TargetNotFound(String),
    NotAchieved(String),
}

impl std::fmt::Display for RevertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RevertError::TargetNotFound(id) => write!(f, "target {id} not found"),
            RevertError::NotAchieved(id) => write!(
                f,
                "🎯{id} is not achieved — `bullseye_revert` re-opens previously-retired targets. \
                 To resume a set-aside target use `bullseye_put` with `status: identified`; to \
                 move an active target backwards, patch its status directly."
            ),
        }
    }
}

/// Re-open a previously-retired target: status moves Achieved →
/// Converging, the achieved date is cleared, and the reason is
/// appended to the target's context with today's date as a Revert
/// label so the audit trail survives in-place. Does NOT save to disk.
///
/// Replaces the v4 verify→rework retry-budget loop. The v4 model
/// modelled validation failure as a graph edge with a counter; v5
/// treats it as a direct mutation on the affected target. If a
/// retirement turns out to have been wrong (regression, missed
/// acceptance criterion, externally-detected breakage), the agent
/// (or human) calls `bullseye_revert` with a reason describing what
/// changed.
pub fn revert(
    file: &mut TargetsFile,
    target_id: &str,
    reason: &str,
) -> Result<RevertResult, RevertError> {
    let target = file
        .targets
        .get_mut(target_id)
        .ok_or_else(|| RevertError::TargetNotFound(target_id.to_string()))?;

    if target.status != Status::Achieved {
        return Err(RevertError::NotAchieved(target_id.to_string()));
    }

    target.status = Status::Converging;
    target.achieved = None;

    let trimmed = reason.trim();
    if !trimmed.is_empty() {
        let today = chrono::Local::now().date_naive();
        let entry = format!("Reverted {today}: {trimmed}");
        if target.context.is_empty() {
            target.context = entry;
        } else {
            target.context.push_str("\n\n");
            target.context.push_str(&entry);
        }
    }

    Ok(RevertResult {
        name: target.name.clone(),
    })
}

// --- subdivide ------------------------------------------------------------

/// How a `subdivide` call should treat the parent target and its
/// existing dependents.
///
/// All three modes create the new children. They differ in what
/// happens to the *parent* and to any existing targets that already
/// list the parent in their `depends_on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubdivideMode {
    /// Safest default: parent untouched. For every target that listed
    /// the parent in `depends_on`, the new children are appended
    /// alongside the parent (the dependent now blocks on both). No
    /// information is destroyed; the graph strictly tightens.
    Add,
    /// Parent becomes a converging umbrella: each new child is
    /// appended to the parent's own `depends_on`. The parent moves to
    /// `Converging` if previously `Identified`. Dependents are not
    /// touched — they continue to depend on the parent, which only
    /// retires once all its children do.
    Aggregate,
    /// Parent retires as-scoped (status → `Achieved`, today's date).
    /// For every existing dependent, the parent ID is removed from
    /// `depends_on` and the new child IDs are inserted in its place.
    /// Use this when the parent's original acceptance is genuinely
    /// met and the new children carry spillover work the original
    /// scope didn't anticipate.
    Retire,
}

impl SubdivideMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "add" => Ok(SubdivideMode::Add),
            "aggregate" => Ok(SubdivideMode::Aggregate),
            "retire" => Ok(SubdivideMode::Retire),
            other => Err(format!(
                "unknown subdivide mode `{other}` — use `add`, `aggregate`, or `retire`"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SubdivideMode::Add => "add",
            SubdivideMode::Aggregate => "aggregate",
            SubdivideMode::Retire => "retire",
        }
    }
}

/// Spec for a single new child target produced by `subdivide`.
///
/// `id` is optional — when omitted the child is auto-assigned the next
/// sub-target slot under the parent (e.g. parent T15 → T15.1, T15.2,
/// skipping any IDs that already exist). Supply `id` explicitly to
/// place the child at a top-level slot or at a specific sub-target ID.
#[derive(Debug, Clone)]
pub struct ChildSpec {
    pub id: Option<String>,
    pub name: String,
    pub acceptance: Vec<String>,
    pub context: Option<String>,
    pub tags: Option<Vec<String>>,
    pub depends_on: Option<Vec<String>>,
}

/// Outcome of a successful `subdivide` call. The handler renders this
/// into the tool response text.
#[derive(Debug)]
pub struct SubdivideResult {
    pub parent_id: String,
    pub parent_name: String,
    pub mode: SubdivideMode,
    pub created_children: Vec<String>,
    pub rewired_dependents: Vec<String>,
    pub parent_status_changed: bool,
}

#[derive(Debug, PartialEq)]
pub enum SubdivideError {
    ParentNotFound(String),
    ParentTerminal {
        id: String,
        status: Status,
    },
    NoChildren,
    /// A child's explicit `id` collides with an existing target.
    IdCollision(String),
    /// Two children in the same call asked for the same explicit ID.
    DuplicateChildId(String),
    EmptyChildField {
        index: usize,
        field: &'static str,
    },
    /// `retire` mode received an empty reason after trimming. The
    /// reason is optional, but if supplied it must be substantive.
    EmptyRetireReason,
    /// `tail` parameter supplied in a mode that has no dependent
    /// rewiring (currently `aggregate`). `tail` only applies to
    /// `retire` mode where dependents are spliced.
    TailRequiresRetireMode,
    /// `tail` parameter supplied but contained no IDs after parsing.
    /// Either omit the parameter or list at least one tail child.
    EmptyTail,
    /// A `tail` entry doesn't match any of the new children's
    /// assigned IDs. The tail must be a subset of the children being
    /// created — there is no fall-through to existing targets.
    TailNotInChildren(String),
}

impl std::fmt::Display for SubdivideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubdivideError::ParentNotFound(id) => write!(f, "parent target {id} not found"),
            SubdivideError::ParentTerminal { id, status } => write!(
                f,
                "🎯{id} is {status:?} — subdivision is rejected on terminal targets. \
                 If a regression makes the parent's achievement premature, run \
                 `bullseye_revert` first; if the parent was set aside, re-open it via \
                 `bullseye_put` with `status: identified` before subdividing.",
            ),
            SubdivideError::NoChildren => write!(
                f,
                "subdivide requires at least one child target — supply `children: [...]`"
            ),
            SubdivideError::IdCollision(id) => write!(
                f,
                "child id {id} collides with an existing target — pick a different id or omit it to auto-assign"
            ),
            SubdivideError::DuplicateChildId(id) => write!(
                f,
                "two children in this call asked for id {id} — child ids must be unique within the call"
            ),
            SubdivideError::EmptyChildField { index, field } => write!(
                f,
                "child at index {index} is missing required field `{field}`"
            ),
            SubdivideError::EmptyRetireReason => write!(
                f,
                "`retire_reason` was supplied but is empty after trimming — omit it or write a real reason"
            ),
            SubdivideError::TailRequiresRetireMode => write!(
                f,
                "`tail` only applies to `retire` mode. In `add` mode all new children are appended \
                 alongside the parent; in `aggregate` mode dependents are untouched. Drop the \
                 `tail` parameter or switch to `retire` mode."
            ),
            SubdivideError::EmptyTail => write!(
                f,
                "`tail` was supplied but contains no IDs. Either list at least one tail child or \
                 omit the parameter (omission means all children become the new dependency)."
            ),
            SubdivideError::TailNotInChildren(id) => write!(
                f,
                "`tail` entry {id} doesn't match any new child's ID. Tail must be a subset of the \
                 children being created in this call — supply explicit `id` values on the tail \
                 children and reference those exact IDs."
            ),
        }
    }
}

/// Auto-assign the next sub-target ID under `parent` (e.g. parent
/// `T15` → `T15.1`, `T15.2`, …). Only direct numeric children count;
/// deeper paths like `T15.1.3` are ignored when picking the next slot.
///
/// Unions the live in-memory keys with `historical` IDs (🎯T28) so
/// a sub-target that exists only on another branch's commits is still
/// honoured. `historical` may be empty when no git context is
/// available (e.g. external-mode shadow storage) — the function then
/// matches pre-T28 behaviour.
pub(crate) fn next_subtarget_id(
    file: &TargetsFile,
    parent: &str,
    historical: &std::collections::HashSet<String>,
) -> String {
    let prefix = format!("{parent}.");
    let in_memory = file.targets.keys().map(String::as_str);
    let from_history = historical.iter().map(String::as_str);
    let max_num = in_memory
        .chain(from_history)
        .filter_map(|k| {
            let suffix = k.strip_prefix(prefix.as_str())?;
            if suffix.contains('.') {
                None
            } else {
                suffix.parse::<u32>().ok()
            }
        })
        .max()
        .unwrap_or(0);
    format!("{parent}.{}", max_num + 1)
}

/// Split a parent target into one or more children, optionally
/// rewiring or retiring the parent. See [`SubdivideMode`] for the
/// per-mode semantics.
///
/// `tail` is only meaningful in [`SubdivideMode::Retire`]: when
/// supplied it restricts dependent rewiring to a subset of the new
/// children — every dependent that previously listed the parent
/// gets the tail IDs spliced in, not the whole child set. This is
/// the load-bearing knob for diamond decomposition (design → build
/// ∥ tests → validate): the diamond's tail is the convergence node,
/// and downstream dependents should only point at it, not at every
/// piece of the diamond.
///
/// Does NOT save to disk; the caller is expected to run this under
/// `with_locked_mutation` so the read-modify-write is atomic.
pub fn subdivide(
    file: &mut TargetsFile,
    parent_id: &str,
    mode: SubdivideMode,
    children: Vec<ChildSpec>,
    retire_reason: Option<&str>,
    tail: Option<&[String]>,
    historical: &std::collections::HashSet<String>,
) -> Result<SubdivideResult, SubdivideError> {
    if children.is_empty() {
        return Err(SubdivideError::NoChildren);
    }

    // Tail only makes sense when retire mode rewires dependents to
    // children. Reject early in the other modes so a caller mis-typing
    // the mode gets a clear error rather than silent drop of the tail.
    if let Some(tail_ids) = tail {
        if mode != SubdivideMode::Retire {
            return Err(SubdivideError::TailRequiresRetireMode);
        }
        if tail_ids.is_empty() {
            return Err(SubdivideError::EmptyTail);
        }
    }

    // Validate each child spec eagerly so we never half-create on a
    // later failure.
    for (idx, child) in children.iter().enumerate() {
        if child.name.trim().is_empty() {
            return Err(SubdivideError::EmptyChildField {
                index: idx,
                field: "name",
            });
        }
        if child.acceptance.is_empty() || child.acceptance.iter().all(|a| a.trim().is_empty()) {
            return Err(SubdivideError::EmptyChildField {
                index: idx,
                field: "acceptance",
            });
        }
    }

    let parent = file
        .targets
        .get(parent_id)
        .ok_or_else(|| SubdivideError::ParentNotFound(parent_id.to_string()))?;

    if parent.status.is_terminal() {
        return Err(SubdivideError::ParentTerminal {
            id: parent_id.to_string(),
            status: parent.status,
        });
    }

    // Resolve child IDs: explicit ones get checked for collisions
    // (both against existing targets and against each other); the rest
    // get auto-assigned next-slot sub-target IDs under the parent.
    // Auto-assignment scans the in-progress assignment list so two
    // auto-assigned children in the same call don't both pick the
    // same slot.
    let mut assigned_ids: Vec<String> = Vec::with_capacity(children.len());
    for child in &children {
        match &child.id {
            Some(explicit) => {
                // Collision against the live file is rejected. Collision
                // against the historical set (🎯T28) is rejected too —
                // an ID that once existed on another branch is reserved
                // even if the current tree has no record of it.
                if file.targets.contains_key(explicit) || historical.contains(explicit) {
                    return Err(SubdivideError::IdCollision(explicit.clone()));
                }
                if assigned_ids.contains(explicit) {
                    return Err(SubdivideError::DuplicateChildId(explicit.clone()));
                }
                assigned_ids.push(explicit.clone());
            }
            None => {
                // next_subtarget_id already unions the file's keys with
                // `historical`. We additionally have to step past slots
                // we just allocated in this same call.
                let mut candidate = next_subtarget_id(file, parent_id, historical);
                while assigned_ids.contains(&candidate) {
                    let next_num: u32 = candidate
                        .rsplit('.')
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0)
                        + 1;
                    candidate = format!("{parent_id}.{next_num}");
                }
                assigned_ids.push(candidate);
            }
        }
    }

    // Trim and validate the retire reason up front so the failure
    // path never produces a partial mutation.
    let trimmed_reason = match (mode, retire_reason) {
        (SubdivideMode::Retire, Some(r)) => {
            let t = r.trim();
            if t.is_empty() {
                return Err(SubdivideError::EmptyRetireReason);
            }
            Some(t.to_string())
        }
        _ => None,
    };

    // Snapshot the parent's name and original status before we touch
    // anything. Parent retirement (Retire mode) and umbrella promotion
    // (Aggregate mode) both inspect the prior status.
    let parent_name = parent.name.clone();
    let parent_prior_status = parent.status;

    // Create the children.
    let today = Local::now().date_naive();
    for (child, id) in children.iter().zip(assigned_ids.iter()) {
        let target = Target {
            name: child.name.clone(),
            status: Status::Identified,
            value: 0.0,
            cost: 0.0,
            actual_cost: None,
            set_aside_reason: None,
            acceptance: child.acceptance.clone(),
            checks: Vec::new(),
            context: child.context.clone().unwrap_or_default(),
            gates: Vec::new(),
            depends_on: child.depends_on.clone().unwrap_or_default(),
            cross_depends: Vec::new(),
            cross_enables: Vec::new(),
            tags: child.tags.clone().unwrap_or_default(),
            strategy: None,
            origin: format!("subdivide(🎯{parent_id})"),
            discovered: today,
            achieved: None,
            owned_by: None,
        };
        file.targets.insert(id.clone(), target);
    }

    // Per-mode parent + dependent rewiring.
    let mut rewired_dependents: Vec<String> = Vec::new();
    let mut parent_status_changed = false;
    match mode {
        SubdivideMode::Add => {
            // Find every active dependent and append each new child
            // ID alongside the parent. Idempotent: skip IDs already
            // present (shouldn't happen, but defensive).
            let dependent_ids: Vec<String> = file
                .targets
                .iter()
                .filter(|(_, t)| t.depends_on.iter().any(|d| d == parent_id))
                .map(|(id, _)| id.clone())
                .collect();
            for dep_id in &dependent_ids {
                if let Some(dep) = file.targets.get_mut(dep_id) {
                    for new_id in &assigned_ids {
                        if !dep.depends_on.contains(new_id) {
                            dep.depends_on.push(new_id.clone());
                        }
                    }
                }
            }
            rewired_dependents = dependent_ids;
        }
        SubdivideMode::Aggregate => {
            let parent = file
                .targets
                .get_mut(parent_id)
                .expect("parent existence checked above");
            for new_id in &assigned_ids {
                if !parent.depends_on.contains(new_id) {
                    parent.depends_on.push(new_id.clone());
                }
            }
            if parent.status == Status::Identified {
                parent.status = Status::Converging;
                parent_status_changed = true;
            }
        }
        SubdivideMode::Retire => {
            // Validate tail (if supplied) before any mutation: every
            // tail ID must match one of the assigned children's IDs.
            // Cross-checking against `assigned_ids` rather than the
            // raw child specs accounts for auto-assignment.
            if let Some(tail_ids) = tail {
                for tid in tail_ids {
                    if !assigned_ids.contains(tid) {
                        return Err(SubdivideError::TailNotInChildren(tid.clone()));
                    }
                }
            }

            // Replacement set: tail when supplied, otherwise all
            // assigned children (the pre-`tail` default behaviour).
            let owned_default = assigned_ids.clone();
            let replacement: &[String] = match tail {
                Some(t) => t,
                None => &owned_default,
            };

            // Rewire dependents: drop parent, splice in replacement.
            let dependent_ids: Vec<String> = file
                .targets
                .iter()
                .filter(|(_, t)| t.depends_on.iter().any(|d| d == parent_id))
                .map(|(id, _)| id.clone())
                .collect();
            for dep_id in &dependent_ids {
                if let Some(dep) = file.targets.get_mut(dep_id) {
                    let mut new_deps: Vec<String> = Vec::with_capacity(dep.depends_on.len());
                    for d in &dep.depends_on {
                        if d == parent_id {
                            for new_id in replacement {
                                if !new_deps.contains(new_id) {
                                    new_deps.push(new_id.clone());
                                }
                            }
                        } else if !new_deps.contains(d) {
                            new_deps.push(d.clone());
                        }
                    }
                    dep.depends_on = new_deps;
                }
            }
            rewired_dependents = dependent_ids;

            // Retire the parent and record the rationale in context.
            let parent = file
                .targets
                .get_mut(parent_id)
                .expect("parent existence checked above");
            parent.status = Status::Achieved;
            parent.achieved = Some(today);
            parent_status_changed = parent_prior_status != Status::Achieved;
            if let Some(reason) = trimmed_reason {
                let entry = format!("Subdivided {today}: {reason}");
                if parent.context.is_empty() {
                    parent.context = entry;
                } else {
                    parent.context.push_str("\n\n");
                    parent.context.push_str(&entry);
                }
            }
        }
    }

    Ok(SubdivideResult {
        parent_id: parent_id.to_string(),
        parent_name,
        mode,
        created_children: assigned_ids,
        rewired_dependents,
        parent_status_changed,
    })
}

// --- verify plan ----------------------------------------------------------

/// Error returned by [`verify_plan`] when the target is missing or has
/// no executable checks to run.
#[derive(Debug, PartialEq)]
pub enum VerifyError {
    /// No target with that ID in the file.
    TargetNotFound(String),
    /// The target exists but has no `checks` field populated.
    NoChecks(String),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::TargetNotFound(id) => write!(f, "target {id} not found"),
            VerifyError::NoChecks(id) => {
                write!(f, "🎯{id} has no executable checks to run")
            }
        }
    }
}

/// The sawmill tool a single planned check should dispatch to.
///
/// These are the *names* of sawmill MCP tools the agent should call.
/// Bullseye does not invoke sawmill itself — MCP servers cannot call
/// each other, so this field tells the orchestrating layer (agent or
/// `/cv` skill) exactly which tool to run. Kept as an enum so callers
/// can match on it rather than parsing a free-form string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SawmillTool {
    /// Sawmill's `check_conventions` — runs a named convention.
    CheckConventions,
    /// Sawmill's `query` — runs a structural query.
    Query,
    /// Sawmill's `check_invariants` — phase 2, sawmill 🎯T19.
    CheckInvariants,
}

/// One planned check: a sawmill tool to invoke and the arguments to
/// pass it. Also carries the original `Check` so the report can match
/// results back to the source without re-walking the target.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PlannedCheck {
    /// Caller-assigned index into the target's `checks` list. Lets
    /// the agent key results back by position without relying on
    /// check contents for identity.
    pub index: usize,
    /// Which sawmill tool to invoke.
    pub tool: SawmillTool,
    /// Human-readable summary of what this check does (for logs and
    /// the fallback text report). Structured fields live in `spec`.
    pub description: String,
    /// Structured arguments for the sawmill tool. Shape depends on
    /// `tool`:
    ///
    /// - `check_conventions` → `{ "convention": "<name>" }`
    /// - `query` → `{ "kind": ..., "pattern": ..., "exclude_path": ..., "expect": N }`
    /// - `check_invariants` → `{ "invariant": "<name>" }`
    pub spec: CheckSpec,
}

/// Per-variant argument payload the agent feeds to the sawmill tool.
///
/// Same on-the-wire shape as [`Check`]: an `untagged` enum whose
/// variants are distinguishable by their unique key, so JSON emits
/// `{"convention": "..."}` / `{"query": {...}}` / `{"invariant": "..."}`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum CheckSpec {
    Convention { convention: String },
    Query { query: QueryCheck },
    Invariant { invariant: String },
}

/// Structured verification plan returned by [`verify_plan`]. The
/// agent executes each `PlannedCheck` via the sawmill MCP server and
/// fills in the `report_template` to produce a final pass/fail summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct VerifyPlan {
    /// Target whose checks are being planned.
    pub target_id: String,
    /// Target name (for logs and report headers).
    pub target_name: String,
    /// The planned check invocations in order.
    pub checks: Vec<PlannedCheck>,
    /// A worked example of the JSON shape the agent should produce
    /// when it reports results back. Carried in the plan so callers
    /// don't have to re-derive it.
    pub report_template: VerifyReport,
}

/// Structured verification report. Bullseye does not fill this in —
/// it is returned from [`verify_plan`] as a *template* showing the
/// agent which fields to populate after executing the planned checks.
///
/// The same type can be round-tripped through JSON when the agent
/// feeds results back to a future tool call (not yet implemented).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VerifyReport {
    /// Target the report is for.
    pub target: String,
    /// Aggregate status across all checks.
    pub overall: CheckOutcome,
    /// Per-check outcomes, one entry per `PlannedCheck` in the plan.
    pub checks: Vec<CheckResult>,
}

/// Outcome of a single check, or of the overall report.
///
/// `Pending` is used in the template returned by [`verify_plan`] to
/// signal that the agent has not yet executed this check. The agent
/// replaces it with `Pass` or `Fail` once sawmill returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckOutcome {
    Pass,
    Fail,
    Pending,
}

/// Result entry for a single check within a [`VerifyReport`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CheckResult {
    /// Index into the plan's `checks` list (matches `PlannedCheck::index`).
    pub index: usize,
    /// Variant discriminator so consumers can group results without
    /// re-reading the plan.
    pub kind: CheckKind,
    /// Check outcome (populated by the agent).
    pub outcome: CheckOutcome,
    /// File/line-level failure detail. Empty for passing checks.
    #[serde(default)]
    pub failures: Vec<CheckFailure>,
}

/// Discriminator for [`CheckResult::kind`] — matches the variant of
/// the corresponding `Check` in the target's `checks` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckKind {
    Convention,
    Query,
    Invariant,
}

/// A single check failure with file/line-level detail, as required by
/// the target acceptance criterion. The agent populates this from
/// sawmill's output.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CheckFailure {
    /// Source file path, if sawmill reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// 1-based line number, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Human-readable failure message from sawmill.
    pub message: String,
}

/// Build a verification plan for the given target. Does NOT execute
/// anything — bullseye can't call sawmill directly — and does NOT
/// mutate the file.
///
/// The returned plan tells the agent exactly which sawmill tools to
/// run and with what arguments, and carries a report template the
/// agent populates with results.
pub fn verify_plan(file: &TargetsFile, target_id: &str) -> Result<VerifyPlan, VerifyError> {
    let target = file
        .targets
        .get(target_id)
        .ok_or_else(|| VerifyError::TargetNotFound(target_id.to_string()))?;

    if target.checks.is_empty() {
        return Err(VerifyError::NoChecks(target_id.to_string()));
    }

    let mut planned: Vec<PlannedCheck> = Vec::with_capacity(target.checks.len());
    let mut template_checks: Vec<CheckResult> = Vec::with_capacity(target.checks.len());

    for (idx, check) in target.checks.iter().enumerate() {
        let (tool, description, spec, kind) = match check {
            Check::Convention { convention } => (
                SawmillTool::CheckConventions,
                format!("check_conventions convention={convention}"),
                CheckSpec::Convention {
                    convention: convention.clone(),
                },
                CheckKind::Convention,
            ),
            Check::Query { query: q } => {
                let mut desc = format!("query kind={}", q.kind);
                if let Some(p) = &q.pattern {
                    desc.push_str(&format!(" pattern={p:?}"));
                }
                if let Some(x) = &q.exclude_path {
                    desc.push_str(&format!(" exclude_path={x}"));
                }
                desc.push_str(&format!(" expect={}", q.expect));
                (
                    SawmillTool::Query,
                    desc,
                    CheckSpec::Query { query: q.clone() },
                    CheckKind::Query,
                )
            }
            Check::Invariant { invariant } => (
                SawmillTool::CheckInvariants,
                format!("check_invariants invariant={invariant}"),
                CheckSpec::Invariant {
                    invariant: invariant.clone(),
                },
                CheckKind::Invariant,
            ),
        };

        planned.push(PlannedCheck {
            index: idx,
            tool,
            description,
            spec,
        });
        template_checks.push(CheckResult {
            index: idx,
            kind,
            outcome: CheckOutcome::Pending,
            failures: Vec::new(),
        });
    }

    Ok(VerifyPlan {
        target_id: target_id.to_string(),
        target_name: target.name.clone(),
        checks: planned,
        report_template: VerifyReport {
            target: target_id.to_string(),
            overall: CheckOutcome::Pending,
            checks: template_checks,
        },
    })
}
