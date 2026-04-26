// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Local;
use rust_mcp_sdk::McpServer;
use rust_mcp_sdk::mcp_server::ServerHandler;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use rust_mcp_sdk::schema::{
    CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams, RpcError,
};

use crate::config::{self, LOCATION_PROMPT, Location};
use crate::graph;
use crate::import;
use crate::ops;
use crate::portfolio;
use crate::schema::{Status, Target};
use crate::store;
use crate::tools::TargetTools;

#[derive(Default)]
pub struct TargetHandler;

#[async_trait]
impl ServerHandler for TargetHandler {
    async fn handle_list_tools_request(
        &self,
        _request: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            tools: TargetTools::tools(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        let tool: TargetTools = TargetTools::try_from(params)?;

        match tool {
            TargetTools::ListTool(t) => handle_list(t),
            TargetTools::GetTool(t) => handle_get(t),
            TargetTools::PutTool(t) => handle_put(t),
            TargetTools::RetireTool(t) => handle_retire(t),
            TargetTools::SetAsideTool(t) => handle_set_aside(t),
            TargetTools::FrontierTool(t) => handle_frontier(t),
            TargetTools::ReworkTool(t) => handle_rework(t),
            TargetTools::TunnelsTool(t) => handle_tunnels(t),
            TargetTools::ValidateTool(t) => handle_validate(t),
            TargetTools::GraphTool(t) => handle_graph(t),
            TargetTools::InitTool(t) => handle_init(t),
            TargetTools::ImportTool(t) => handle_import(t),
            TargetTools::StartupContextTool(t) => handle_startup_context(t),
            TargetTools::PortfolioTool(t) => handle_portfolio(t),
            TargetTools::SummaryTool(t) => handle_summary(t),
            TargetTools::VerifyTool(t) => handle_verify(t),
            TargetTools::ConvergenceTool(t) => handle_convergence(t),
        }
    }
}

type ToolResult = Result<CallToolResult, CallToolError>;

fn text_result(text: String) -> ToolResult {
    Ok(CallToolResult::text_content(vec![text.into()]))
}

fn tool_err(msg: impl Into<String>) -> CallToolError {
    CallToolError::from_message(msg)
}

fn err(msg: impl Into<String>) -> ToolResult {
    Err(tool_err(msg))
}

fn load_file(cwd: &str) -> Result<(std::path::PathBuf, crate::schema::TargetsFile), CallToolError> {
    let dir = Path::new(cwd);
    let path =
        store::discover_anywhere(dir).ok_or_else(|| tool_err(no_targets_file_message(dir)))?;
    let file = store::load(&path).map_err(|e| tool_err(e.to_string()))?;
    Ok((path, file))
}

/// Explanatory text when `discover_anywhere` finds nothing. Routes
/// the agent to `bullseye_init` with the locked location prompt.
fn no_targets_file_message(cwd: &Path) -> String {
    format!(
        "no bullseye.yaml found for {} (checked in-repo walk-up from cwd and shadow tree under {}).\n\n{}",
        cwd.display(),
        config::external_root().display(),
        LOCATION_PROMPT,
    )
}

/// Substrings that uniquely identify a leaked Claude tool-call XML
/// envelope. They have no place in any caller-controlled string that
/// `bullseye` persists, so seeing one in a parameter value means the
/// agent serialised the tool call as XML and the harness's wrapper-
/// stripping was incomplete (🎯T20). Detecting them at the write
/// boundary keeps malformed envelopes from landing in `bullseye.yaml`
/// and forcing an unrelated future agent to debug a stray tag.
///
/// Generic tags like `<context>`/`</tags>` are NOT included — those
/// appear in legitimate prose. The four below are unambiguous markers
/// of the `<invoke name="..."><parameter name="...">…</parameter></invoke>`
/// protocol shape.
const TOOL_CALL_ENVELOPE_MARKERS: &[&str] =
    &["<invoke ", "</invoke>", "<parameter ", "</parameter>"];

/// Reject `value` if it contains any tool-call envelope marker. The
/// error names the field and the marker so the caller (and any human
/// reading the log) sees exactly what leaked. See 🎯T20.
fn check_no_envelope_leak(field: &str, value: &str) -> Result<(), String> {
    for marker in TOOL_CALL_ENVELOPE_MARKERS {
        if value.contains(marker) {
            return Err(format!(
                "{field} contains tool-call envelope marker `{marker}` — looks \
                 like XML tool-call syntax leaked into the parameter value. \
                 This usually means the agent serialised the call as XML and \
                 the closing tags weren't stripped. Re-issue the call with \
                 well-formed JSON parameters."
            ));
        }
    }
    Ok(())
}

/// Walk every caller-controlled string on a parsed `Target` and run
/// the envelope-leak check. Used by `handle_import`, where the input
/// is bulk-parsed from markdown and we don't have a per-field handler
/// signature to validate against.
fn check_target_no_envelope_leaks(id: &str, target: &crate::schema::Target) -> Result<(), String> {
    check_no_envelope_leak(&format!("{id}.name"), &target.name)?;
    check_no_envelope_leak(&format!("{id}.context"), &target.context)?;
    check_no_envelope_leak(&format!("{id}.origin"), &target.origin)?;
    if let Some(d) = &target.demonstration {
        check_no_envelope_leak(&format!("{id}.demonstration"), d)?;
    }
    if let Some(r) = &target.set_aside_reason {
        check_no_envelope_leak(&format!("{id}.set_aside_reason"), r)?;
    }
    for (i, a) in target.acceptance.iter().enumerate() {
        check_no_envelope_leak(&format!("{id}.acceptance[{i}]"), a)?;
    }
    for (i, t) in target.tags.iter().enumerate() {
        check_no_envelope_leak(&format!("{id}.tags[{i}]"), t)?;
    }
    for (i, e) in target.cross_depends.iter().enumerate() {
        if let Some(n) = &e.note {
            check_no_envelope_leak(&format!("{id}.cross_depends[{i}].note"), n)?;
        }
    }
    for (i, e) in target.cross_enables.iter().enumerate() {
        if let Some(n) = &e.note {
            check_no_envelope_leak(&format!("{id}.cross_enables[{i}].note"), n)?;
        }
    }
    Ok(())
}

/// Discover the `bullseye.yaml` path for `cwd` without loading its
/// contents. Mutating handlers call this first so they can enter the
/// locked read-modify-write block without holding the parse cache's
/// copy across the lock boundary.
fn discover_path(cwd: &str) -> Result<std::path::PathBuf, CallToolError> {
    let dir = Path::new(cwd);
    store::discover_anywhere(dir).ok_or_else(|| tool_err(no_targets_file_message(dir)))
}

pub fn handle_list(t: crate::tools::ListTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let targets: Vec<(&str, &Target)> = match t.filter.as_str() {
        "active" => file.active().into_iter().collect(),
        "achieved" => file.achieved().into_iter().collect(),
        "set_aside" => file.set_aside().into_iter().collect(),
        "all" => file.targets.iter().map(|(k, v)| (k.as_str(), v)).collect(),
        other => {
            return err(format!(
                "unknown filter: {other} (use active, achieved, set_aside, all)"
            ));
        }
    };

    let mut sorted: Vec<_> = targets;
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    let mut out = format!("# Targets ({})\nFile: {}\n\n", t.filter, path.display());
    for (id, target) in &sorted {
        out.push_str(&format!(
            "🎯{id} {name}\n  status: {status:?}  value: {v}, cost: {c}\n",
            name = target.name,
            status = target.status,
            v = target.value,
            c = target.cost,
        ));
        if let Some(reason) = &target.set_aside_reason {
            out.push_str(&format!("  reason: {reason}\n"));
        }
        if !target.tags.is_empty() {
            out.push_str(&format!("  tags: {}\n", target.tags.join(", ")));
        }
        out.push('\n');
    }
    out.push_str(&format!("{} target(s)", sorted.len()));

    text_result(out)
}

fn handle_get(t: crate::tools::GetTool) -> ToolResult {
    let (_path, file) = load_file(&t.cwd)?;

    let target = file
        .targets
        .get(&t.id)
        .ok_or_else(|| tool_err(format!("target {} not found", t.id)))?;

    let yaml = serde_yaml_ng::to_string(target).map_err(|e| tool_err(e.to_string()))?;
    text_result(format!("🎯{} {}\n\n{yaml}", t.id, target.name))
}

/// Auto-assign the next `TN` ID (ignoring sub-targets like `T1.2`).
fn next_top_level_id(file: &crate::schema::TargetsFile) -> String {
    let max_num = file
        .targets
        .keys()
        .filter_map(|k| {
            let num_str = k.strip_prefix('T')?;
            if num_str.contains('.') {
                None
            } else {
                num_str.parse::<u32>().ok()
            }
        })
        .max()
        .unwrap_or(0);
    format!("T{}", max_num + 1)
}

pub fn handle_put(t: crate::tools::PutTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;

    // Envelope-leak guard (🎯T20). Validate every caller-controlled
    // string before entering the locked mutation block — fail fast if
    // an XML tool-call envelope leaked into any field, so the file is
    // never written with the corruption.
    if let Some(s) = &t.name {
        check_no_envelope_leak("name", s).map_err(tool_err)?;
    }
    if let Some(s) = &t.context {
        check_no_envelope_leak("context", s).map_err(tool_err)?;
    }
    if let Some(s) = &t.origin {
        check_no_envelope_leak("origin", s).map_err(tool_err)?;
    }
    if let Some(items) = &t.acceptance {
        for (i, s) in items.iter().enumerate() {
            check_no_envelope_leak(&format!("acceptance[{i}]"), s).map_err(tool_err)?;
        }
    }
    if let Some(items) = &t.tags {
        for (i, s) in items.iter().enumerate() {
            check_no_envelope_leak(&format!("tags[{i}]"), s).map_err(tool_err)?;
        }
    }
    if let Some(items) = &t.depends_on {
        for (i, s) in items.iter().enumerate() {
            check_no_envelope_leak(&format!("depends_on[{i}]"), s).map_err(tool_err)?;
        }
    }
    if let Some(items) = &t.blocks {
        for (i, s) in items.iter().enumerate() {
            check_no_envelope_leak(&format!("blocks[{i}]"), s).map_err(tool_err)?;
        }
    }
    if let Some(items) = &t.verifies {
        for (i, s) in items.iter().enumerate() {
            check_no_envelope_leak(&format!("verifies[{i}]"), s).map_err(tool_err)?;
        }
    }

    struct Outcome {
        id: String,
        is_create: bool,
        target_name: String,
        injected_into: Vec<String>,
    }

    let parse_status_s = |s: &str| -> Result<Status, String> {
        match s {
            "identified" => Ok(Status::Identified),
            "converging" => Ok(Status::Converging),
            "achieved" => Ok(Status::Achieved),
            "set_aside" => Err(
                "status `set_aside` is not settable via bullseye_put — call \
                 `bullseye_set_aside(id, reason)` instead so the rationale is recorded \
                 alongside the status change"
                    .to_string(),
            ),
            other => Err(format!(
                "unknown status: {other} (use identified, converging, achieved)"
            )),
        }
    };
    let parse_kind_s = |s: &str| -> Result<crate::schema::Kind, String> {
        match s {
            "work" => Ok(crate::schema::Kind::Work),
            "verify" => Ok(crate::schema::Kind::Verify),
            other => Err(format!("unknown kind: {other} (use work or verify)")),
        }
    };

    let outcome = store::with_locked_mutation(&path, |file| -> Result<Outcome, String> {
        // Resolve target ID. None → auto-assign a new top-level ID.
        let (id, is_create) = match t.id.clone() {
            Some(explicit) => {
                let exists = file.targets.contains_key(&explicit);
                (explicit, !exists)
            }
            None => (next_top_level_id(file), true),
        };

        if is_create {
            // Creation path — name and acceptance are required.
            // value and cost are optional at repo scope (they are portfolio-scope
            // metadata consumed by cross-repo WSJF ranking, not by the repo-level
            // frontier ordering which uses distance-to-checkpoint instead).
            // Omitting them defaults to 0.0, which signals "not set at repo scope"
            // and is skipped by portfolio WSJF scoring.
            let name = t
                .name
                .clone()
                .ok_or_else(|| "name is required when creating a target".to_string())?;
            let value = t.value.unwrap_or(0.0);
            let cost = t.cost.unwrap_or(0.0);
            let acceptance = t
                .acceptance
                .clone()
                .filter(|a| !a.is_empty())
                .ok_or_else(|| "acceptance is required when creating a target".to_string())?;

            let kind = match t.kind.as_deref() {
                Some(k) => parse_kind_s(k)?,
                None => crate::schema::Kind::Work,
            };
            let status = match t.status.as_deref() {
                Some(s) => parse_status_s(s)?,
                None => Status::Identified,
            };

            let target = Target {
                name,
                kind,
                status,
                value,
                cost,
                showcase: t.showcase.unwrap_or(false),
                actual_cost: None,
                demonstration: None,
                set_aside_reason: None,
                acceptance,
                checks: Vec::new(),
                context: t.context.clone().unwrap_or_default(),
                gates: Vec::new(),
                depends_on: t.depends_on.clone().unwrap_or_default(),
                cross_depends: Vec::new(),
                cross_enables: Vec::new(),
                verifies: t.verifies.clone().unwrap_or_default(),
                rework: None,
                retry_budget: None,
                retries: 0,
                tags: t.tags.clone().unwrap_or_default(),
                strategy: None,
                origin: t.origin.clone().unwrap_or_else(|| "manual".to_string()),
                discovered: Local::now().date_naive(),
                achieved: if status == Status::Achieved {
                    Some(Local::now().date_naive())
                } else {
                    None
                },
            };
            file.targets.insert(id.clone(), target);
        } else {
            // Patch path — only provided fields change.

            // Kind is creation-only; reject early so the error surfaces
            // regardless of any other field state.
            if t.kind.is_some() {
                return Err("kind can only be set when creating a target".to_string());
            }

            // Parse the optional new status upfront so the
            // achieved-immutability check below and the field application
            // below share a single parse.
            let new_status: Option<Status> = match t.status.as_deref() {
                Some(s) => Some(parse_status_s(s)?),
                None => None,
            };

            // Safety — reject content edits on achieved targets unless
            // the same call is simultaneously re-opening them. Achieved
            // targets are historical artifacts; their content is
            // immutable until the human explicitly re-opens them by
            // patching `status: identified`. See 🎯T8.
            let target = file.targets.get_mut(&id).expect("existence checked above");
            let target_currently_achieved = target.status == Status::Achieved;
            let would_remain_achieved = match new_status {
                Some(s) => s == Status::Achieved,
                None => target_currently_achieved,
            };
            let content_edits_present = t.name.is_some()
                || t.value.is_some()
                || t.cost.is_some()
                || t.acceptance.is_some()
                || t.context.is_some()
                || t.tags.is_some()
                || t.origin.is_some()
                || t.depends_on.is_some()
                || t.verifies.is_some()
                || t.showcase.is_some();
            if target_currently_achieved && would_remain_achieved && content_edits_present {
                return Err(format!(
                    "🎯{id} is achieved — its content is immutable. Re-open it first by \
                     calling bullseye_put with `status: identified`, then apply content \
                     changes in a separate call. (Achieved targets are historical artifacts.)"
                ));
            }

            if let Some(ref name) = t.name {
                target.name = name.clone();
            }
            if let Some(value) = t.value {
                target.value = value;
            }
            if let Some(cost) = t.cost {
                target.cost = cost;
            }
            if let Some(ref acceptance) = t.acceptance {
                target.acceptance = acceptance.clone();
            }
            if let Some(ref context) = t.context {
                target.context = context.clone();
            }
            if let Some(ref tags) = t.tags {
                target.tags = tags.clone();
            }
            if let Some(ref origin) = t.origin {
                target.origin = origin.clone();
            }
            if let Some(ref deps) = t.depends_on {
                target.depends_on = deps.clone();
            }
            if let Some(showcase) = t.showcase {
                target.showcase = showcase;
            }
            if let Some(ref verifies) = t.verifies {
                target.verifies = verifies.clone();
            }
            if let Some(status) = new_status {
                target.status = status;
                if status == Status::Achieved && target.achieved.is_none() {
                    target.achieved = Some(Local::now().date_naive());
                }
            }
        }

        // Apply `blocks` sugar: inject `id` into each listed target's depends_on.
        let mut injected_into: Vec<String> = Vec::new();
        if let Some(ref blocks) = t.blocks {
            for other_id in blocks {
                if other_id == &id {
                    return Err(format!("target {id} cannot block itself"));
                }
                let other = file.targets.get_mut(other_id).ok_or_else(|| {
                    format!("blocks target {other_id} does not exist (cannot add dependency)")
                })?;
                if other.status == Status::Achieved {
                    return Err(format!(
                        "cannot inject dependency into 🎯{other_id} — it is achieved. \
                         Re-open it first by patching `status: identified`. See 🎯T8."
                    ));
                }
                if !other.depends_on.contains(&id) {
                    other.depends_on.push(id.clone());
                    injected_into.push(other_id.clone());
                }
            }
        }

        let target_name = file
            .targets
            .get(&id)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        Ok(Outcome {
            id,
            is_create,
            target_name,
            injected_into,
        })
    })
    .map_err(|e| tool_err(e.to_string()))?;

    let verb = if outcome.is_create {
        "Created"
    } else {
        "Updated"
    };
    let mut out = format!("{verb} 🎯{}", outcome.id);
    if !outcome.target_name.is_empty() {
        out.push_str(&format!(" \"{}\"", outcome.target_name));
    }
    if !outcome.injected_into.is_empty() {
        out.push_str(&format!(
            "\nInjected as dependency into: {}",
            outcome
                .injected_into
                .iter()
                .map(|s| format!("🎯{s}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push_str(&format!("\nFile: {}", path.display()));
    text_result(out)
}

pub fn handle_retire(t: crate::tools::RetireTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;

    // Envelope-leak guard (🎯T20).
    if let Some(s) = &t.demonstration {
        check_no_envelope_leak("demonstration", s).map_err(tool_err)?;
    }

    enum Outcome {
        AlreadyAchieved,
        Retired {
            name: String,
            cost: f64,
            demonstration: Option<String>,
        },
    }

    // Normalise the optional demonstration to its trimmed form once
    // — empty/whitespace-only strings are treated as "not provided"
    // so callers can't sidestep the showcase obligation by passing
    // a single space.
    let demonstration: Option<String> = t
        .demonstration
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let outcome = store::with_locked_mutation(&path, |file| -> Result<Outcome, String> {
        let target = file
            .targets
            .get_mut(&t.id)
            .ok_or_else(|| format!("target {} not found", t.id))?;
        if target.status == Status::Achieved {
            return Ok(Outcome::AlreadyAchieved);
        }
        // Showcase obligation (🎯T14): a target with `showcase: true`
        // must be demonstrated before it retires. A non-empty
        // `demonstration` argument is the recorded evidence; refuse
        // retirement otherwise so the agent can't shortcut a real
        // demo with a "tests pass" report.
        if target.showcase && demonstration.is_none() {
            return Err(format!(
                "🎯{id} is a showcase target — `bullseye_retire` requires a non-empty \
                 `demonstration` argument describing what was actually shown to the user \
                 (e.g. \"ran the binary with the player attached and shared a screen \
                 recording\"). Technical verification (\"tests pass\", \"builds clean\") \
                 is not a demonstration. Retire this target only after performing the \
                 user-visible step the showcase flag promised, and pass that step's \
                 description as `demonstration`.",
                id = t.id,
            ));
        }
        target.status = Status::Achieved;
        target.achieved = Some(Local::now().date_naive());
        if let Some(actual) = t.actual_cost {
            target.actual_cost = Some(actual);
        }
        if let Some(ref demo) = demonstration {
            target.demonstration = Some(demo.clone());
        }
        Ok(Outcome::Retired {
            name: target.name.clone(),
            cost: target.cost,
            demonstration: demonstration.clone(),
        })
    })
    .map_err(|e| tool_err(e.to_string()))?;

    match outcome {
        Outcome::AlreadyAchieved => text_result(format!("🎯{} is already achieved", t.id)),
        Outcome::Retired {
            name,
            cost,
            demonstration,
        } => {
            let mut out = format!("Retired 🎯{} \"{name}\"", t.id);
            if let Some(actual) = t.actual_cost {
                out.push_str(&format!("\nCost: estimated {cost}, actual {actual}"));
            }
            if let Some(demo) = demonstration {
                out.push_str(&format!("\nDemonstration: {demo}"));
            }
            text_result(out)
        }
    }
}

/// Set a target aside (🎯T18). Distinct from retirement: the target
/// is not delivered, but it is removed from the active set / frontier
/// and unblocks its dependents the same way an achieved target would.
/// Requires a non-empty trimmed `reason`. Refuses to set aside a
/// target that is already achieved (would be lying about delivery)
/// or already set aside with the same reason (no-op).
pub fn handle_set_aside(t: crate::tools::SetAsideTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;

    // Envelope-leak guard (🎯T20). Validate before trim/empty check
    // so a reason that's "just a leaked tag" reports the leak (more
    // actionable) rather than the empty-after-trim error.
    check_no_envelope_leak("reason", &t.reason).map_err(tool_err)?;

    let reason = t.reason.trim().to_string();
    if reason.is_empty() {
        return Err(tool_err(
            "bullseye_set_aside requires a non-empty `reason` describing why the target is being \
             parked / deferred / wont-fixed (e.g. \"deferred to v2.0\", \"won't fix — superseded \
             by 🎯T57\")."
                .to_string(),
        ));
    }

    enum Outcome {
        AlreadyAchieved,
        AlreadySetAside { existing_reason: String },
        SetAside { name: String, prior: Status },
    }

    let outcome = store::with_locked_mutation(&path, |file| -> Result<Outcome, String> {
        let target = file
            .targets
            .get_mut(&t.id)
            .ok_or_else(|| format!("target {} not found", t.id))?;
        if target.status == Status::Achieved {
            return Ok(Outcome::AlreadyAchieved);
        }
        if target.status == Status::SetAside {
            return Ok(Outcome::AlreadySetAside {
                existing_reason: target.set_aside_reason.clone().unwrap_or_default(),
            });
        }
        let prior = target.status;
        target.status = Status::SetAside;
        target.set_aside_reason = Some(reason.clone());
        Ok(Outcome::SetAside {
            name: target.name.clone(),
            prior,
        })
    })
    .map_err(|e| tool_err(e.to_string()))?;

    match outcome {
        Outcome::AlreadyAchieved => Err(tool_err(format!(
            "🎯{id} is already achieved — `bullseye_set_aside` is for targets that were not \
             delivered. If you need to revise the achievement record, edit bullseye.yaml \
             directly.",
            id = t.id,
        ))),
        Outcome::AlreadySetAside { existing_reason } => text_result(format!(
            "🎯{id} is already set aside.\nExisting reason: {existing_reason}\nNew reason \
             (not applied): {reason}",
            id = t.id,
        )),
        Outcome::SetAside { name, prior } => text_result(format!(
            "Set aside 🎯{id} \"{name}\" (was {prior:?})\nReason: {reason}",
            id = t.id,
        )),
    }
}

fn handle_frontier(t: crate::tools::FrontierTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let errors = graph::validate_blocking(&file);
    if !errors.is_empty() {
        return text_result(format!("# Validation errors\n\n{}", errors.join("\n")));
    }

    let targets = graph::frontier(&file);
    let ranked = graph::rank_frontier(&file, &targets);

    let mut out = format!("# Frontier\nFile: {}\n\n", path.display());
    out.push_str(graph::REPO_SCOPE_BANNER);
    if ranked.is_empty() {
        out.push_str("(no targets ready for work)\n");
    }
    for rf in &ranked {
        let ft = rf.target;
        let kind_label = match ft.kind {
            crate::schema::Kind::Work => {
                if file.targets.get(&ft.id).is_some_and(|t| t.showcase) {
                    " [showcase]"
                } else {
                    ""
                }
            }
            crate::schema::Kind::Verify => " [verify]",
        };
        let distance_label = match rf.distance {
            Some(0) => "checkpoint".to_string(),
            Some(d) => format!("dist={d}"),
            None => "no checkpoint reachable".to_string(),
        };
        out.push_str(&format!(
            "🎯{id} {name}{kind}  [{status:?}] — {dist}, fanout={fan}\n",
            id = ft.id,
            name = ft.name,
            kind = kind_label,
            status = ft.status,
            dist = distance_label,
            fan = rf.fanout,
        ));
        if !ft.verifies.is_empty() {
            let vs: Vec<String> = ft.verifies.iter().map(|v| format!("🎯{v}")).collect();
            out.push_str(&format!("  verifies: {}\n", vs.join(", ")));
        }
        if !ft.tags.is_empty() {
            out.push_str(&format!("  tags: {}\n", ft.tags.join(", ")));
        }
        out.push('\n');
    }
    out.push_str(&format!("{} target(s) ready for work", ranked.len()));

    text_result(out)
}

fn handle_rework(t: crate::tools::ReworkTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;

    // Envelope-leak guard (🎯T20). Check each input separately so the
    // error names the offending field; check before composing so the
    // pretty-printed JSON doesn't mask whichever payload was malformed.
    check_no_envelope_leak("diagnosis", &t.diagnosis).map_err(tool_err)?;
    if let Some(s) = &t.sawmill_failure {
        check_no_envelope_leak("sawmill_failure", s).map_err(tool_err)?;
    }
    if let Some(s) = &t.mnemo_history {
        check_no_envelope_leak("mnemo_history", s).map_err(tool_err)?;
    }

    let diagnosis = compose_rework_diagnosis(
        &t.diagnosis,
        t.sawmill_failure.as_deref(),
        t.mnemo_history.as_deref(),
    )
    .map_err(tool_err)?;

    let result = store::with_locked_mutation(&path, |file| -> Result<_, String> {
        ops::rework(file, &t.id, &diagnosis).map_err(|e| e.to_string())
    })
    .map_err(|e| tool_err(e.to_string()))?;

    let mut out = format!(
        "Rework triggered: 🎯{} → 🎯{} \"{}\"\nRetry {}",
        t.id, result.rework_id, result.rework_name, result.retries,
    );
    if let Some(budget) = result.budget {
        out.push_str(&format!(" of {budget}"));
        if result.budget_exhausted {
            out.push_str("\n\n⚠ RETRY BUDGET EXHAUSTED — escalate to human review");
        }
    }

    text_result(out)
}

/// Compose the rework diagnosis string from the free-form prose plus
/// optional structured payloads (sawmill failure, mnemo prior-attempt
/// history). Each structured payload is validated as JSON, pretty-
/// printed, and emitted as a fenced ```json block under a labelled
/// `##` header so the resulting context blob remains readable to a
/// human and re-parseable by tooling. Returns an error if either
/// payload fails to parse — better to reject early than persist
/// garbage into the target file.
fn compose_rework_diagnosis(
    prose: &str,
    sawmill_failure: Option<&str>,
    mnemo_history: Option<&str>,
) -> Result<String, String> {
    let mut sections: Vec<String> = Vec::new();
    let trimmed_prose = prose.trim();
    if !trimmed_prose.is_empty() {
        sections.push(trimmed_prose.to_string());
    }

    let mut append_json_section = |label: &str, raw: &str| -> Result<(), String> {
        let parsed: serde_json::Value =
            serde_json::from_str(raw).map_err(|e| format!("{label} is not valid JSON: {e}"))?;
        let pretty = serde_json::to_string_pretty(&parsed)
            .map_err(|e| format!("{label}: failed to serialise: {e}"))?;
        sections.push(format!("## {label}\n```json\n{pretty}\n```"));
        Ok(())
    };

    if let Some(raw) = sawmill_failure
        && !raw.trim().is_empty()
    {
        append_json_section("Sawmill failure", raw)?;
    }
    if let Some(raw) = mnemo_history
        && !raw.trim().is_empty()
    {
        append_json_section("Prior attempts (mnemo)", raw)?;
    }

    Ok(sections.join("\n\n"))
}

fn handle_tunnels(t: crate::tools::TunnelsTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let max_depth = t.max_depth.unwrap_or(2) as usize;
    let warnings = graph::tunnels(&file, max_depth);

    let mut out = format!(
        "# Tunnel Detection\nFile: {}\nMax depth: {max_depth}\n\n",
        path.display()
    );

    if warnings.is_empty() {
        out.push_str("No tunnels detected — all work targets have verification within range.\n");
    } else {
        for w in &warnings {
            match (&w.depth, &w.nearest_verify) {
                (None, _) => {
                    out.push_str(&format!(
                        "⚠ 🎯{} \"{}\" — no verification target covers this work\n\
                         \x20\x20Suggestion: add a verify target that checks this work\n\n",
                        w.target_id, w.target_name,
                    ));
                }
                (Some(depth), Some(verify)) => {
                    out.push_str(&format!(
                        "⚠ 🎯{} \"{}\" — nearest verification is 🎯{} ({depth} hops, max {max_depth})\n\
                         \x20\x20Suggestion: insert a verification checkpoint closer to this work\n\n",
                        w.target_id, w.target_name, verify,
                    ));
                }
                _ => {}
            }
        }
        out.push_str(&format!("{} tunnel(s) detected", warnings.len()));
    }

    text_result(out)
}

fn handle_validate(t: crate::tools::ValidateTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let errors = graph::validate_blocking(&file);
    let warnings = graph::validate_warnings(&file);
    if errors.is_empty() && warnings.is_empty() {
        return text_result(format!(
            "Valid: {} ({} targets)",
            path.display(),
            file.targets.len()
        ));
    }
    let mut out = format!("# Validation report for {}\n", path.display());
    if !errors.is_empty() {
        out.push_str(&format!(
            "\n## Errors (block frontier/convergence)\n\n{}\n",
            errors.join("\n")
        ));
    }
    if !warnings.is_empty() {
        out.push_str(&format!(
            "\n## Warnings (advisory; non-blocking)\n\n{}\n",
            warnings.join("\n")
        ));
    }
    text_result(out)
}

fn handle_graph(t: crate::tools::GraphTool) -> ToolResult {
    let (_path, file) = load_file(&t.cwd)?;
    let mermaid = graph::mermaid(&file);
    text_result(format!("```mermaid\n{mermaid}\n```"))
}

pub fn handle_init(t: crate::tools::InitTool) -> ToolResult {
    let dir = Path::new(&t.cwd);

    // `location` is required. An empty or unknown value returns the
    // locked prompt so the agent can ask the user.
    let location =
        Location::parse(&t.location).map_err(|e| tool_err(format!("{e}\n\n{LOCATION_PROMPT}",)))?;

    // Refuse if a targets file already exists in *either* location.
    // Two files for the same repo would make discovery ambiguous; the
    // user must resolve before init can proceed.
    if let Some(existing) = store::discover_anywhere(dir) {
        return err(format!(
            "bullseye.yaml already exists at {} — use bullseye_put to add targets",
            existing.display(),
        ));
    }

    let project = t.project_name.unwrap_or_else(|| {
        dir.file_name()
            .map_or("my-project".into(), |n| n.to_string_lossy().into_owned())
    });

    let path = store::create_at(dir, location, &project).map_err(tool_err)?;
    let _ = store::load(&path).map_err(|e| tool_err(e.to_string()))?;

    text_result(format!(
        "Created starter targets file at {} (location: {}).\n\
         Contains 1 sample target (🎯T1) — edit or replace it with your own.",
        path.display(),
        location.as_str(),
    ))
}

pub fn handle_import(t: crate::tools::ImportTool) -> ToolResult {
    let dir = Path::new(&t.cwd);

    let location =
        Location::parse(&t.location).map_err(|e| tool_err(format!("{e}\n\n{LOCATION_PROMPT}")))?;

    // Refuse to overwrite unless force is set. Look in both locations
    // — an existing file anywhere is a collision.
    if !t.force
        && let Some(existing) = store::discover_anywhere(dir)
    {
        return err(format!(
            "bullseye.yaml already exists at {} — use force: true to overwrite, \
             or use bullseye_put to modify existing targets",
            existing.display(),
        ));
    }

    // The import source must be an explicit path — no auto-discovery.
    let md_path = t
        .path
        .as_ref()
        .map(std::path::PathBuf::from)
        .ok_or_else(|| tool_err("path is required — specify the markdown file to import from"))?;

    let content = std::fs::read_to_string(&md_path)
        .map_err(|e| tool_err(format!("failed to read {}: {e}", md_path.display())))?;

    let file = import::parse_markdown(&content).map_err(tool_err)?;

    // Envelope-leak guard (🎯T20). The parsed markdown can carry
    // arbitrary string content into every target field; refuse to
    // persist any target whose content shows leaked tool-call syntax.
    for (id, target) in &file.targets {
        check_target_no_envelope_leaks(id, target).map_err(tool_err)?;
    }

    // Validate the parsed result.
    let errors = graph::validate(&file);
    if !errors.is_empty() {
        return err(format!(
            "Parsed {} targets but validation failed:\n{}",
            file.targets.len(),
            errors.join("\n")
        ));
    }

    // Write to the requested target path (in-repo or shadow).
    let yaml_path = store::target_path_for_new(dir, location);
    if let Some(parent) = yaml_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| tool_err(format!("failed to create {}: {e}", parent.display())))?;
    }
    store::with_locked_write(&yaml_path, &file).map_err(|e| tool_err(e.to_string()))?;

    text_result(format!(
        "Imported {} targets from {}\n\
         Written to {} (location: {})\n\
         Validation: OK",
        file.targets.len(),
        md_path.display(),
        yaml_path.display(),
        location.as_str(),
    ))
}

fn handle_startup_context(t: crate::tools::StartupContextTool) -> ToolResult {
    // Unlike most tools, startup_context is meant to be called
    // automatically at session start — possibly from a harness hook
    // that runs before the agent knows whether the repo uses bullseye
    // or whether its targets file is in a usable state. A raw error
    // would disrupt the session start and tell the agent nothing
    // useful, so degrade gracefully for the three "not my problem
    // right now" cases:
    //
    //   - no bullseye.yaml at all               → info message
    //   - bullseye.yaml present but unreadable  → info message + detail
    //   - bullseye.yaml present but won't parse → info message + detail
    //
    // The exception is [`store::LoadError::VersionTooNew`]: a newer
    // schema version is a correctness hazard that the user must
    // resolve (by upgrading bullseye), so we deliberately keep that
    // as a hard tool-call error. Silently degrading it would hide
    // the whole point of the version check.
    let dir = Path::new(&t.cwd);
    let Some(path) = store::discover_anywhere(dir) else {
        return text_result(graph::startup_context_no_file(&dir.display().to_string()));
    };
    match store::load(&path) {
        Ok(file) => {
            let recent_days = t.recent_days.unwrap_or(14);
            let out = graph::startup_context(&file, &path.display().to_string(), recent_days);
            text_result(out)
        }
        Err(e @ store::LoadError::VersionTooNew { .. }) => err(e.to_string()),
        Err(e @ (store::LoadError::Io(_) | store::LoadError::Parse(_))) => text_result(
            graph::startup_context_broken_file(&path.display().to_string(), &e.to_string()),
        ),
    }
}

fn handle_portfolio(t: crate::tools::PortfolioTool) -> ToolResult {
    // Default root is the user's workspace (`~/work`). Callers
    // wanting to scan external-mode repos pass the external shadow
    // root explicitly (e.g. `~/.local/share/bullseye`).
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/marcelo".to_string());
    let default_root = std::path::PathBuf::from(format!("{home}/work"));
    let root = t
        .root
        .as_deref()
        .map(|s| config::expand_tilde(Path::new(s)))
        .unwrap_or(default_root);

    if !root.is_dir() {
        return err(format!("{} is not a directory", root.display()));
    }

    let max_depth = t.max_depth.unwrap_or(5) as usize;
    let momentum: Vec<portfolio::Momentum> = t
        .momentum
        .unwrap_or_default()
        .into_iter()
        .map(|m| portfolio::Momentum {
            id: m.id,
            multiplier: m.multiplier,
        })
        .collect();
    let scan = portfolio::discover_repos(&root, max_depth, &momentum);
    text_result(portfolio::format_portfolio(&scan))
}

/// Repo root is the parent of `bullseye.yaml`.
pub(crate) fn repo_root_from_targets_path(path: &Path, fallback: &Path) -> std::path::PathBuf {
    path.parent().unwrap_or(fallback).to_path_buf()
}

pub fn handle_convergence(
    t: crate::tools::ConvergenceTool,
) -> Result<CallToolResult, CallToolError> {
    use crate::convergence;

    let dir = Path::new(&t.cwd);
    let Some(path) = store::discover_anywhere(dir) else {
        return text_result(graph::startup_context_no_file(&dir.display().to_string()));
    };
    let repo_root = repo_root_from_targets_path(&path, dir);

    // Convergence handles missing-hook / parse-error / etc. internally
    // — no short-circuits here. The only hard-error path is the schema
    // version check, which must always be surfaced loudly.
    let file = store::load(&path).map_err(|e| tool_err(e.to_string()))?;
    let momentum_map: Option<std::collections::BTreeMap<String, f64>> =
        t.momentum.as_ref().map(|entries| {
            let mut map = std::collections::BTreeMap::new();
            for entry in entries {
                map.insert(entry.id.clone(), entry.multiplier);
            }
            map
        });

    let out = convergence::convergence(
        &file,
        &path,
        &repo_root,
        momentum_map.as_ref(),
        t.skip_invariants.unwrap_or(false),
    );
    text_result(out)
}

fn handle_summary(t: crate::tools::SummaryTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;
    // The wire format is a list of `{id, multiplier}` entries (because
    // the rust-mcp-sdk derive can't schema-ify a keyed map), but
    // `graph::summary` still takes the canonical keyed form. Flatten
    // here. Duplicate ids keep the last multiplier — documented on
    // `MomentumEntry`.
    let momentum_map: Option<std::collections::BTreeMap<String, f64>> =
        t.momentum.as_ref().map(|entries| {
            let mut map = std::collections::BTreeMap::new();
            for entry in entries {
                map.insert(entry.id.clone(), entry.multiplier);
            }
            map
        });
    let out = graph::summary(
        &file,
        &path.display().to_string(),
        momentum_map.as_ref(),
        t.frontier_details.unwrap_or(false),
    );
    text_result(out)
}

fn handle_verify(t: crate::tools::VerifyTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let plan = ops::verify_plan(&file, &t.id).map_err(|e| tool_err(e.to_string()))?;

    // The tool returns both a JSON payload (machine-readable plan +
    // report template) and a human-readable summary. Bundle them as a
    // single markdown document so both audiences are served from one
    // text block — callers that want the JSON parse the fenced code
    // block, callers that want a quick scan read the bullet list.
    let json =
        serde_json::to_string_pretty(&plan).map_err(|e| tool_err(format!("serialize: {e}")))?;

    let mut out = format!(
        "# Verification plan for 🎯{} \"{}\"\nFile: {}\n\n\
         Bullseye cannot call sawmill directly. Execute each planned check via the \
         sawmill MCP server in order, then populate the report template with outcomes \
         and file/line-level failures.\n\n\
         ## Planned checks ({} total)\n\n",
        plan.target_id,
        plan.target_name,
        path.display(),
        plan.checks.len(),
    );

    for check in &plan.checks {
        out.push_str(&format!(
            "{}. sawmill tool `{}` — {}\n",
            check.index + 1,
            sawmill_tool_name(check.tool),
            check.description,
        ));
    }

    out.push_str("\n## Plan and report template (JSON)\n\n```json\n");
    out.push_str(&json);
    out.push_str("\n```\n");

    text_result(out)
}

fn sawmill_tool_name(tool: ops::SawmillTool) -> &'static str {
    match tool {
        ops::SawmillTool::CheckConventions => "check_conventions",
        ops::SawmillTool::Query => "query",
        ops::SawmillTool::CheckInvariants => "check_invariants",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::PutTool;

    /// Minimal targets.yaml with one achieved target (T1) and one
    /// identified target (T2). Exercises the achieved-immutability
    /// rule in [`handle_put`] (see 🎯T8).
    const FIXTURE_YAML: &str = r#"schema_version: 1
targets:
  T1:
    name: Old achieved target
    kind: work
    status: achieved
    value: 5
    cost: 3
    acceptance:
      - Did the thing
    context: Historical target, should be immutable
    origin: manual
    discovered: 2026-01-01
    achieved: 2026-02-01
  T2:
    name: Active target
    kind: work
    status: identified
    value: 8
    cost: 5
    acceptance:
      - Do the other thing
    origin: manual
    discovered: 2026-03-01
"#;

    /// RAII guard: redirects the external shadow root to an isolated
    /// tempdir so any external-mode discovery attempts can't touch the
    /// developer's real `~/.local/share/bullseye`. Cleared on drop.
    pub(super) struct ShadowScope {
        _shadow_tmp: tempfile::TempDir,
    }

    impl Drop for ShadowScope {
        fn drop(&mut self) {
            config::set_external_root_override(None);
        }
    }

    /// Stand up an in-repo fixture at a fresh tempdir and isolate the
    /// external shadow root so both discovery branches are deterministic.
    fn fixture() -> (tempfile::TempDir, ShadowScope, String) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bullseye.yaml"), FIXTURE_YAML).unwrap();
        let cwd = tmp.path().to_string_lossy().to_string();

        let shadow_tmp = tempfile::tempdir().unwrap();
        config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

        (
            tmp,
            ShadowScope {
                _shadow_tmp: shadow_tmp,
            },
            cwd,
        )
    }

    fn put(cwd: &str, id: &str) -> PutTool {
        PutTool {
            cwd: cwd.to_string(),
            id: Some(id.to_string()),
            name: None,
            value: None,
            cost: None,
            acceptance: None,
            context: None,
            kind: None,
            status: None,
            depends_on: None,
            showcase: None,
            blocks: None,
            verifies: None,
            origin: None,
            tags: None,
        }
    }

    fn load_target(cwd: &str, id: &str) -> Target {
        let dir = Path::new(cwd);
        let path = store::discover(dir).unwrap();
        let file = store::load(&path).unwrap();
        file.targets.get(id).unwrap().clone()
    }

    #[test]
    fn achieved_content_patch_is_rejected() {
        let (_tmp, _cfg, cwd) = fixture();
        let mut t = put(&cwd, "T1");
        t.name = Some("Sneaky rename".to_string());
        let result = handle_put(t);
        let err = result.expect_err("content patch on achieved must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("achieved") && msg.contains("immutable"),
            "error should mention achieved + immutable: {msg}"
        );
        // File unchanged.
        let t1 = load_target(&cwd, "T1");
        assert_eq!(t1.name, "Old achieved target");
        assert_eq!(t1.status, Status::Achieved);
    }

    #[test]
    fn identified_content_patch_is_allowed() {
        let (_tmp, _cfg, cwd) = fixture();
        let mut t = put(&cwd, "T2");
        t.name = Some("Renamed active target".to_string());
        handle_put(t).expect("content patch on identified must succeed");
        let t2 = load_target(&cwd, "T2");
        assert_eq!(t2.name, "Renamed active target");
        assert_eq!(t2.status, Status::Identified);
    }

    #[test]
    fn achieved_status_only_transition_is_allowed() {
        let (_tmp, _cfg, cwd) = fixture();
        let mut t = put(&cwd, "T1");
        t.status = Some("identified".to_string());
        handle_put(t).expect("status-only transition on achieved must succeed");
        let t1 = load_target(&cwd, "T1");
        assert_eq!(t1.status, Status::Identified);
        // Name and other content fields preserved.
        assert_eq!(t1.name, "Old achieved target");
    }

    #[test]
    fn achieved_atomic_reopen_with_content_is_allowed() {
        // A single call that both re-opens an achieved target AND
        // applies content edits is allowed — the reopen is applied
        // first, and the content edits land on the now-identified
        // target. This is the recovery path for a fat-fingered
        // historical ID: one call instead of two.
        let (_tmp, _cfg, cwd) = fixture();
        let mut t = put(&cwd, "T1");
        t.status = Some("identified".to_string());
        t.name = Some("Re-opened and renamed".to_string());
        handle_put(t).expect("atomic reopen + content must succeed");
        let t1 = load_target(&cwd, "T1");
        assert_eq!(t1.status, Status::Identified);
        assert_eq!(t1.name, "Re-opened and renamed");
    }

    #[test]
    fn blocks_injection_into_achieved_target_is_rejected() {
        // `blocks: [T1]` mutates T1.depends_on — a content edit on
        // an achieved target, just via the sugar. Same rule applies.
        let (_tmp, _cfg, cwd) = fixture();
        let mut t = put(&cwd, "T2");
        t.blocks = Some(vec!["T1".to_string()]);
        let result = handle_put(t);
        let err = result.expect_err("blocks into achieved must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("achieved"),
            "error should mention achieved: {msg}"
        );
        // T1's depends_on unchanged.
        let t1 = load_target(&cwd, "T1");
        assert!(t1.depends_on.is_empty());
    }

    #[test]
    fn kind_patch_is_still_rejected() {
        // Regression: the pre-existing "kind only on create" rule
        // must still fire on the patch path. This test also pins
        // the error ordering — kind rejection should happen before
        // the achieved-immutability check.
        let (_tmp, _cfg, cwd) = fixture();
        let mut t = put(&cwd, "T2");
        t.kind = Some("verify".to_string());
        let result = handle_put(t);
        let err = result.expect_err("kind patch must be rejected");
        let msg = format!("{err:?}");
        assert!(msg.contains("kind"), "error should mention kind: {msg}");
    }

    #[test]
    fn repo_root_is_parent_of_bullseye_yaml() {
        let path = Path::new("/tmp/myrepo/bullseye.yaml");
        let fallback = Path::new("/tmp/myrepo");
        let repo_root = repo_root_from_targets_path(path, fallback);
        assert_eq!(repo_root, Path::new("/tmp/myrepo"));
    }

    // --- compose_rework_diagnosis (🎯T1.5) ---------------------------------

    #[test]
    fn compose_rework_prose_only() {
        let out = compose_rework_diagnosis("linker error on arm64", None, None).unwrap();
        assert_eq!(out, "linker error on arm64");
    }

    #[test]
    fn compose_rework_trims_prose() {
        let out = compose_rework_diagnosis("  hello  \n", None, None).unwrap();
        assert_eq!(out, "hello");
    }

    #[test]
    fn compose_rework_all_empty_returns_empty() {
        // ops::rework treats empty diagnosis as "don't append" — preserve that.
        let out = compose_rework_diagnosis("", None, None).unwrap();
        assert!(out.is_empty());
        let out = compose_rework_diagnosis("   ", Some("   "), Some("")).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn compose_rework_with_sawmill() {
        let sawmill = r#"{"violations":[{"file":"src/lib.rs","line":42,"message":"x"}]}"#;
        let out = compose_rework_diagnosis("checks failed", Some(sawmill), None).unwrap();
        assert!(out.starts_with("checks failed\n\n## Sawmill failure\n```json\n"));
        assert!(out.contains("\"file\": \"src/lib.rs\""));
        assert!(out.trim_end().ends_with("```"));
    }

    #[test]
    fn compose_rework_with_mnemo() {
        let mnemo = r#"[{"attempt":1,"diagnosis":"timeout"}]"#;
        let out = compose_rework_diagnosis("retry needed", None, Some(mnemo)).unwrap();
        assert!(out.contains("## Prior attempts (mnemo)\n```json\n"));
        assert!(out.contains("\"attempt\": 1"));
    }

    #[test]
    fn compose_rework_full_payload_order_is_stable() {
        // Sawmill section precedes mnemo section so the most actionable
        // ("what failed") sits closest to the prose.
        let out = compose_rework_diagnosis("fail", Some("{\"a\":1}"), Some("[{\"b\":2}]")).unwrap();
        let saw = out.find("## Sawmill failure").expect("sawmill section");
        let mnemo = out
            .find("## Prior attempts (mnemo)")
            .expect("mnemo section");
        assert!(saw < mnemo, "sawmill should appear before mnemo");
    }

    #[test]
    fn compose_rework_rejects_invalid_sawmill_json() {
        let err = compose_rework_diagnosis("", Some("not-json"), None).unwrap_err();
        assert!(err.contains("Sawmill failure"), "error: {err}");
    }

    #[test]
    fn compose_rework_rejects_invalid_mnemo_json() {
        let err = compose_rework_diagnosis("", None, Some("[")).unwrap_err();
        assert!(err.contains("Prior attempts (mnemo)"), "error: {err}");
    }

    #[test]
    fn handle_rework_persists_structured_payload() {
        // End-to-end: drive handle_rework with both structured fields
        // and confirm the rework destination's context contains the
        // labelled fenced JSON blocks. Uses a fixture with a verify
        // target (V1) pointing at a work target (W1) it reworks.
        const FIXTURE: &str = r#"schema_version: 1
targets:
  W1:
    name: Work target
    kind: work
    status: converging
    value: 5
    cost: 3
    acceptance: [does the thing]
    origin: manual
    discovered: 2026-01-01
  V1:
    name: Verify target
    kind: verify
    status: identified
    value: 1
    cost: 1
    acceptance: [W1 passes]
    verifies: [W1]
    rework: W1
    origin: manual
    discovered: 2026-01-01
"#;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bullseye.yaml"), FIXTURE).unwrap();
        let cwd = tmp.path().to_string_lossy().to_string();

        let shadow_tmp = tempfile::tempdir().unwrap();
        config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
        let _guard = ShadowScope {
            _shadow_tmp: shadow_tmp,
        };

        let tool = crate::tools::ReworkTool {
            cwd: cwd.clone(),
            id: "V1".to_string(),
            diagnosis: "linker error".to_string(),
            sawmill_failure: Some(r#"{"file":"src/lib.rs","line":12}"#.to_string()),
            mnemo_history: Some(r#"[{"attempt":1}]"#.to_string()),
        };
        handle_rework(tool).expect("rework should succeed");

        let w1 = load_target(&cwd, "W1");
        assert!(w1.context.contains("Rework #1: linker error"));
        assert!(w1.context.contains("## Sawmill failure"));
        assert!(w1.context.contains("\"file\": \"src/lib.rs\""));
        assert!(w1.context.contains("## Prior attempts (mnemo)"));
        assert!(w1.context.contains("\"attempt\": 1"));
    }

    #[test]
    fn handle_rework_rejects_invalid_json_payload() {
        const FIXTURE: &str = r#"schema_version: 1
targets:
  W1:
    name: Work target
    kind: work
    status: converging
    value: 5
    cost: 3
    acceptance: [does the thing]
    origin: manual
    discovered: 2026-01-01
  V1:
    name: Verify target
    kind: verify
    status: identified
    value: 1
    cost: 1
    acceptance: [W1 passes]
    verifies: [W1]
    rework: W1
    origin: manual
    discovered: 2026-01-01
"#;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bullseye.yaml"), FIXTURE).unwrap();
        let cwd = tmp.path().to_string_lossy().to_string();

        let shadow_tmp = tempfile::tempdir().unwrap();
        config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
        let _guard = ShadowScope {
            _shadow_tmp: shadow_tmp,
        };

        let tool = crate::tools::ReworkTool {
            cwd: cwd.clone(),
            id: "V1".to_string(),
            diagnosis: String::new(),
            sawmill_failure: Some("not-json".to_string()),
            mnemo_history: None,
        };
        let err = handle_rework(tool).expect_err("invalid JSON should be rejected");
        let msg = format!("{err:?}");
        assert!(msg.contains("Sawmill failure"), "got: {msg}");

        // File should be unchanged: W1 still converging, no context appended.
        let w1 = load_target(&cwd, "W1");
        assert_eq!(w1.status, Status::Converging);
        assert!(!w1.context.contains("Sawmill failure"));
    }

    // --- envelope-leak guard (🎯T20) -------------------------------------

    #[test]
    fn envelope_marker_in_string_is_rejected() {
        for marker in TOOL_CALL_ENVELOPE_MARKERS {
            let value = format!("prose with {marker} leaked in");
            let err = check_no_envelope_leak("ctx", &value).unwrap_err();
            assert!(
                err.contains(marker),
                "error should name marker {marker}: {err}"
            );
            assert!(err.contains("ctx"), "error should name field: {err}");
        }
    }

    #[test]
    fn envelope_clean_string_passes() {
        // Generic angle brackets and `<context>` substrings (without
        // the protocol-specific tags) are legitimate prose.
        check_no_envelope_leak("ctx", "see <context> for details").unwrap();
        check_no_envelope_leak("ctx", "compare a < b vs a > b").unwrap();
        check_no_envelope_leak("ctx", "</context> mentioned in a doc").unwrap();
        check_no_envelope_leak("ctx", "").unwrap();
    }

    #[test]
    fn envelope_leak_in_put_context_is_rejected() {
        let (_tmp, _cfg, cwd) = fixture();
        let mut t = put(&cwd, "T2");
        t.context = Some("good prose\n</invoke>\nmore".to_string());
        let err = handle_put(t).expect_err("envelope leak in context must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("context") && msg.contains("</invoke>"),
            "error should name field and marker: {msg}"
        );
        // T2 unchanged.
        let t2 = load_target(&cwd, "T2");
        assert!(!t2.context.contains("</invoke>"));
    }

    #[test]
    fn envelope_leak_in_put_acceptance_is_rejected() {
        let (_tmp, _cfg, cwd) = fixture();
        let mut t = put(&cwd, "T2");
        t.acceptance = Some(vec![
            "fine".to_string(),
            "bad <parameter name=\"x\">".to_string(),
        ]);
        let err = handle_put(t).expect_err("envelope leak in acceptance must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("acceptance[1]"),
            "error should name index: {msg}"
        );
        assert!(
            msg.contains("<parameter "),
            "error should name marker: {msg}"
        );
    }

    #[test]
    fn envelope_leak_in_set_aside_reason_is_rejected() {
        let (_tmp, _cfg, cwd) = fixture();
        let tool = crate::tools::SetAsideTool {
            cwd: cwd.clone(),
            id: "T2".to_string(),
            reason: "won't fix </invoke>".to_string(),
        };
        let err = handle_set_aside(tool).expect_err("envelope leak must be rejected");
        let msg = format!("{err:?}");
        assert!(msg.contains("reason"), "error should name field: {msg}");
        assert!(msg.contains("</invoke>"), "error should name marker: {msg}");
        // T2 unchanged (still identified, no set_aside_reason).
        let t2 = load_target(&cwd, "T2");
        assert_eq!(t2.status, Status::Identified);
        assert!(t2.set_aside_reason.is_none());
    }

    #[test]
    fn envelope_leak_in_rework_diagnosis_is_rejected() {
        const FIXTURE: &str = r#"schema_version: 1
targets:
  W1:
    name: Work
    kind: work
    status: converging
    value: 5
    cost: 3
    acceptance: [does the thing]
    origin: manual
    discovered: 2026-01-01
  V1:
    name: Verify
    kind: verify
    status: identified
    value: 1
    cost: 1
    acceptance: [W1 passes]
    verifies: [W1]
    rework: W1
    origin: manual
    discovered: 2026-01-01
"#;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bullseye.yaml"), FIXTURE).unwrap();
        let cwd = tmp.path().to_string_lossy().to_string();
        let shadow_tmp = tempfile::tempdir().unwrap();
        config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
        let _guard = ShadowScope {
            _shadow_tmp: shadow_tmp,
        };

        let tool = crate::tools::ReworkTool {
            cwd: cwd.clone(),
            id: "V1".to_string(),
            diagnosis: "linker error </invoke>".to_string(),
            sawmill_failure: None,
            mnemo_history: None,
        };
        let err = handle_rework(tool).expect_err("envelope leak in diagnosis must be rejected");
        let msg = format!("{err:?}");
        assert!(msg.contains("diagnosis"), "error should name field: {msg}");
        assert!(msg.contains("</invoke>"), "error should name marker: {msg}");

        // W1 unchanged.
        let w1 = load_target(&cwd, "W1");
        assert_eq!(w1.status, Status::Converging);
        assert!(!w1.context.contains("</invoke>"));
    }

    #[test]
    fn envelope_leak_in_rework_sawmill_payload_is_rejected() {
        const FIXTURE: &str = r#"schema_version: 1
targets:
  W1:
    name: Work
    kind: work
    status: converging
    value: 5
    cost: 3
    acceptance: [does the thing]
    origin: manual
    discovered: 2026-01-01
  V1:
    name: Verify
    kind: verify
    status: identified
    value: 1
    cost: 1
    acceptance: [W1 passes]
    verifies: [W1]
    rework: W1
    origin: manual
    discovered: 2026-01-01
"#;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bullseye.yaml"), FIXTURE).unwrap();
        let cwd = tmp.path().to_string_lossy().to_string();
        let shadow_tmp = tempfile::tempdir().unwrap();
        config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
        let _guard = ShadowScope {
            _shadow_tmp: shadow_tmp,
        };

        // Note: the sawmill_failure value here is also invalid JSON,
        // but the envelope check fires first because it sits ahead of
        // compose_rework_diagnosis in the handler.
        let tool = crate::tools::ReworkTool {
            cwd,
            id: "V1".to_string(),
            diagnosis: "fail".to_string(),
            sawmill_failure: Some("<invoke name=\"x\">stuff</invoke>".to_string()),
            mnemo_history: None,
        };
        let err =
            handle_rework(tool).expect_err("envelope leak in sawmill payload must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("sawmill_failure"),
            "error should name field: {msg}"
        );
    }

    #[test]
    fn check_target_walker_finds_leak_in_cross_depends_note() {
        use crate::schema::{CrossEdge, Kind, Status, Target};
        let target = Target {
            name: "ok".into(),
            kind: Kind::Work,
            status: Status::Identified,
            value: 0.0,
            cost: 0.0,
            showcase: false,
            actual_cost: None,
            demonstration: None,
            set_aside_reason: None,
            acceptance: vec!["fine".into()],
            checks: Vec::new(),
            context: "fine".into(),
            gates: Vec::new(),
            depends_on: Vec::new(),
            cross_depends: vec![CrossEdge {
                repo: "marcelocantos/foo".into(),
                target: None,
                capability: Some("x".into()),
                note: Some("</parameter> leaked".into()),
            }],
            cross_enables: Vec::new(),
            verifies: Vec::new(),
            rework: None,
            retry_budget: None,
            retries: 0,
            tags: Vec::new(),
            strategy: None,
            origin: "manual".into(),
            discovered: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            achieved: None,
        };
        let err = check_target_no_envelope_leaks("T99", &target).unwrap_err();
        assert!(err.contains("T99.cross_depends[0].note"), "got: {err}");
        assert!(err.contains("</parameter>"), "got: {err}");
    }
}
