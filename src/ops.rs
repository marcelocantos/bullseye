// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use crate::schema::{Check, Kind, QueryCheck, Status, TargetsFile};

/// Result of a rework operation.
#[derive(Debug)]
pub struct ReworkResult {
    /// The rework destination target ID.
    pub rework_id: String,
    /// Name of the rework destination.
    pub rework_name: String,
    /// New retry count after this rework.
    pub retries: u32,
    /// Retry budget (if set).
    pub budget: Option<u32>,
    /// Whether the retry budget is exhausted.
    pub budget_exhausted: bool,
}

/// Error from a rework operation.
#[derive(Debug, PartialEq)]
pub enum ReworkError {
    TargetNotFound(String),
    NotVerifyTarget(String),
    NoReworkTarget(String),
    ReworkDestNotFound(String),
}

impl std::fmt::Display for ReworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReworkError::TargetNotFound(id) => write!(f, "target {id} not found"),
            ReworkError::NotVerifyTarget(id) => write!(f, "🎯{id} is not a verify target"),
            ReworkError::NoReworkTarget(id) => write!(f, "🎯{id} has no rework target"),
            ReworkError::ReworkDestNotFound(id) => write!(f, "rework target {id} does not exist"),
        }
    }
}

/// Execute a rework cycle: reset verify target to identified, reset
/// rework destination to converging, increment retries, append diagnosis.
///
/// Returns information about the rework for display. Does NOT save to disk.
pub fn rework(
    file: &mut TargetsFile,
    verify_id: &str,
    diagnosis: &str,
) -> Result<ReworkResult, ReworkError> {
    // Validate the verify target.
    let verify = file
        .targets
        .get(verify_id)
        .ok_or_else(|| ReworkError::TargetNotFound(verify_id.to_string()))?;

    if verify.kind != Kind::Verify {
        return Err(ReworkError::NotVerifyTarget(verify_id.to_string()));
    }

    let rework_id = verify
        .rework
        .clone()
        .ok_or_else(|| ReworkError::NoReworkTarget(verify_id.to_string()))?;

    if !file.targets.contains_key(&rework_id) {
        return Err(ReworkError::ReworkDestNotFound(rework_id));
    }

    // Reset the verify target to identified.
    file.targets.get_mut(verify_id).unwrap().status = Status::Identified;

    // Reset the rework target to converging and increment retries.
    let rework_target = file.targets.get_mut(&rework_id).unwrap();
    rework_target.status = Status::Converging;
    rework_target.retries += 1;
    let retries = rework_target.retries;
    let budget = rework_target.retry_budget;
    let rework_name = rework_target.name.clone();

    // Append diagnosis to rework target's context if provided.
    if !diagnosis.is_empty() {
        let ctx = &mut file.targets.get_mut(&rework_id).unwrap().context;
        if !ctx.is_empty() {
            ctx.push_str("\n\n");
        }
        ctx.push_str(&format!("Rework #{retries}: {diagnosis}"));
    }

    Ok(ReworkResult {
        rework_id,
        rework_name,
        retries,
        budget,
        budget_exhausted: budget.is_some_and(|b| retries >= b),
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
