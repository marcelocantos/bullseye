// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! The single write verb (🎯T76).
//!
//! Every ledger mutation is a partial desired-state *fragment* applied
//! to the current file. The engine diffs the fragment against what is
//! there, derives the transitions, enforces the evidence policy, and
//! mutates. The eleven `commit --op` verbs are sugar that build a
//! request and call [`apply`].
//!
//! Two properties this module exists to guarantee, both of which were
//! foot-guns in the per-verb design that preceded it:
//!
//! 1. **A fragment is partial-merge and never authoritative for
//!    absence.** Listing three targets must not delete the other 113.
//!    Removal is explicit ([`ApplyRequest::remove`]).
//! 2. **Unknown fields are errors.** `deny_unknown_fields` on
//!    [`Fragment`] means a typo'd field is rejected rather than
//!    silently dropped while the call reports success.
//!
//! The engine is pure: it operates on a `&mut TargetsFile` and does no
//! I/O, so both the MCP adapter and the CLI share one implementation
//! and it is unit-testable without a filesystem.

use std::collections::{BTreeMap, HashSet};

use chrono::{Local, NaiveDate};
use serde::Deserialize;

use crate::api::ErrorCode;
use crate::ops;
use crate::schema::{Check, OwnedBy, Status, Target, TargetsFile};

/// Every field a fragment may carry, and therefore every field an
/// agent can reach through `apply`.
///
/// The `--help` text and the MCP tool description are generated from
/// [`FIELD_HELP`] below, which a test pins against this struct so the
/// documented surface cannot drift from the real one — the exact
/// failure that drove agents to hand-edit the YAML (🎯T76).
#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fragment {
    /// Short assertion describing the desired state. Markdown.
    /// Downstream renderers interpret HTML. Cite tags as entities or
    /// code spans. Bullseye stores the text as written and does not escape.
    pub name: Option<String>,
    /// `identified` | `converging` | `achieved` | `set_aside`.
    pub status: Option<String>,
    /// Portfolio-scope value (omit at repo scope).
    pub value: Option<f64>,
    /// Portfolio-scope cost (omit at repo scope).
    pub cost: Option<f64>,
    /// Actual cost recorded on achievement, for calibration.
    pub actual_cost: Option<f64>,
    /// How to verify the desired state is achieved. Replaces the list.
    /// Markdown. Downstream renderers interpret HTML. Cite tags as
    /// entities or code spans. Bullseye stores the text as written and
    /// does not escape.
    pub acceptance: Option<Vec<String>>,
    /// Executable checks. Replaces the list (🎯T83). Acceptance is prose
    /// a human adjudicates; this is what a machine can run unattended.
    pub checks: Option<Vec<Check>>,
    /// Why this target matters, how it was discovered. Markdown.
    /// Downstream renderers interpret HTML. Cite tags as entities or
    /// code spans. Bullseye stores the text as written and does not escape.
    pub context: Option<String>,
    /// Freeform tags. Replaces the list.
    pub tags: Option<Vec<String>>,
    /// Hard blocking dependencies. Replaces the list.
    pub depends_on: Option<Vec<String>>,
    /// Sugar: inject this target into each listed target's `depends_on`.
    pub blocks: Option<Vec<String>>,
    /// How the target was created.
    pub origin: Option<String>,
    /// Allocate this target as the next child of the named parent.
    pub child_of: Option<String>,
    /// Evidence for `status → achieved`. Markdown. Downstream renderers
    /// interpret HTML. Cite tags as entities or code spans. Bullseye
    /// stores the text as written and does not escape.
    pub attestation: Option<String>,
    /// Evidence for `→ set_aside`, reopen, and owner changes.
    #[serde(alias = "set_aside_reason")]
    pub reason: Option<String>,
    /// Owner handle; excludes from the frontier without unblocking
    /// dependents. Empty string clears.
    pub owner: Option<String>,
    /// Wake date — excluded from the frontier until then.
    pub postponed_until: Option<NaiveDate>,
    /// Opaque agent-evaluated wake condition.
    pub postpone_predicate: Option<String>,
    /// Precondition: refuse unless the target currently has this
    /// status. Complements the file-level `base` hash with a
    /// per-target check, and lets a verb like `reopen` insist that it
    /// is really reopening something.
    pub if_status: Option<String>,
    /// Fields to reset to empty. A partial fragment says what a field
    /// *becomes*, which cannot express "becomes nothing" — omitting a
    /// field means "leave it alone". `clear` is that missing half, and
    /// is uniform across fields rather than a per-field sentinel.
    pub clear: Option<Vec<String>>,
}

/// Fields `clear` accepts, and what clearing each one means.
pub const CLEARABLE_FIELDS: &[&str] = &[
    "owner",
    "postponed_until",
    "postpone_predicate",
    "actual_cost",
    "context",
    "tags",
    "depends_on",
];

/// One row of the documented field surface. Pinned against
/// [`Fragment`] by a test so help text cannot drift from the schema.
pub struct FieldHelp {
    pub name: &'static str,
    pub blurb: &'static str,
}

/// The complete patchable field set, in help-display order.
pub const FIELD_HELP: &[FieldHelp] = &[
    FieldHelp {
        name: "name",
        blurb: "short assertion of the desired state. Markdown. Downstream renderers interpret HTML. Cite tags as entities or code spans. Bullseye stores the text as written and does not escape",
    },
    FieldHelp {
        name: "status",
        blurb: "identified | converging | achieved | set_aside",
    },
    FieldHelp {
        name: "value",
        blurb: "portfolio-scope value (omit at repo scope)",
    },
    FieldHelp {
        name: "cost",
        blurb: "portfolio-scope cost (omit at repo scope)",
    },
    FieldHelp {
        name: "actual_cost",
        blurb: "actual cost recorded on achievement",
    },
    FieldHelp {
        name: "acceptance",
        blurb: "verification criteria (replaces the list). Markdown. Downstream renderers interpret HTML. Cite tags as entities or code spans. Bullseye stores the text as written and does not escape",
    },
    FieldHelp {
        name: "checks",
        blurb: "executable checks (replaces the list)",
    },
    FieldHelp {
        name: "context",
        blurb: "why this matters. Markdown. Downstream renderers interpret HTML. Cite tags as entities or code spans. Bullseye stores the text as written and does not escape",
    },
    FieldHelp {
        name: "tags",
        blurb: "freeform tags (replaces the list)",
    },
    FieldHelp {
        name: "depends_on",
        blurb: "hard blocking dependencies (replaces the list)",
    },
    FieldHelp {
        name: "blocks",
        blurb: "inject this target into others' depends_on",
    },
    FieldHelp {
        name: "origin",
        blurb: "how the target was created",
    },
    FieldHelp {
        name: "child_of",
        blurb: "allocate as the next child of this parent",
    },
    FieldHelp {
        name: "attestation",
        blurb: "evidence for status → achieved. Markdown. Downstream renderers interpret HTML. Cite tags as entities or code spans. Bullseye stores the text as written and does not escape",
    },
    FieldHelp {
        name: "reason",
        blurb: "evidence for set_aside, reopen, owner changes",
    },
    FieldHelp {
        name: "owner",
        blurb: "owner handle; empty string clears",
    },
    FieldHelp {
        name: "postponed_until",
        blurb: "wake date (YYYY-MM-DD)",
    },
    FieldHelp {
        name: "postpone_predicate",
        blurb: "opaque agent-evaluated wake condition",
    },
    FieldHelp {
        name: "if_status",
        blurb: "precondition: refuse unless the target has this status",
    },
    FieldHelp {
        name: "clear",
        blurb: "list of fields to reset to empty (see CLEARABLE_FIELDS)",
    },
];

/// An evidence obligation attached to a transition.
///
/// This table is the single source of truth for what a mutation must
/// carry. Previously each obligation lived inside the handler for the
/// verb that happened to perform it, so the rules could not be read,
/// tested, or quoted back to the caller in one place.
pub struct Obligation {
    /// Human label for the transition, quoted in the rejection.
    pub transition: &'static str,
    /// Fragment field that must be present and non-empty.
    pub requires: &'static str,
    /// Why the ledger insists on it.
    pub because: &'static str,
}

/// The transition-policy table.
pub const POLICY: &[Obligation] = &[
    Obligation {
        transition: "status → achieved",
        requires: "attestation",
        because: "an achievement claim without stated evidence is the failure mode the ledger exists to prevent",
    },
    Obligation {
        transition: "status → set_aside",
        requires: "reason",
        because: "a target dropped without a recorded rationale is indistinguishable later from one that was forgotten",
    },
    Obligation {
        transition: "achieved → active (reopen)",
        requires: "reason",
        because: "reopening contradicts a recorded attestation, so the contradiction must be explained",
    },
    Obligation {
        transition: "owner assigned",
        requires: "reason",
        because: "assigning moves work out of this frontier without unblocking dependents; clearing it needs no defence",
    },
];

/// Fields with no evidence obligation — free to patch at any time on
/// an active target. Documented here so the absence is deliberate
/// rather than an oversight.
pub const UNOBLIGED_FIELDS: &[&str] = &[
    "name",
    "acceptance",
    "checks",
    "context",
    "tags",
    "value",
    "cost",
    "depends_on",
    "origin",
];

/// A refusal, carrying the stable error code the envelope reports.
/// Alias of the shared [`crate::api::CodedError`] (🎯T74.14) — `apply`
/// used to own its own copy of this shape, which is exactly the kind
/// of duplication that let the code get dropped again downstream in
/// `ops::subdivide`'s `SubdivideError::Apply(e.message)`.
pub type ApplyError = crate::api::CodedError;

/// What an apply did.
#[derive(Debug, Default, PartialEq)]
pub struct ApplyReport {
    /// Newly created target IDs, in request order. For allocation
    /// slots this is the only way to learn the assigned ID.
    pub created: Vec<String>,
    /// Existing targets whose fields changed.
    pub updated: Vec<String>,
    /// Targets removed by an explicit `remove` list.
    pub removed: Vec<String>,
    /// `(blocker, blocked)` pairs added by `blocks` sugar.
    pub injected: Vec<(String, String)>,
}

impl ApplyReport {
    /// Every ID this apply touched, for the envelope's `changed:` line.
    pub fn changed(&self) -> Vec<String> {
        let mut out = self.created.clone();
        out.extend(self.updated.iter().cloned());
        out.extend(self.removed.iter().cloned());
        // `blocks` mutates the injectee; the structured `changed:` line
        // has to name it or T82's envelope lies (Fable F4).
        for (_, blocked) in &self.injected {
            out.push(blocked.clone());
        }
        out.sort();
        out.dedup();
        out
    }
}

/// One apply: a set of fragments, plus explicit removals.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyRequest {
    /// Optional CAS token — the `content_hash` the caller believes the
    /// file has. A mismatch is [`ErrorCode::Conflict`], which is what
    /// makes the hash load-bearing on the write path instead of a
    /// passive tripwire.
    #[serde(default)]
    pub base: Option<String>,
    /// Audit note for the apply as a whole.
    #[serde(default)]
    pub reason: Option<String>,
    /// Fragments keyed by target ID. A key beginning with `_` is an
    /// allocation slot: the server assigns the ID and reports it in
    /// [`ApplyReport::created`].
    #[serde(default)]
    pub targets: BTreeMap<String, Fragment>,
    /// Explicit removals. Never inferred from a fragment's absence.
    #[serde(default)]
    pub remove: Vec<String>,
    /// Acknowledge the one-way rollout cost of introducing this ledger's
    /// first `command` check (🎯T86).
    ///
    /// Lives here, on the single write verb, rather than on one command.
    /// It was originally checked in the `commit op=track` handler, which
    /// left `bullseye_apply` as an unguarded route — and fragment files
    /// routed through it stamped three ledgers at schema_version 6 with
    /// no prompt at all. A one-way door with a guard on one of its two
    /// doors is not guarded.
    #[serde(default)]
    pub adopt_command_checks: bool,
}

/// True when this map key asks the server to allocate an ID.
pub fn is_allocation_slot(key: &str) -> bool {
    key.starts_with('_')
}

fn append_context_audit(target: &mut Target, entry: String) {
    if target.context.is_empty() {
        target.context = entry;
    } else {
        target.context.push_str("\n\n");
        target.context.push_str(&entry);
    }
}

fn parse_status(s: &str) -> Result<Status, ApplyError> {
    match s {
        "identified" => Ok(Status::Identified),
        "converging" => Ok(Status::Converging),
        "achieved" => Ok(Status::Achieved),
        "set_aside" => Ok(Status::SetAside),
        other => Err(ApplyError::new(
            ErrorCode::InvalidArgs,
            format!(
                "unknown status `{other}` — use identified, converging, achieved, or set_aside"
            ),
        )),
    }
}

fn non_empty(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.trim().is_empty())
}

/// Portfolio scores must be real numbers. `NaN < 0.0` is false and Inf
/// is non-negative, so a `< 0` check alone lets them through and
/// poisons WSJF ranking (Fable F7).
fn refuse_non_finite(id: &str, field: &str, v: f64) -> Result<(), ApplyError> {
    if v.is_finite() {
        return Ok(());
    }
    Err(ApplyError::new(
        ErrorCode::Validation,
        format!("🎯{id}: {field} must be a finite number, got {v}"),
    ))
}

/// Normalize and validate achieve attestation (🎯T58).
///
/// Soft API nudge: non-empty free text, reject a few trivial tokens
/// (`done`, `ok`, …). Not a semantic judge of truth.
pub fn normalize_attestation(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(
            "achieve requires a non-empty `attestation` — a short note on how you believe \
             the target is met (SHA, test name, persona oracle, owner smoke, residual risk). \
             Not formal proof."
                .to_string(),
        );
    }
    // Cheap trivial-string reject (optional acceptance). Case-insensitive
    // whole-string match only — not a semantic judge.
    const TRIVIAL: &[&str] = &[
        "done",
        "ok",
        "yes",
        "yep",
        "fixed",
        "n/a",
        "na",
        "pass",
        "passed",
        "lgtm",
        "shipped",
        "complete",
        "completed",
        "achieved",
        "finished",
    ];
    let lower = trimmed.to_ascii_lowercase();
    if TRIVIAL.contains(&lower.as_str()) {
        return Err(format!(
            "attestation `{trimmed}` is too trivial — write a short note on how you believe \
             the target is met (SHA, test name, persona oracle, owner smoke, residual risk). \
             Not formal proof."
        ));
    }
    if trimmed.chars().count() < 4 {
        return Err(
            "attestation is too short — write a short note on how you believe the target is \
             met (SHA, test name, persona oracle, owner smoke, residual risk). Not formal proof."
                .to_string(),
        );
    }
    Ok(trimmed.to_string())
}

/// Check the fragment against [`POLICY`] for one target's transition.
///
/// `from` is `None` for a create.
fn check_obligations(
    id: &str,
    from: Option<Status>,
    to: Status,
    frag: &Fragment,
) -> Result<(), ApplyError> {
    // Assigning an owner needs a justification; clearing one does not
    // — it returns the work to this frontier rather than removing it.
    let owner_assigned = frag.owner.as_deref().is_some_and(|o| !o.trim().is_empty());
    let becomes_achieved = to == Status::Achieved && from != Some(Status::Achieved);
    let becomes_set_aside = to == Status::SetAside && from != Some(Status::SetAside);
    let reopened = from == Some(Status::Achieved) && to != Status::Achieved;

    let mut unmet: Vec<&Obligation> = Vec::new();
    for ob in POLICY {
        let triggered = match ob.transition {
            "status → achieved" => becomes_achieved,
            "status → set_aside" => becomes_set_aside,
            "achieved → active (reopen)" => reopened,
            "owner assigned" => owner_assigned,
            _ => false,
        };
        if !triggered {
            continue;
        }
        let satisfied = match ob.requires {
            // Attestation carries a content gate, not just a presence
            // one: "done" is a claim, not evidence. Enforced here so
            // `apply` cannot be used to route around the bar that
            // `commit --op achieve` has always applied.
            "attestation" => match &frag.attestation {
                Some(raw) => match normalize_attestation(raw) {
                    Ok(_) => true,
                    Err(msg) => {
                        return Err(ApplyError::new(
                            ErrorCode::Validation,
                            format!("apply rejected for 🎯{id}: {msg}"),
                        ));
                    }
                },
                None => false,
            },
            "reason" => non_empty(&frag.reason),
            _ => true,
        };
        if !satisfied {
            unmet.push(ob);
        }
    }

    if unmet.is_empty() {
        return Ok(());
    }
    let detail = unmet
        .iter()
        .map(|o| {
            format!(
                "`{}` requires `{}` — {}",
                o.transition, o.requires, o.because
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(ApplyError::new(
        ErrorCode::Validation,
        format!("apply rejected for 🎯{id}: {detail}"),
    ))
}

/// Apply `req` to `file`.
///
/// `historical` is the set of every target ID ever assigned in git
/// history, used so an allocated or explicit ID cannot collide with
/// one that exists on another branch (🎯T28).
/// Would this request introduce the ledger's first `command` check?
///
/// False once the ledger already has one: the compatibility cost is paid
/// and re-asking would only train people to pass the flag.
fn introduces_first_command_check(file: &TargetsFile, req: &ApplyRequest) -> bool {
    let already =
        crate::schema::required_schema_version(file) >= crate::schema::COMMAND_CHECK_SCHEMA_VERSION;
    if already {
        return false;
    }
    req.targets.values().any(|f| {
        f.checks
            .as_ref()
            .is_some_and(|cs| cs.iter().any(|c| matches!(c, Check::Command { .. })))
    })
}

/// True when the fragment would rewrite stored content.
///
/// Presence is not a change: a `--set` whose value already matches the
/// target is a no-op, and the no-change report is reserved for that
/// case. Achieved immutability (🎯T8) fires only when the write would
/// actually mutate — including `attestation`, which is content on an
/// achieved row even though it is only *applied* on a status
/// transition to achieved.
fn fragment_changes_content(target: &Target, frag: &Fragment) -> bool {
    if frag.name.as_ref().is_some_and(|v| v != &target.name) {
        return true;
    }
    if frag.value.is_some_and(|v| v != target.value) {
        return true;
    }
    if frag.cost.is_some_and(|v| v != target.cost) {
        return true;
    }
    if frag
        .actual_cost
        .is_some_and(|v| target.actual_cost != Some(v))
    {
        return true;
    }
    if frag
        .acceptance
        .as_ref()
        .is_some_and(|v| v != &target.acceptance)
    {
        return true;
    }
    if frag.checks.as_ref().is_some_and(|v| v != &target.checks) {
        return true;
    }
    if frag.context.as_ref().is_some_and(|v| v != &target.context) {
        return true;
    }
    if frag.tags.as_ref().is_some_and(|v| v != &target.tags) {
        return true;
    }
    if frag.origin.as_ref().is_some_and(|v| v != &target.origin) {
        return true;
    }
    if frag
        .depends_on
        .as_ref()
        .is_some_and(|v| v != &target.depends_on)
    {
        return true;
    }
    if frag
        .attestation
        .as_ref()
        .is_some_and(|v| target.attestation.as_ref() != Some(v))
    {
        return true;
    }
    if let Some(fields) = &frag.clear {
        for field in fields {
            let occupied = match field.as_str() {
                "owner" => target.owned_by.is_some(),
                "postponed_until" => target.postponed_until.is_some(),
                "postpone_predicate" => target.postpone_predicate.is_some(),
                "actual_cost" => target.actual_cost.is_some(),
                "context" => !target.context.is_empty(),
                "tags" => !target.tags.is_empty(),
                "depends_on" => !target.depends_on.is_empty(),
                // Unknown clear keys are a later validation error; treat
                // them as a change so an achieved row still refuses first.
                _ => true,
            };
            if occupied {
                return true;
            }
        }
    }
    false
}

pub fn apply(
    file: &mut TargetsFile,
    req: &ApplyRequest,
    historical: &HashSet<String>,
) -> Result<ApplyReport, ApplyError> {
    // Rollout gate before anything is touched (🎯T86). Adopting the
    // `command` kind stamps the ledger schema_version 6, after which
    // every older bullseye refuses to read the WHOLE file — other
    // agents' servers, other checkouts, CI, at once — and no fix reaches
    // an installed binary. Refusal is the default so the one-way door
    // cannot be walked through by accident.
    if introduces_first_command_check(file, req) && !req.adopt_command_checks {
        return Err(ApplyError::new(
            ErrorCode::Validation,
            format!(
                "refusing to add the first `command` check to this ledger.\n\
                 \n\
                 This is a one-way door. The file will be stamped schema_version {v}, \
                 and EVERY bullseye older than that kind will then refuse to read the \
                 whole file — not just the checked target. That includes other agents' \
                 MCP servers, other checkouts, and CI, all at once. No fix reaches a \
                 binary already installed.\n\
                 \n\
                 Before adopting: upgrade every consumer of this ledger, then re-run \
                 with the adoption acknowledged (`--adopt-command-checks` on the CLI, \
                 `adopt_command_checks: true` on a tool call, or that key in an apply \
                 fragment).",
                v = crate::schema::COMMAND_CHECK_SCHEMA_VERSION,
            ),
        ));
    }
    // CAS first: refuse before touching anything if the caller was
    // reasoning about a different version of the file.
    if let Some(base) = &req.base {
        let actual = crate::store::compute_content_hash(file);
        let want = base.trim().trim_start_matches("sha256:");
        if want != actual {
            return Err(ApplyError::new(
                ErrorCode::Conflict,
                format!(
                    "apply rejected: base hash {want} does not match the file's current \
                     content_hash {actual}. The ledger changed since you read it — re-read \
                     and rebuild the fragment on the current state."
                ),
            ));
        }
    }

    let today = Local::now().date_naive();
    let mut report = ApplyReport::default();
    let targets_before = file.targets.clone();

    // Removals run first so a fragment may re-create a removed ID in
    // the same apply.
    for id in &req.remove {
        if !file.targets.contains_key(id) {
            return Err(ApplyError::new(
                ErrorCode::NotFound,
                format!("cannot remove 🎯{id}: no such target"),
            ));
        }
        let dependents: Vec<String> = file
            .targets
            .iter()
            .filter(|(other, t)| *other != id && t.depends_on.iter().any(|d| d == id))
            .map(|(other, _)| other.clone())
            .collect();
        if !dependents.is_empty() {
            return Err(ApplyError::new(
                ErrorCode::Validation,
                format!(
                    "cannot remove 🎯{id}: {} still depends on it. Remove the edge first, \
                     or set_aside 🎯{id} instead so the record survives.",
                    dependents.join(", ")
                ),
            ));
        }
        file.targets.remove(id);
        report.removed.push(id.clone());
    }

    for (key, frag) in &req.targets {
        // Resolve the ID: allocation slot, child_of, or explicit.
        let (id, is_create) = if is_allocation_slot(key) {
            match &frag.child_of {
                Some(parent) => {
                    if !file.targets.contains_key(parent) {
                        return Err(ApplyError::new(
                            ErrorCode::NotFound,
                            format!("child_of parent 🎯{parent} does not exist"),
                        ));
                    }
                    (ops::next_subtarget_id(file, parent, historical), true)
                }
                None => (crate::id_alloc::next_top_level_id(file, historical), true),
            }
        } else {
            let exists = file.targets.contains_key(key);
            if !exists && frag.child_of.is_some() {
                return Err(ApplyError::new(
                    ErrorCode::InvalidArgs,
                    format!(
                        "🎯{key}: `child_of` allocates an ID, so it cannot be combined with an \
                         explicit key. Use an allocation slot (a key beginning with `_`) instead."
                    ),
                ));
            }
            (key.clone(), !exists)
        };

        if is_create && historical.contains(&id) {
            return Err(ApplyError::new(
                ErrorCode::IdReserved,
                format!(
                    "🎯{id} collides with a target recorded in git history (it may exist on \
                     another branch or have been deleted). Use an allocation slot to take the \
                     next free ID."
                ),
            ));
        }

        let from = file.targets.get(&id).map(|t| t.status);
        if let Some(expected) = &frag.if_status {
            let expected = parse_status(expected)?;
            if from != Some(expected) {
                return Err(ApplyError::new(
                    ErrorCode::Conflict,
                    match from {
                        Some(actual) => format!(
                            "🎯{id} is {actual:?}, but this apply requires it to be \
                             {expected:?} — the ledger is not in the state you assumed."
                        ),
                        None => format!("🎯{id} does not exist, so it cannot be {expected:?}"),
                    },
                ));
            }
        }
        let to = match &frag.status {
            Some(s) => parse_status(s)?,
            None => from.unwrap_or(Status::Identified),
        };
        // Structural refusal precedes evidence: telling a caller to
        // write an attestation for a transition that is forbidden
        // regardless would be the wrong correction.
        if to == Status::Achieved && from != Some(Status::Achieved) {
            // The effective edge set: a fragment may rewrite depends_on
            // in the same call, so check what the target will have
            // rather than what it had.
            // Dotted children are wired into the parent's depends_on
            // (🎯T39.1), so they would also trip the dependency check
            // below. Ask the family question first: for an umbrella the
            // family message names the relationship, which is the more
            // useful correction.
            // Both already return `CodedError` with `Validation` baked
            // in, so `?` carries the code through without re-wrapping.
            if from.is_some() {
                ops::refuse_active_family(file, &id)?;
            }
            let effective_deps: Vec<String> = frag.depends_on.clone().unwrap_or_else(|| {
                file.targets
                    .get(&id)
                    .map(|t| t.depends_on.clone())
                    .unwrap_or_default()
            });
            ops::refuse_open_dependencies(file, &id, &effective_deps)?;
        }
        check_obligations(&id, from, to, frag)?;

        if is_create {
            let name = frag.name.clone().ok_or_else(|| {
                ApplyError::new(
                    ErrorCode::InvalidArgs,
                    format!("🎯{id}: `name` is required when creating a target"),
                )
            })?;
            let acceptance = frag
                .acceptance
                .clone()
                .filter(|a| !a.is_empty())
                .ok_or_else(|| {
                    ApplyError::new(
                        ErrorCode::InvalidArgs,
                        format!("🎯{id}: `acceptance` is required when creating a target"),
                    )
                })?;
            let value = frag.value.unwrap_or(0.0);
            refuse_non_finite(&id, "value", value)?;
            let cost = frag.cost.unwrap_or(0.0);
            refuse_non_finite(&id, "cost", cost)?;
            let mut target = Target {
                name,
                status: to,
                value,
                cost,
                actual_cost: frag.actual_cost,
                set_aside_reason: (to == Status::SetAside)
                    .then(|| frag.reason.clone())
                    .flatten(),
                attestation: (to == Status::Achieved)
                    .then(|| frag.attestation.clone())
                    .flatten(),
                acceptance,
                checks: frag.checks.clone().unwrap_or_default(),
                context: frag.context.clone().unwrap_or_default(),
                gates: Vec::new(),
                depends_on: frag.depends_on.clone().unwrap_or_default(),
                cross_depends: Vec::new(),
                cross_enables: Vec::new(),
                tags: frag.tags.clone().unwrap_or_default(),
                strategy: None,
                origin: frag.origin.clone().unwrap_or_else(|| "manual".to_string()),
                discovered: today,
                achieved: (to == Status::Achieved).then_some(today),
                owned_by: None,
                postponed_until: frag.postponed_until,
                postpone_predicate: frag.postpone_predicate.clone(),
            };
            if to == Status::Achieved
                && let Some(note) = target.attestation.clone()
            {
                append_context_audit(&mut target, format!("Achieved {today}: {note}"));
            }
            file.targets.insert(id.clone(), target);
            // 🎯T39.1: a dotted create is a family edge, not a prefix.
            let parent_mutated = ops::attach_dotted_child(file, &id)
                .map_err(|e| ApplyError::new(ErrorCode::Validation, e.to_string()))?;
            report.created.push(id.clone());
            if parent_mutated && let Some(parent_id) = ops::direct_parent_id(&id) {
                report.updated.push(parent_id.to_string());
            }
        } else {
            // Achieved targets are historical artifacts: their content
            // is immutable unless this same apply reopens them (🎯T8).
            // Compare against stored values, not field presence: a
            // refused write must not be reported as "already matched".
            if from == Some(Status::Achieved)
                && to == Status::Achieved
                && let Some(existing) = file.targets.get(&id)
                && fragment_changes_content(existing, frag)
            {
                return Err(ApplyError::new(
                    ErrorCode::ImmutableAchieved,
                    format!(
                        "🎯{id} is achieved — its content is immutable. Reopen it in this same \
                         apply by setting `status: identified` with a `reason`, then patch."
                    ),
                ));
            }

            let target = file.targets.get_mut(&id).expect("existence checked above");
            if let Some(v) = &frag.name {
                target.name = v.clone();
            }
            if let Some(v) = frag.value {
                refuse_non_finite(&id, "value", v)?;
                target.value = v;
            }
            if let Some(v) = frag.cost {
                refuse_non_finite(&id, "cost", v)?;
                target.cost = v;
            }
            if let Some(v) = frag.actual_cost {
                target.actual_cost = Some(v);
            }
            if let Some(v) = &frag.acceptance {
                target.acceptance = v.clone();
            }
            if let Some(v) = &frag.checks {
                target.checks = v.clone();
            }
            if let Some(v) = &frag.context {
                target.context = v.clone();
            }
            if let Some(v) = &frag.tags {
                target.tags = v.clone();
            }
            if let Some(v) = &frag.origin {
                target.origin = v.clone();
            }
            if let Some(v) = &frag.depends_on {
                target.depends_on = v.clone();
            }
            if (frag.postponed_until.is_some() || frag.postpone_predicate.is_some())
                && target.status.is_terminal()
            {
                return Err(ApplyError::new(
                    ErrorCode::Validation,
                    format!(
                        "🎯{id} is terminal ({:?}) — reopen or un-set-aside before postponing",
                        target.status
                    ),
                ));
            }
            if let Some(v) = frag.postponed_until {
                target.postponed_until = Some(v);
            }
            if let Some(v) = &frag.postpone_predicate {
                target.postpone_predicate = Some(v.clone());
            }
            if let Some(owner) = &frag.owner {
                if !owner.trim().is_empty() && target.status.is_terminal() {
                    return Err(ApplyError::new(
                        ErrorCode::Validation,
                        format!(
                            "🎯{id} is {:?} — ownership exclusion only applies to active targets",
                            target.status
                        ),
                    ));
                }
                target.owned_by = if owner.trim().is_empty() {
                    None
                } else {
                    Some(OwnedBy {
                        owner: owner.clone(),
                        reason: frag.reason.clone().unwrap_or_default(),
                    })
                };
            }

            if let Some(fields) = &frag.clear {
                for field in fields {
                    match field.as_str() {
                        "owner" => target.owned_by = None,
                        "postponed_until" => target.postponed_until = None,
                        "postpone_predicate" => target.postpone_predicate = None,
                        "actual_cost" => target.actual_cost = None,
                        "context" => target.context = String::new(),
                        "tags" => target.tags.clear(),
                        "depends_on" => target.depends_on.clear(),
                        other => {
                            return Err(ApplyError::new(
                                ErrorCode::InvalidArgs,
                                format!(
                                    "cannot clear `{other}` — clearable fields are: {}",
                                    CLEARABLE_FIELDS.join(", ")
                                ),
                            ));
                        }
                    }
                }
            }

            if frag.status.is_some() && Some(to) != from {
                target.status = to;
                target.clear_illegal_status_scoped_fields();
                match to {
                    Status::Achieved => {
                        target.attestation = frag.attestation.clone();
                        if target.achieved.is_none() {
                            target.achieved = Some(today);
                        }
                        if let Some(note) = frag.attestation.clone() {
                            append_context_audit(target, format!("Achieved {today}: {note}"));
                        }
                    }
                    Status::SetAside => {
                        target.set_aside_reason = frag.reason.clone();
                    }
                    // Reopening contradicts a recorded attestation, so
                    // the explanation is appended to context: the audit
                    // trail survives in-place rather than living only in
                    // a tool result nobody reads again.
                    Status::Identified | Status::Converging if from == Some(Status::Achieved) => {
                        if let Some(reason) = frag.reason.as_deref().map(str::trim)
                            && !reason.is_empty()
                        {
                            append_context_audit(target, format!("Reverted {today}: {reason}"));
                        }
                    }
                    _ => {}
                }
            }
        }

        // `blocks` sugar: inject this target into each listed
        // target's depends_on.
        if let Some(blocks) = &frag.blocks {
            for other_id in blocks {
                if other_id == &id {
                    return Err(ApplyError::new(
                        ErrorCode::Validation,
                        format!("🎯{id} cannot block itself"),
                    ));
                }
                let other = file.targets.get_mut(other_id).ok_or_else(|| {
                    ApplyError::new(
                        ErrorCode::NotFound,
                        format!("blocks target 🎯{other_id} does not exist"),
                    )
                })?;
                if other.status == Status::Achieved {
                    return Err(ApplyError::new(
                        ErrorCode::ImmutableAchieved,
                        format!(
                            "cannot inject a dependency into 🎯{other_id} — it is achieved. \
                             Reopen it first. See 🎯T8."
                        ),
                    ));
                }
                if !other.depends_on.iter().any(|d| d == &id) {
                    other.depends_on.push(id.clone());
                    report.injected.push((id.clone(), other_id.clone()));
                }
            }
        }
    }

    // 🎯T82: `changed:` must reflect every target whose record actually
    // changed, not merely the IDs named in the request.
    report.updated = crate::store::targets_differing(&targets_before, &file.targets)
        .into_iter()
        .filter(|id| !report.created.contains(id) && !report.removed.contains(id))
        .collect();

    Ok(report)
}

#[cfg(test)]
mod tests {
    //! Unit tests for the single write verb (🎯T76). The engine is
    //! pure, so these run without touching a filesystem.
    use super::*;

    fn file_with(yaml: &str) -> TargetsFile {
        serde_yaml_ng::from_str(yaml).expect("fixture parses")
    }

    fn base_file() -> TargetsFile {
        file_with(
            r#"
schema_version: 5
targets:
  T1:
    name: thing works
    status: identified
    value: 0.0
    cost: 0.0
    acceptance: [it works]
    discovered: 2026-01-01
  T2:
    name: other thing works
    status: achieved
    value: 0.0
    cost: 0.0
    acceptance: [it also works]
    attestation: green on abc123
    discovered: 2026-01-01
    achieved: 2026-01-02
"#,
        )
    }

    fn req(targets: BTreeMap<String, Fragment>) -> ApplyRequest {
        ApplyRequest {
            base: None,
            reason: None,
            targets,
            remove: Vec::new(),
            adopt_command_checks: false,
        }
    }

    fn one(id: &str, frag: Fragment) -> ApplyRequest {
        let mut m = BTreeMap::new();
        m.insert(id.to_string(), frag);
        req(m)
    }

    fn no_history() -> HashSet<String> {
        HashSet::new()
    }

    // --- The two foot-gun rules -------------------------------------

    #[test]
    fn fragment_is_partial_merge_and_never_deletes_unlisted_targets() {
        let mut file = base_file();
        let r = apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    name: Some("renamed".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        assert_eq!(r.updated, vec!["T1"]);
        assert!(r.removed.is_empty());
        // T2 was not listed and must survive untouched.
        assert_eq!(file.targets.len(), 2);
        assert_eq!(file.targets["T2"].name, "other thing works");
        assert_eq!(file.targets["T1"].name, "renamed");
        // Unlisted fields on the listed target are also untouched.
        assert_eq!(file.targets["T1"].acceptance, vec!["it works".to_string()]);
    }

    #[test]
    fn unknown_fragment_field_is_an_error_not_a_silent_no_op() {
        let err = serde_yaml_ng::from_str::<Fragment>("nmae: typo\n").unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "expected unknown-field rejection, got: {err}"
        );
    }

    #[test]
    fn unknown_request_field_is_an_error() {
        let err = serde_yaml_ng::from_str::<ApplyRequest>("targts: {}\n").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "got: {err}");
    }

    // --- Policy table ------------------------------------------------

    #[test]
    fn achieve_without_attestation_is_rejected_and_quotes_the_policy() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    status: Some("achieved".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::Validation);
        assert!(
            err.message.contains("status → achieved"),
            "got: {}",
            err.message
        );
        assert!(err.message.contains("attestation"), "got: {}", err.message);
        // Refusal must not have mutated anything.
        assert_eq!(file.targets["T1"].status, Status::Identified);
    }

    #[test]
    fn achieve_with_attestation_records_it_and_dates_the_achievement() {
        let mut file = base_file();
        apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    status: Some("achieved".into()),
                    attestation: Some("green on deadbeef".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        assert_eq!(file.targets["T1"].status, Status::Achieved);
        assert_eq!(
            file.targets["T1"].attestation.as_deref(),
            Some("green on deadbeef")
        );
        assert!(file.targets["T1"].achieved.is_some());
        assert!(
            file.targets["T1"].context.contains("Achieved ")
                && file.targets["T1"].context.contains("green on deadbeef"),
            "achieve must append the skim-friendly audit line: {}",
            file.targets["T1"].context
        );
    }

    #[test]
    fn whitespace_only_attestation_does_not_satisfy_the_obligation() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    status: Some("achieved".into()),
                    attestation: Some("   ".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::Validation);
    }

    #[test]
    fn apply_cannot_route_around_the_attestation_content_gate() {
        // "done" is a claim, not evidence. `commit --op achieve` has
        // always rejected it; apply must not be the way around that.
        for trivial in ["done", "ok", "lgtm", "n/a", "xy"] {
            let mut file = base_file();
            let err = apply(
                &mut file,
                &one(
                    "T1",
                    Fragment {
                        status: Some("achieved".into()),
                        attestation: Some(trivial.into()),
                        ..Default::default()
                    },
                ),
                &no_history(),
            )
            .expect_err("must refuse trivial attestation");
            assert_eq!(err.code, ErrorCode::Validation);
            assert_eq!(file.targets["T1"].status, Status::Identified);
        }
    }

    #[test]
    fn set_aside_requires_a_reason_and_records_it() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    status: Some("set_aside".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert!(err.message.contains("set_aside"), "got: {}", err.message);

        apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    status: Some("set_aside".into()),
                    reason: Some("superseded by T9".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies with reason");
        assert_eq!(file.targets["T1"].status, Status::SetAside);
        assert_eq!(
            file.targets["T1"].set_aside_reason.as_deref(),
            Some("superseded by T9")
        );
    }

    #[test]
    fn reopening_an_achieved_target_requires_a_reason() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    status: Some("converging".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert!(err.message.contains("reopen"), "got: {}", err.message);

        apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    status: Some("converging".into()),
                    reason: Some("regression found in prod".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies with reason");
        assert_eq!(file.targets["T2"].status, Status::Converging);
    }

    #[test]
    fn assigning_an_owner_requires_a_reason() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    owner: Some("alice".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert!(err.message.contains("owner"), "got: {}", err.message);
    }

    #[test]
    fn clearing_an_owner_needs_no_reason() {
        let mut file = base_file();
        apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    owner: Some("alice".into()),
                    reason: Some("alice is driving it".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("assign");
        apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    owner: Some(String::new()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("clearing needs no reason");
        assert!(file.targets["T1"].owned_by.is_none());
    }

    #[test]
    fn an_owner_cannot_be_assigned_to_a_terminal_target() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    owner: Some("alice".into()),
                    reason: Some("r".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert!(
            err.message.contains("active targets"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn unobliged_fields_patch_freely_on_an_active_target() {
        let mut file = base_file();
        apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    name: Some("n".into()),
                    acceptance: Some(vec!["a".into()]),
                    context: Some("c".into()),
                    tags: Some(vec!["t".into()]),
                    value: Some(8.0),
                    cost: Some(5.0),
                    origin: Some("o".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        let t = &file.targets["T1"];
        assert_eq!(
            (t.name.as_str(), t.context.as_str(), t.value, t.cost),
            ("n", "c", 8.0, 5.0)
        );
        assert_eq!(t.acceptance, vec!["a".to_string()]);
        assert_eq!(t.tags, vec!["t".to_string()]);
        assert_eq!(t.origin, "o");
    }

    #[test]
    fn every_policy_row_names_a_real_fragment_field() {
        let known: HashSet<&str> = FIELD_HELP.iter().map(|f| f.name).collect();
        for ob in POLICY {
            assert!(
                known.contains(ob.requires),
                "policy names unknown field {}",
                ob.requires
            );
            assert!(
                !ob.because.is_empty(),
                "obligation {} has no rationale",
                ob.transition
            );
        }
        for f in UNOBLIGED_FIELDS {
            assert!(
                known.contains(f),
                "UNOBLIGED_FIELDS names unknown field {f}"
            );
        }
    }

    // --- Preconditions and clearing ---------------------------------

    #[test]
    fn if_status_refuses_when_the_ledger_is_not_in_the_assumed_state() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    if_status: Some("achieved".into()),
                    name: Some("renamed".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::Conflict);
        assert_eq!(file.targets["T1"].name, "thing works");
    }

    #[test]
    fn if_status_allows_the_change_when_it_matches() {
        let mut file = base_file();
        apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    if_status: Some("identified".into()),
                    name: Some("renamed".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        assert_eq!(file.targets["T1"].name, "renamed");
    }

    #[test]
    fn clear_resets_fields_that_omission_cannot_express() {
        let mut file = base_file();
        apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    context: Some("some background".into()),
                    tags: Some(vec!["a".into()]),
                    postpone_predicate: Some("when CI is green".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("set");
        apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    clear: Some(vec![
                        "context".into(),
                        "tags".into(),
                        "postpone_predicate".into(),
                    ]),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("clear");
        let t = &file.targets["T1"];
        assert!(t.context.is_empty() && t.tags.is_empty() && t.postpone_predicate.is_none());
    }

    #[test]
    fn clearing_an_unclearable_field_names_the_legal_set() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    clear: Some(vec!["name".into()]),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::InvalidArgs);
        assert!(
            err.message.contains("clearable fields are"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn reopening_appends_the_reason_to_context_as_an_audit_trail() {
        let mut file = base_file();
        apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    status: Some("converging".into()),
                    reason: Some("regression in prod".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        assert!(
            file.targets["T2"].context.contains("regression in prod"),
            "context should carry the revert note: {}",
            file.targets["T2"].context
        );
        assert!(file.targets["T2"].context.contains("Reverted"));
    }

    // --- depends_on gates achievement (🎯T79) ------------------------

    fn chain() -> TargetsFile {
        // T8 -> T7 -> T6, the shape reported from xbnf.
        file_with(
            r#"
schema_version: 5
targets:
  T6:
    name: base
    status: identified
    value: 0.0
    cost: 0.0
    acceptance: [a]
    discovered: 2026-01-01
  T7:
    name: middle
    status: identified
    value: 0.0
    cost: 0.0
    acceptance: [a]
    depends_on: [T6]
    discovered: 2026-01-01
  T8:
    name: top
    status: identified
    value: 0.0
    cost: 0.0
    acceptance: [a]
    depends_on: [T7]
    discovered: 2026-01-01
"#,
        )
    }

    fn achieve(id: &str) -> ApplyRequest {
        one(
            id,
            Fragment {
                status: Some("achieved".into()),
                attestation: Some("the work is genuinely done".into()),
                ..Default::default()
            },
        )
    }

    #[test]
    fn achieving_a_target_with_open_dependencies_is_refused() {
        let mut file = chain();
        let err = apply(&mut file, &achieve("T8"), &no_history()).expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::Validation);
        assert!(
            err.message.contains("T7"),
            "must name the blocker: {}",
            err.message
        );
        assert_eq!(
            file.targets["T8"].status,
            Status::Identified,
            "refusal must not mutate"
        );
    }

    #[test]
    fn the_chain_unblocks_in_dependency_order() {
        let mut file = chain();
        apply(&mut file, &achieve("T6"), &no_history()).expect("base has no blockers");
        apply(&mut file, &achieve("T7"), &no_history()).expect("T6 is achieved");
        apply(&mut file, &achieve("T8"), &no_history()).expect("T7 is achieved");
        assert_eq!(file.targets["T8"].status, Status::Achieved);
    }

    #[test]
    fn a_set_aside_dependency_unblocks_just_like_an_achieved_one() {
        // set_aside is terminal: the owner decided not to pursue it, so
        // it no longer gates dependents. The test is terminality, not
        // achievement.
        let mut file = chain();
        apply(
            &mut file,
            &one(
                "T7",
                Fragment {
                    status: Some("set_aside".into()),
                    reason: Some("superseded by another approach".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("set aside");
        apply(&mut file, &achieve("T8"), &no_history()).expect("a set-aside blocker unblocks");
    }

    #[test]
    fn a_dangling_dependency_does_not_block_achievement() {
        // A depends_on edge pointing at a target that does not exist is
        // a validation error in its own right; failing the achieve for
        // it would report the wrong problem.
        let mut file = chain();
        file.targets.get_mut("T8").unwrap().depends_on = vec!["T999".to_string()];
        apply(&mut file, &achieve("T8"), &no_history()).expect("dangling edge must not block");
    }

    #[test]
    fn creating_a_target_directly_as_achieved_still_honours_its_dependencies() {
        let mut file = chain();
        let err = apply(
            &mut file,
            &one(
                "T9",
                Fragment {
                    name: Some("born achieved".into()),
                    acceptance: Some(vec!["a".into()]),
                    depends_on: Some(vec!["T7".into()]),
                    status: Some("achieved".into()),
                    attestation: Some("done before it began".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert!(err.message.contains("T7"), "got: {}", err.message);
    }

    #[test]
    fn a_fragment_that_drops_the_edge_in_the_same_call_may_achieve() {
        // The check runs against the edge set the target will have, not
        // the one it had — dropping an edge that no longer holds is a
        // legitimate way to unblock.
        let mut file = chain();
        apply(
            &mut file,
            &one(
                "T8",
                Fragment {
                    status: Some("achieved".into()),
                    attestation: Some("T7 turned out not to gate this".into()),
                    depends_on: Some(vec![]),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("dropping the edge in the same call unblocks");
        assert_eq!(file.targets["T8"].status, Status::Achieved);
        assert!(file.targets["T8"].depends_on.is_empty());
    }

    // --- Achieved immutability --------------------------------------

    #[test]
    fn content_edit_on_achieved_target_is_refused_with_immutable_code() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    name: Some("rewritten".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::ImmutableAchieved);
        assert_eq!(file.targets["T2"].name, "other thing works");
    }

    #[test]
    fn clear_on_achieved_target_is_refused_with_immutable_code() {
        // Fable F2 / 🎯T8: `clear` is a content edit. The name-only
        // test above does not cover this hole.
        let mut file = base_file();
        file.targets.get_mut("T2").unwrap().context = "historical".into();
        let err = apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    clear: Some(vec!["context".into()]),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::ImmutableAchieved);
        assert_eq!(file.targets["T2"].context, "historical");
    }

    #[test]
    fn actual_cost_on_achieved_target_is_refused_with_immutable_code() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    actual_cost: Some(8.0),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::ImmutableAchieved);
        assert!(file.targets["T2"].actual_cost.is_none());
    }

    #[test]
    fn differing_attestation_on_achieved_target_is_refused_with_reopen_path() {
        // Specimen: `apply --set attestation=<corrected>` on an
        // achieved row used to succeed with an empty report, so the
        // handler claimed the fragment already matched. Attestation is
        // content; a differing write must name immutability.
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    attestation: Some("corrected: the Makefile does wipe bin/".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::ImmutableAchieved);
        assert!(
            err.message.contains("status: identified") && err.message.contains("reason"),
            "refusal must name the reopen path, got: {}",
            err.message
        );
        assert_eq!(
            file.targets["T2"].attestation.as_deref(),
            Some("green on abc123")
        );
    }

    #[test]
    fn re_achieve_with_differing_attestation_is_refused() {
        // `commit --op achieve --attestation <corrected>` is sugar for
        // status=achieved plus attestation. Same hole as --set.
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    status: Some("achieved".into()),
                    attestation: Some("corrected: the Makefile does wipe bin/".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::ImmutableAchieved);
        assert_eq!(
            file.targets["T2"].attestation.as_deref(),
            Some("green on abc123")
        );
    }

    #[test]
    fn identical_attestation_on_achieved_target_is_no_change() {
        let mut file = base_file();
        let report = apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    attestation: Some("green on abc123".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("byte-identical attestation is a no-op, not a refusal");
        assert!(report.updated.is_empty());
        assert_eq!(
            file.targets["T2"].attestation.as_deref(),
            Some("green on abc123")
        );
    }

    #[test]
    fn identical_name_on_achieved_target_is_no_change() {
        let mut file = base_file();
        let report = apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    name: Some("other thing works".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("byte-identical name is a no-op, not a refusal");
        assert!(report.updated.is_empty());
        assert_eq!(file.targets["T2"].name, "other thing works");
    }

    #[test]
    fn reopen_and_patch_in_one_apply_is_allowed() {
        let mut file = base_file();
        apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    status: Some("identified".into()),
                    reason: Some("acceptance was wrong".into()),
                    name: Some("rewritten".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        assert_eq!(file.targets["T2"].status, Status::Identified);
        assert_eq!(file.targets["T2"].name, "rewritten");
    }

    #[test]
    fn reopen_and_clear_in_one_apply_is_allowed() {
        let mut file = base_file();
        file.targets.get_mut("T2").unwrap().context = "historical".into();
        apply(
            &mut file,
            &one(
                "T2",
                Fragment {
                    status: Some("identified".into()),
                    reason: Some("acceptance was wrong".into()),
                    clear: Some(vec!["context".into()]),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        assert_eq!(file.targets["T2"].status, Status::Identified);
        assert!(
            !file.targets["T2"].context.contains("historical"),
            "clear must wipe the historical context: {}",
            file.targets["T2"].context
        );
    }

    // --- Creation and allocation ------------------------------------

    #[test]
    fn allocation_slot_assigns_an_id_and_reports_it() {
        let mut file = base_file();
        let r = apply(
            &mut file,
            &one(
                "_new",
                Fragment {
                    name: Some("fresh".into()),
                    acceptance: Some(vec!["works".into()]),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        assert_eq!(r.created.len(), 1);
        let id = &r.created[0];
        assert!(
            file.targets.contains_key(id),
            "allocated id {id} must exist"
        );
        assert_ne!(id, "_new");
    }

    #[test]
    fn create_requires_name_and_acceptance() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T9",
                Fragment {
                    name: Some("only a name".into()),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::InvalidArgs);
        assert!(err.message.contains("acceptance"), "got: {}", err.message);
    }

    #[test]
    fn explicit_id_colliding_with_git_history_is_reserved() {
        let mut file = base_file();
        let mut hist = HashSet::new();
        hist.insert("T9".to_string());
        let err = apply(
            &mut file,
            &one(
                "T9",
                Fragment {
                    name: Some("n".into()),
                    acceptance: Some(vec!["a".into()]),
                    ..Default::default()
                },
            ),
            &hist,
        )
        .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::IdReserved);
    }

    // --- Removal ------------------------------------------------------

    #[test]
    fn removal_is_explicit_and_refuses_to_dangle_an_edge() {
        let mut file = file_with(
            r#"
schema_version: 5
targets:
  T1:
    name: blocker
    status: identified
    value: 0.0
    cost: 0.0
    acceptance: [a]
    discovered: 2026-01-01
  T2:
    name: blocked
    status: identified
    value: 0.0
    cost: 0.0
    acceptance: [a]
    depends_on: [T1]
    discovered: 2026-01-01
"#,
        );
        let r = ApplyRequest {
            remove: vec!["T1".to_string()],
            ..Default::default()
        };
        let err = apply(&mut file, &r, &no_history()).expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::Validation);
        assert!(err.message.contains("T2"), "got: {}", err.message);
        assert!(file.targets.contains_key("T1"), "refusal must not remove");

        let r2 = ApplyRequest {
            remove: vec!["T2".to_string()],
            ..Default::default()
        };
        let rep = apply(&mut file, &r2, &no_history()).expect("leaf removes");
        assert_eq!(rep.removed, vec!["T2"]);
        assert!(!file.targets.contains_key("T2"));
    }

    #[test]
    fn removing_a_missing_target_is_not_found() {
        let mut file = base_file();
        let r = ApplyRequest {
            remove: vec!["T99".to_string()],
            ..Default::default()
        };
        let err = apply(&mut file, &r, &no_history()).expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    // --- CAS ----------------------------------------------------------

    #[test]
    fn base_hash_mismatch_is_a_conflict_and_changes_nothing() {
        let mut file = base_file();
        let mut r = one(
            "T1",
            Fragment {
                name: Some("renamed".into()),
                ..Default::default()
            },
        );
        r.base =
            Some("sha256:0000000000000000000000000000000000000000000000000000000000000000".into());
        let err = apply(&mut file, &r, &no_history()).expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::Conflict);
        assert_eq!(file.targets["T1"].name, "thing works");
    }

    #[test]
    fn matching_base_hash_is_accepted_with_or_without_the_prefix() {
        for prefix in ["", "sha256:"] {
            let mut file = base_file();
            let actual = crate::store::compute_content_hash(&file);
            let mut r = one(
                "T1",
                Fragment {
                    name: Some("renamed".into()),
                    ..Default::default()
                },
            );
            r.base = Some(format!("{prefix}{actual}"));
            apply(&mut file, &r, &no_history()).expect("applies");
            assert_eq!(file.targets["T1"].name, "renamed");
        }
    }

    // --- blocks sugar --------------------------------------------------

    #[test]
    fn blocks_sugar_injects_the_edge_once_and_is_idempotent() {
        let mut file = base_file();
        let frag = || Fragment {
            blocks: Some(vec!["T1".to_string()]),
            ..Default::default()
        };
        // T1 blocked by a new target.
        let r = apply(
            &mut file,
            &one(
                "_n",
                Fragment {
                    name: Some("blocker".into()),
                    acceptance: Some(vec!["a".into()]),
                    ..frag()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        let new_id = r.created[0].clone();
        assert_eq!(file.targets["T1"].depends_on, vec![new_id.clone()]);

        let again = apply(&mut file, &one(&new_id, frag()), &no_history()).expect("applies");
        assert!(again.injected.is_empty(), "second inject must be a no-op");
        assert_eq!(file.targets["T1"].depends_on, vec![new_id]);
    }

    #[test]
    fn a_target_cannot_block_itself() {
        let mut file = base_file();
        let err = apply(
            &mut file,
            &one(
                "T1",
                Fragment {
                    blocks: Some(vec!["T1".into()]),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect_err("must refuse");
        assert!(
            err.message.contains("cannot block itself"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn changed_includes_blocks_injectee() {
        // Fable F4 / 🎯T82: `changed:` must name every record that
        // actually moved, including the target `blocks` injected into.
        let mut file = base_file();
        let r = apply(
            &mut file,
            &one(
                "T3",
                Fragment {
                    name: Some("blocker".into()),
                    acceptance: Some(vec!["blocks T1".into()]),
                    blocks: Some(vec!["T1".into()]),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        assert_eq!(r.created, vec!["T3".to_string()]);
        assert_eq!(file.targets["T1"].depends_on, vec!["T3".to_string()]);
        assert!(
            r.changed().contains(&"T1".to_string()),
            "blocks injectee T1 must appear on changed:; got {:?}",
            r.changed()
        );
    }

    #[test]
    fn changed_includes_dotted_parent_on_attach() {
        // Fable F4 / 🎯T39.1: creating T1.1 appends T1.1 to T1 and may
        // promote T1 Identified→Converging. `changed:` must name T1.
        let mut file = base_file();
        let r = apply(
            &mut file,
            &one(
                "T1.1",
                Fragment {
                    name: Some("child of the thing".into()),
                    acceptance: Some(vec!["child works".into()]),
                    ..Default::default()
                },
            ),
            &no_history(),
        )
        .expect("applies");
        assert_eq!(r.created, vec!["T1.1".to_string()]);
        assert_eq!(file.targets["T1"].depends_on, vec!["T1.1".to_string()]);
        assert_eq!(file.targets["T1"].status, Status::Converging);
        assert!(
            r.changed().contains(&"T1".to_string()),
            "dotted parent T1 must appear on changed:; got {:?}",
            r.changed()
        );
    }

    #[test]
    fn non_finite_value_and_cost_are_rejected() {
        // Fable F7: NaN/Inf pass `v < 0.0` and poison portfolio WSJF.
        for (field, frag) in [
            (
                "value",
                Fragment {
                    value: Some(f64::NAN),
                    ..Default::default()
                },
            ),
            (
                "value",
                Fragment {
                    value: Some(f64::INFINITY),
                    ..Default::default()
                },
            ),
            (
                "cost",
                Fragment {
                    cost: Some(f64::NEG_INFINITY),
                    ..Default::default()
                },
            ),
        ] {
            let mut file = base_file();
            let err = apply(&mut file, &one("T1", frag), &no_history())
                .expect_err("non-finite score must be refused");
            assert_eq!(err.code, ErrorCode::Validation, "{field}: {}", err.message);
            assert!(
                err.message.contains(field),
                "{field} error must name the field: {}",
                err.message
            );
            assert_eq!(file.targets["T1"].value, 0.0);
            assert_eq!(file.targets["T1"].cost, 0.0);
        }
    }
}
