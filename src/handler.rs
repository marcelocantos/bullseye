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
use crate::git_commit;
use crate::graph;
use crate::id_alloc;
use crate::import;
use crate::ops;
use crate::portfolio;
use crate::repo_guard;
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
            TargetTools::RevertTool(t) => handle_revert(t),
            TargetTools::SetAsideTool(t) => handle_set_aside(t),
            TargetTools::SubdivideTool(t) => handle_subdivide(t),
            TargetTools::FrontierTool(t) => handle_frontier(t),
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

/// Refuse the mutation when the repo containing `targets_path` is in
/// a state that would silently lose the auto-commit (submodule clone,
/// detached HEAD). See [`crate::repo_guard`] for the full rationale
/// and the two unsafe states. `cwd` is the caller-supplied working
/// directory, used only in the error message — the structural check
/// runs against the *repo containing* `targets_path`, not `cwd`,
/// because `cwd` may sit deeper in the tree.
fn ensure_mutation_allowed(targets_path: &Path, cwd: &str) -> Result<(), CallToolError> {
    let repo_root = targets_path.parent().unwrap_or(Path::new("."));
    repo_guard::check_mutation_allowed(repo_root)
        .map_err(|guard| tool_err(guard.message(Path::new(cwd))))
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

/// Auto-assign the next top-level `T<N>` ID (ignoring sub-targets
/// like `T1.2`). Considers both the live in-memory file and the set
/// of historical IDs surfaced from git (🎯T28), so two branches that
/// haven't seen each other's commits don't both pick the same slot.
/// The historical set is empty for repos without git history (e.g.
/// external-mode shadow storage) and the function then behaves
/// exactly like the pre-T28 in-memory-only allocator.
fn next_top_level_id(
    file: &crate::schema::TargetsFile,
    historical: &std::collections::HashSet<String>,
) -> String {
    let in_memory = file.targets.keys().map(String::as_str);
    let from_history = historical.iter().map(String::as_str);
    let max_num = in_memory
        .chain(from_history)
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
    ensure_mutation_allowed(&path, &t.cwd)?;

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

    // Scan git history for every target ID ever assigned across all
    // branches/remotes (🎯T28). Done outside the locked mutation so
    // the (potentially expensive on first call per session)
    // subprocess isn't holding the file lock. The cache makes
    // subsequent calls cheap.
    let historical = id_alloc::historical_ids(&path);

    let outcome = store::with_locked_mutation(&path, |file| -> Result<Outcome, String> {
        // Resolve target ID. None → auto-assign a new top-level ID.
        let (id, is_create) = match t.id.clone() {
            Some(explicit) => {
                let exists = file.targets.contains_key(&explicit);
                (explicit, !exists)
            }
            None => (next_top_level_id(file, &historical), true),
        };

        // 🎯T28: an explicit-id create that collides with a target
        // recorded in git history (deleted, or on another branch)
        // is rejected — the whole point of reserving historical
        // IDs is so two branches can't independently end up with
        // the same ID pointing at different targets.
        if is_create && historical.contains(&id) {
            return Err(format!(
                "🎯{id} collides with a target recorded in git history (it may exist \
                 on another branch or have been deleted from the current tree). \
                 Pick a different ID, or omit `id` to auto-assign the next free slot."
            ));
        }

        if is_create {
            // Creation path — name and acceptance are required.
            // value and cost are optional at repo scope (they are portfolio-scope
            // metadata consumed by cross-repo WSJF ranking, not by the repo-level
            // frontier ordering which uses `depends_on` shape only).
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

            let status = match t.status.as_deref() {
                Some(s) => parse_status_s(s)?,
                None => Status::Identified,
            };

            let target = Target {
                name,
                status,
                value,
                cost,
                actual_cost: None,
                set_aside_reason: None,
                acceptance,
                checks: Vec::new(),
                context: t.context.clone().unwrap_or_default(),
                gates: Vec::new(),
                depends_on: t.depends_on.clone().unwrap_or_default(),
                cross_depends: Vec::new(),
                cross_enables: Vec::new(),
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
                || t.depends_on.is_some();
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

    git_commit::auto_commit_yaml(&path);

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
    ensure_mutation_allowed(&path, &t.cwd)?;

    enum Outcome {
        AlreadyAchieved,
        Retired { name: String, cost: f64 },
    }

    let outcome = store::with_locked_mutation(&path, |file| -> Result<Outcome, String> {
        let target = file
            .targets
            .get_mut(&t.id)
            .ok_or_else(|| format!("target {} not found", t.id))?;
        if target.status == Status::Achieved {
            return Ok(Outcome::AlreadyAchieved);
        }
        target.status = Status::Achieved;
        target.achieved = Some(Local::now().date_naive());
        if let Some(actual) = t.actual_cost {
            target.actual_cost = Some(actual);
        }
        Ok(Outcome::Retired {
            name: target.name.clone(),
            cost: target.cost,
        })
    })
    .map_err(|e| tool_err(e.to_string()))?;

    match outcome {
        Outcome::AlreadyAchieved => text_result(format!("🎯{} is already achieved", t.id)),
        Outcome::Retired { name, cost } => {
            git_commit::auto_commit_yaml(&path);

            let mut out = format!("Retired 🎯{} \"{name}\"", t.id);
            if let Some(actual) = t.actual_cost {
                out.push_str(&format!("\nCost: estimated {cost}, actual {actual}"));
            }
            text_result(out)
        }
    }
}

/// Re-open a previously-retired target (🎯T25). Replaces the v4
/// verify→rework retry-budget loop. Refuses to revert a target that
/// is not currently achieved — the operation is achievement-only;
/// to resume a set-aside target use `bullseye_put` with
/// `status: identified`, and to move an active target backwards
/// patch its status directly.
pub fn handle_revert(t: crate::tools::RevertTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;
    ensure_mutation_allowed(&path, &t.cwd)?;

    // Envelope-leak guard (🎯T20). Validate before trim/empty check
    // so a reason that's "just a leaked tag" reports the leak (more
    // actionable) rather than the empty-after-trim error.
    check_no_envelope_leak("reason", &t.reason).map_err(tool_err)?;

    let reason = t.reason.trim().to_string();
    if reason.is_empty() {
        return Err(tool_err(
            "bullseye_revert requires a non-empty `reason` describing what changed since \
             retirement (e.g. \"regression detected in CI run 42\", \"acceptance criterion \
             #3 was never actually checked\")."
                .to_string(),
        ));
    }

    let result = store::with_locked_mutation(&path, |file| -> Result<_, String> {
        ops::revert(file, &t.id, &reason).map_err(|e| e.to_string())
    })
    .map_err(|e| tool_err(e.to_string()))?;

    git_commit::auto_commit_yaml(&path);

    text_result(format!(
        "Reverted 🎯{} \"{}\" — status moved Achieved → Converging.\nReason: {reason}\nFile: {}",
        t.id,
        result.name,
        path.display(),
    ))
}

/// Set a target aside (🎯T18). Distinct from retirement: the target
/// is not delivered, but it is removed from the active set / frontier
/// and unblocks its dependents the same way an achieved target would.
/// Requires a non-empty trimmed `reason`. Refuses to set aside a
/// target that is already achieved (would be lying about delivery)
/// or already set aside with the same reason (no-op).
pub fn handle_set_aside(t: crate::tools::SetAsideTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;
    ensure_mutation_allowed(&path, &t.cwd)?;

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
        Outcome::SetAside { name, prior } => {
            git_commit::auto_commit_yaml(&path);

            let out = format!(
                "Set aside 🎯{id} \"{name}\" (was {prior:?})\nReason: {reason}",
                id = t.id,
            );
            text_result(out)
        }
    }
}

/// Split a parent target into children with one of three dependent
/// -rewiring modes (🎯T27.1). See `ops::subdivide` for full semantics.
pub fn handle_subdivide(t: crate::tools::SubdivideTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;
    ensure_mutation_allowed(&path, &t.cwd)?;

    // Envelope-leak guard (🎯T20): every caller-controlled string,
    // including those nested inside child specs, must be clean before
    // we enter the locked mutation.
    check_no_envelope_leak("parent", &t.parent).map_err(tool_err)?;
    check_no_envelope_leak("mode", &t.mode).map_err(tool_err)?;
    if let Some(s) = &t.retire_reason {
        check_no_envelope_leak("retire_reason", s).map_err(tool_err)?;
    }
    for (idx, child) in t.children.iter().enumerate() {
        if let Some(s) = &child.id {
            check_no_envelope_leak(&format!("children[{idx}].id"), s).map_err(tool_err)?;
        }
        check_no_envelope_leak(&format!("children[{idx}].name"), &child.name).map_err(tool_err)?;
        for (j, a) in child.acceptance.iter().enumerate() {
            check_no_envelope_leak(&format!("children[{idx}].acceptance[{j}]"), a)
                .map_err(tool_err)?;
        }
        if let Some(s) = &child.context {
            check_no_envelope_leak(&format!("children[{idx}].context"), s).map_err(tool_err)?;
        }
        if let Some(items) = &child.tags {
            for (j, s) in items.iter().enumerate() {
                check_no_envelope_leak(&format!("children[{idx}].tags[{j}]"), s)
                    .map_err(tool_err)?;
            }
        }
        if let Some(items) = &child.depends_on {
            for (j, s) in items.iter().enumerate() {
                check_no_envelope_leak(&format!("children[{idx}].depends_on[{j}]"), s)
                    .map_err(tool_err)?;
            }
        }
    }

    let mode = ops::SubdivideMode::parse(&t.mode).map_err(tool_err)?;

    let child_specs: Vec<ops::ChildSpec> = t
        .children
        .into_iter()
        .map(|c| ops::ChildSpec {
            id: c.id,
            name: c.name,
            acceptance: c.acceptance,
            context: c.context,
            tags: c.tags,
            depends_on: c.depends_on,
        })
        .collect();
    let retire_reason = t.retire_reason.clone();

    // 🎯T28: pull historical IDs from git before entering the locked
    // mutation so sub-target auto-assignment sees every slot ever
    // taken across branches, not just the current tree.
    let historical = id_alloc::historical_ids(&path);

    let result = store::with_locked_mutation(&path, |file| -> Result<_, String> {
        ops::subdivide(
            file,
            &t.parent,
            mode,
            child_specs,
            retire_reason.as_deref(),
            &historical,
        )
        .map_err(|e| e.to_string())
    })
    .map_err(|e| tool_err(e.to_string()))?;

    git_commit::auto_commit_yaml(&path);

    let mut out = format!(
        "Subdivided 🎯{parent} \"{name}\" — mode `{mode}`\nCreated: {created}",
        parent = result.parent_id,
        name = result.parent_name,
        mode = result.mode.as_str(),
        created = result
            .created_children
            .iter()
            .map(|id| format!("🎯{id}"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    if !result.rewired_dependents.is_empty() {
        out.push_str(&format!(
            "\nRewired dependents: {}",
            result
                .rewired_dependents
                .iter()
                .map(|id| format!("🎯{id}"))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    if result.parent_status_changed {
        let new_status = match result.mode {
            ops::SubdivideMode::Aggregate => "converging",
            ops::SubdivideMode::Retire => "achieved",
            ops::SubdivideMode::Add => "(no change)",
        };
        out.push_str(&format!("\nParent status: {new_status}"));
    }
    out.push_str(&format!("\nFile: {}", path.display()));
    text_result(out)
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
        out.push_str(&format!(
            "🎯{id} {name}  [{status:?}] — fanout={fan}\n",
            id = ft.id,
            name = ft.name,
            status = ft.status,
            fan = rf.fanout,
        ));
        if !ft.tags.is_empty() {
            out.push_str(&format!("  tags: {}\n", ft.tags.join(", ")));
        }
        out.push('\n');
    }
    out.push_str(&format!("{} target(s) ready for work", ranked.len()));

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

    // Guard 🎯T24 against the would-be repo root *before* writing the
    // file. For in-repo init, that's `dir`; for external init, the
    // shadow path isn't a git repo so the guard naturally passes.
    let probe_root = match location {
        Location::InRepo => dir.to_path_buf(),
        Location::External => store::target_path_for_new(dir, location)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dir.to_path_buf()),
    };
    repo_guard::check_mutation_allowed(&probe_root)
        .map_err(|guard| tool_err(guard.message(dir)))?;

    let path = store::create_at(dir, location, &project).map_err(tool_err)?;
    let _ = store::load(&path).map_err(|e| tool_err(e.to_string()))?;

    git_commit::auto_commit_yaml(&path);

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

    // Guard 🎯T24 against the would-be repo root before any write.
    // For in-repo writes this is the cwd; for external writes the
    // shadow tree is not a git repo so the guard passes.
    let probe_root = match location {
        Location::InRepo => dir.to_path_buf(),
        Location::External => yaml_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dir.to_path_buf()),
    };
    repo_guard::check_mutation_allowed(&probe_root)
        .map_err(|guard| tool_err(guard.message(dir)))?;

    if let Some(parent) = yaml_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| tool_err(format!("failed to create {}: {e}", parent.display())))?;
    }
    store::with_locked_write(&yaml_path, &file).map_err(|e| tool_err(e.to_string()))?;

    git_commit::auto_commit_yaml(&yaml_path);

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
    const FIXTURE_YAML: &str = r#"schema_version: 5
targets:
  T1:
    name: Old achieved target
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
            status: None,
            depends_on: None,
            blocks: None,
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
    fn repo_root_is_parent_of_bullseye_yaml() {
        let path = Path::new("/tmp/myrepo/bullseye.yaml");
        let fallback = Path::new("/tmp/myrepo");
        let repo_root = repo_root_from_targets_path(path, fallback);
        assert_eq!(repo_root, Path::new("/tmp/myrepo"));
    }

    // --- revert (🎯T25) ----------------------------------------------------

    #[test]
    fn revert_reopens_achieved_target() {
        let (_tmp, _cfg, cwd) = fixture();
        let tool = crate::tools::RevertTool {
            cwd: cwd.clone(),
            id: "T1".to_string(),
            reason: "regression detected in nightly run".to_string(),
        };
        handle_revert(tool).expect("revert should succeed");
        let t1 = load_target(&cwd, "T1");
        assert_eq!(t1.status, Status::Converging);
        assert!(t1.achieved.is_none(), "achieved date should be cleared");
        assert!(
            t1.context.contains("Reverted ") && t1.context.contains("regression detected"),
            "context should record the reason: {:?}",
            t1.context,
        );
        // Earlier context is preserved (appended, not replaced).
        assert!(t1.context.contains("Historical target"));
    }

    #[test]
    fn revert_rejects_active_target() {
        let (_tmp, _cfg, cwd) = fixture();
        let tool = crate::tools::RevertTool {
            cwd,
            id: "T2".to_string(),
            reason: "trying to revert an active target".to_string(),
        };
        let err = handle_revert(tool).expect_err("revert of active target must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("not achieved"),
            "error should mention `not achieved`: {msg}"
        );
    }

    #[test]
    fn revert_requires_reason() {
        let (_tmp, _cfg, cwd) = fixture();
        let tool = crate::tools::RevertTool {
            cwd,
            id: "T1".to_string(),
            reason: "   ".to_string(),
        };
        let err = handle_revert(tool).expect_err("empty reason must be rejected");
        let msg = format!("{err:?}");
        assert!(msg.contains("reason"), "error should mention reason: {msg}");
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
    fn envelope_leak_in_revert_reason_is_rejected() {
        let (_tmp, _cfg, cwd) = fixture();
        let tool = crate::tools::RevertTool {
            cwd: cwd.clone(),
            id: "T1".to_string(),
            reason: "regression </invoke>".to_string(),
        };
        let err = handle_revert(tool).expect_err("envelope leak in reason must be rejected");
        let msg = format!("{err:?}");
        assert!(msg.contains("reason"), "error should name field: {msg}");
        assert!(msg.contains("</invoke>"), "error should name marker: {msg}");
        // T1 unchanged — still achieved with original context.
        let t1 = load_target(&cwd, "T1");
        assert_eq!(t1.status, Status::Achieved);
    }

    #[test]
    fn check_target_walker_finds_leak_in_cross_depends_note() {
        use crate::schema::{CrossEdge, Status, Target};
        let target = Target {
            name: "ok".into(),
            status: Status::Identified,
            value: 0.0,
            cost: 0.0,
            actual_cost: None,
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
