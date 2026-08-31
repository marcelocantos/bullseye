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

use crate::api;
use crate::config::{self, LOCATION_PROMPT, Location};
use crate::github;
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
            // Core surface (🎯T45)
            TargetTools::OpenTool(t) => handle_open(t),
            TargetTools::QueryTool(t) => handle_query(t),
            TargetTools::CommitTool(t) => handle_commit(t),
            TargetTools::ApplyTool(t) => handle_apply(t),
            TargetTools::PlanChecksTool(t) => handle_plan_checks(t),
            // Compatibility shims + extended tools
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
            TargetTools::ResolveTool(t) => handle_resolve(t),
            TargetTools::StartupContextTool(t) => handle_startup_context(t),
            TargetTools::PortfolioTool(t) => handle_portfolio(t),
            TargetTools::SummaryTool(t) => handle_summary(t),
            TargetTools::VerifyTool(t) => handle_verify(t),
            TargetTools::ConvergenceTool(t) => handle_convergence(t),
            TargetTools::GithubSyncTool(t) => handle_github_sync(t),
            TargetTools::SyncPrioritiesTool(t) => handle_sync_priorities(t),
        }
    }
}

type ToolResult = Result<CallToolResult, CallToolError>;

fn text_result(text: String) -> ToolResult {
    Ok(CallToolResult::text_content(vec![text.into()]))
}

/// Prefer coded errors so agents can branch on `code=` without scraping.
fn tool_err(msg: impl Into<String>) -> CallToolError {
    let msg = msg.into();
    if msg.starts_with("code=") {
        return CallToolError::from_message(msg);
    }
    let code = api::classify_message(&msg);
    CallToolError::from_message(api::format_error(code, msg))
}

fn err(msg: impl Into<String>) -> ToolResult {
    Err(tool_err(msg))
}

fn coded_err(code: api::ErrorCode, msg: impl Into<String>) -> ToolResult {
    Err(CallToolError::from_message(api::format_error(
        code,
        msg.into(),
    )))
}

/// Mutation success envelope: structured header + human body + frontier.
fn mutation_text(
    path: &Path,
    op: &str,
    ids: &[String],
    changed: &[String],
    body: String,
) -> ToolResult {
    let frontier = api::frontier_ids_from_path(path);
    text_result(api::format_mutation_result(
        op, ids, changed, &frontier, path, &body,
    ))
}

/// Build [`github::GithubArgs`] from the MCP tool params. `pull_only` /
/// `push_only` invert into the `pull` / `push` enables (default: both on).
pub fn github_args_for(t: &crate::tools::GithubSyncTool) -> github::GithubArgs {
    github::GithubArgs {
        cwd: std::path::PathBuf::from(&t.cwd),
        repo: t.repo.clone(),
        label: t.label.clone(),
        assignee: t.assignee.clone(),
        pull: !t.push_only,
        push: !t.pull_only,
        dry_run: t.dry_run,
    }
}

/// MCP twin of `bullseye github sync` — mirrors GitHub issues ⇄ targets
/// via the shared [`github::run_with`] entry point.
pub fn handle_github_sync(t: crate::tools::GithubSyncTool) -> ToolResult {
    let args = github_args_for(&t);
    let today = Local::now().date_naive();
    let client = github::RealGh::new(args.cwd.clone());
    match github::run_with(&client, &args, today) {
        Ok(report) => text_result(report.summary()),
        Err(e) => err(format!("github sync failed: {e}")),
    }
}

/// MCP twin of `bullseye sync-priorities` — reuses the shared
/// [`crate::priorities::run_sync`] entry point so the two surfaces can't drift.
pub fn handle_sync_priorities(t: crate::tools::SyncPrioritiesTool) -> ToolResult {
    #[cfg(not(feature = "sqlite"))]
    {
        let _ = t;
        return err(
            "sync-priorities unavailable: this binary was built without the `sqlite` feature \
             (cargo build --no-default-features). Rebuild with default features or \
             `--features sqlite`.",
        );
    }
    #[cfg(feature = "sqlite")]
    {
        let mut args: Vec<String> = Vec::new();
        if let Some(db) = &t.db {
            args.push("--db".to_string());
            args.push(db.clone());
        }
        if let Some(root) = &t.root {
            args.push("--root".to_string());
            args.push(root.clone());
        }
        args.push("--horizon".to_string());
        args.push(t.horizon.clone());
        args.push("--max-depth".to_string());
        args.push(t.max_depth.to_string());
        match crate::priorities::run_sync(&args) {
            Ok(msg) => text_result(msg),
            Err(e) => err(format!("sync-priorities failed: {e}")),
        }
    }
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

/// Reject non-whitespace C0 controls in caller-controlled strings
/// before they can be serialised into `bullseye.yaml` (🎯T40). Newline,
/// carriage return, and tab remain allowed because existing prose
/// fields and YAML block scalars legitimately use them; bytes like
/// U+0001 are almost always terminal/editor/control-protocol damage.
fn check_no_invalid_control_chars(field: &str, value: &str) -> Result<(), String> {
    for ch in value.chars() {
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            return Err(format!(
                "{field} contains invalid control character U+{:04X} — \
                 refusing to write malformed target text to bullseye.yaml",
                ch as u32
            ));
        }
    }
    Ok(())
}

fn check_persisted_string(field: &str, value: &str) -> Result<(), String> {
    check_no_envelope_leak(field, value)?;
    check_no_invalid_control_chars(field, value)?;
    Ok(())
}

fn id_ends_in_zero_dotted_segment(id: &str) -> bool {
    id.strip_prefix('T')
        .and_then(|rest| rest.rsplit('.').next().filter(|_| rest.contains('.')))
        == Some("0")
}

fn check_explicit_target_id(field: &str, id: &str) -> Result<(), String> {
    check_persisted_string(field, id)?;
    if id_ends_in_zero_dotted_segment(id) {
        return Err(format!(
            "{field} `{id}` is ambiguous — dotted target IDs whose final segment is zero \
             are disallowed because humans conflate `T4` and `T4.0`. Omit `id` and let \
             Bullseye allocate the next child, or choose a non-zero child segment."
        ));
    }
    Ok(())
}

/// Walk every caller-controlled string on a parsed `Target` and run
/// the envelope-leak check. Used by `handle_import`, where the input
/// is bulk-parsed from markdown and we don't have a per-field handler
/// signature to validate against.
fn check_target_no_envelope_leaks(id: &str, target: &crate::schema::Target) -> Result<(), String> {
    check_explicit_target_id("target id", id)?;
    check_persisted_string(&format!("{id}.name"), &target.name)?;
    check_persisted_string(&format!("{id}.context"), &target.context)?;
    check_persisted_string(&format!("{id}.origin"), &target.origin)?;
    if let Some(r) = &target.set_aside_reason {
        check_persisted_string(&format!("{id}.set_aside_reason"), r)?;
    }
    if let Some(a) = &target.attestation {
        check_persisted_string(&format!("{id}.attestation"), a)?;
    }
    if let Some(ob) = &target.owned_by {
        check_persisted_string(&format!("{id}.owned_by.owner"), &ob.owner)?;
        check_persisted_string(&format!("{id}.owned_by.reason"), &ob.reason)?;
    }
    for (i, a) in target.acceptance.iter().enumerate() {
        check_persisted_string(&format!("{id}.acceptance[{i}]"), a)?;
    }
    for (i, t) in target.tags.iter().enumerate() {
        check_persisted_string(&format!("{id}.tags[{i}]"), t)?;
    }
    for (i, e) in target.cross_depends.iter().enumerate() {
        if let Some(n) = &e.note {
            check_persisted_string(&format!("{id}.cross_depends[{i}].note"), n)?;
        }
    }
    for (i, e) in target.cross_enables.iter().enumerate() {
        if let Some(n) = &e.note {
            check_persisted_string(&format!("{id}.cross_enables[{i}].note"), n)?;
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
/// a state that would silently lose the write (submodule clone,
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

// ─── Core surface (🎯T45) ─────────────────────────────────────────────

/// Discover / init / session snapshot.
pub fn handle_open(t: crate::tools::OpenTool) -> ToolResult {
    let dir = Path::new(&t.cwd);
    if store::discover_anywhere(dir).is_some() {
        return handle_startup_context(crate::tools::StartupContextTool {
            cwd: t.cwd,
            recent_days: t.recent_days,
        });
    }
    // Create when per-call location is set, or when server default_location
    // can supply one (🎯T61). Otherwise probe-only → not_initialized.
    let can_create = t
        .location
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
        || config::default_location().is_some();
    if can_create {
        handle_init(crate::tools::InitTool {
            cwd: t.cwd.clone(),
            location: t.location,
            project_name: t.project_name,
        })?;
        return handle_startup_context(crate::tools::StartupContextTool {
            cwd: t.cwd,
            recent_days: t.recent_days,
        });
    }
    coded_err(api::ErrorCode::NotInitialized, no_targets_file_message(dir))
}

/// Unified read path.
pub fn handle_query(t: crate::tools::QueryTool) -> ToolResult {
    let view = t.view.as_deref().unwrap_or("context");
    match view {
        "context" => handle_startup_context(crate::tools::StartupContextTool {
            cwd: t.cwd,
            recent_days: t.recent_days,
        }),
        "frontier" => handle_frontier(crate::tools::FrontierTool { cwd: t.cwd }),
        "target" => {
            let id = t.id.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "view=target requires `id`",
                ))
            })?;
            handle_get(crate::tools::GetTool { cwd: t.cwd, id })
        }
        "list" => handle_list(crate::tools::ListTool {
            cwd: t.cwd,
            filter: t.filter.unwrap_or_else(|| "active".to_string()),
        }),
        "summary" => handle_summary(crate::tools::SummaryTool {
            cwd: t.cwd,
            momentum: t.momentum,
            frontier_details: t.frontier_details,
        }),
        "graph" => handle_graph(crate::tools::GraphTool {
            cwd: t.cwd,
            scope: t.scope,
            nodes: t.nodes,
            seeds: t.seeds,
            expand: t.expand,
        }),
        "validate" => handle_validate(crate::tools::ValidateTool { cwd: t.cwd }),
        other => coded_err(
            api::ErrorCode::InvalidArgs,
            format!(
                "unknown view: {other} (use context, frontier, target, list, summary, graph, validate)"
            ),
        ),
    }
}

/// Build and run a single-target apply, reporting `op` in the envelope.
/// The `commit` verbs are sugar: each assembles the fragment its name
/// implies and hands it to the one engine (🎯T76).
fn commit_sugar(cwd: &str, id: String, frag: crate::apply::Fragment, op: &str) -> ToolResult {
    let mut targets = std::collections::BTreeMap::new();
    targets.insert(id, frag);
    apply_request_as(
        cwd,
        crate::apply::ApplyRequest {
            targets,
            ..Default::default()
        },
        op,
    )
}

/// Required `id` for the verbs that address an existing target.
fn require_id(id: Option<String>, op: &str) -> Result<String, CallToolError> {
    id.ok_or_else(|| {
        tool_err(api::format_error(
            api::ErrorCode::InvalidArgs,
            format!("op={op} requires `id`"),
        ))
    })
}

/// Unified mutation path: the named verbs, each sugar over `apply`.
pub fn handle_commit(t: crate::tools::CommitTool) -> ToolResult {
    match t.op.as_str() {
        "track" => handle_put(crate::tools::PutTool {
            reason: t.reason.clone(),
            cwd: t.cwd,
            id: t.id,
            child_of: t.child_of,
            name: t.name,
            value: t.value,
            cost: t.cost,
            acceptance: t.acceptance,
            context: t.context,
            status: t.status,
            depends_on: t.depends_on,
            blocks: t.blocks,
            origin: t.origin,
            tags: t.tags,
        }),
        "block" => {
            let id = t.id.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=block requires `id` (the blocking target)",
                ))
            })?;
            let blocks = t.blocks.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=block requires `blocks` (targets that gain this dependency)",
                ))
            })?;
            handle_put(crate::tools::PutTool {
                reason: None,
                cwd: t.cwd,
                id: Some(id),
                child_of: None,
                name: None,
                value: None,
                cost: None,
                acceptance: None,
                context: None,
                status: None,
                depends_on: None,
                blocks: Some(blocks),
                origin: None,
                tags: None,
            })
        }
        "split" => {
            let parent = t.parent.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=split requires `parent`",
                ))
            })?;
            let mode = t.mode.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=split requires `mode` (add, aggregate, retire)",
                ))
            })?;
            let children = t.children.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=split requires non-empty `children`",
                ))
            })?;
            handle_subdivide(crate::tools::SubdivideTool {
                cwd: t.cwd,
                parent,
                mode,
                children,
                retire_reason: t.retire_reason,
                tail: t.tail,
            })
        }
        "achieve" => {
            let id = require_id(t.id, "achieve")?;
            let attestation = t.attestation.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=achieve requires non-empty `attestation` — a short note on how you \
                     believe the target is met (SHA, test name, persona oracle, owner smoke, \
                     residual risk). Not formal proof.",
                ))
            })?;
            commit_sugar(
                &t.cwd,
                id,
                crate::apply::Fragment {
                    status: Some("achieved".to_string()),
                    attestation: Some(attestation),
                    actual_cost: t.actual_cost,
                    ..Default::default()
                },
                "achieve",
            )
        }
        "defer" => {
            let id = require_id(t.id, "defer")?;
            let reason = t.reason.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=defer requires non-empty `reason`",
                ))
            })?;
            commit_sugar(
                &t.cwd,
                id,
                crate::apply::Fragment {
                    status: Some("set_aside".to_string()),
                    reason: Some(reason),
                    ..Default::default()
                },
                "defer",
            )
        }
        "reopen" => {
            let id = require_id(t.id, "reopen")?;
            let reason = t.reason.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=reopen requires non-empty `reason`",
                ))
            })?;
            let result = commit_sugar(
                &t.cwd,
                id.clone(),
                crate::apply::Fragment {
                    if_status: Some("achieved".to_string()),
                    status: Some("converging".to_string()),
                    reason: Some(reason),
                    ..Default::default()
                },
                "reopen",
            );
            // The engine reports a generic precondition failure; this
            // verb has always explained the specific case, so restore
            // its wording rather than regress the message.
            result.map_err(|e| {
                let msg = e.to_string();
                if msg.contains("requires it to be Achieved") {
                    tool_err(api::format_error(
                        api::ErrorCode::Conflict,
                        format!(
                            "🎯{id} is not Achieved — `bullseye_revert` re-opens \
                             previously-retired targets. To resume a set-aside target, or to \
                             move an active one backwards, use `bullseye_apply` with \
                             `status: identified` and a `reason`."
                        ),
                    ))
                } else {
                    e
                }
            })
        }
        "assign" => {
            let id = require_id(t.id, "assign")?;
            let owner = t.owner.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=assign requires `owner`",
                ))
            })?;
            let reason = t.reason.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=assign requires non-empty `reason`",
                ))
            })?;
            commit_sugar(
                &t.cwd,
                id,
                crate::apply::Fragment {
                    owner: Some(owner),
                    reason: Some(reason),
                    ..Default::default()
                },
                "assign",
            )
        }
        "unassign" => {
            let id = require_id(t.id, "unassign")?;
            commit_sugar(
                &t.cwd,
                id,
                crate::apply::Fragment {
                    clear: Some(vec!["owner".to_string()]),
                    ..Default::default()
                },
                "unassign",
            )
        }
        "postpone" => handle_postpone(&t),
        "wake" => {
            let id = require_id(t.id, "wake")?;
            commit_sugar(
                &t.cwd,
                id,
                crate::apply::Fragment {
                    clear: Some(vec![
                        "postponed_until".to_string(),
                        "postpone_predicate".to_string(),
                    ]),
                    ..Default::default()
                },
                "wake",
            )
        }
        "rehash" => {
            let reason = t.reason.ok_or_else(|| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    "op=rehash requires non-empty `reason` (audit trail for authorized direct edit)",
                ))
            })?;
            handle_rehash(&t.cwd, &reason)
        }
        other => coded_err(
            api::ErrorCode::InvalidArgs,
            format!(
                "unknown op: {other} (use track, block, split, achieve, defer, reopen, assign, unassign, postpone, wake, rehash)"
            ),
        ),
    }
}

fn handle_postpone(t: &crate::tools::CommitTool) -> ToolResult {
    let id = t.id.clone().ok_or_else(|| {
        tool_err(api::format_error(
            api::ErrorCode::InvalidArgs,
            "op=postpone requires `id`",
        ))
    })?;
    let until = match t
        .postponed_until
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => Some(
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
                tool_err(api::format_error(
                    api::ErrorCode::InvalidArgs,
                    format!("postponed_until must be YYYY-MM-DD: {e}"),
                ))
            })?,
        ),
        None => None,
    };
    let pred = t
        .postpone_predicate
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if until.is_none() && pred.is_none() {
        return coded_err(
            api::ErrorCode::InvalidArgs,
            "op=postpone requires postponed_until and/or postpone_predicate",
        );
    }
    // postpone replaces the whole wake condition rather than merging
    // into it, so whichever half was not supplied is cleared.
    let mut clear = Vec::new();
    if until.is_none() {
        clear.push("postponed_until".to_string());
    }
    if pred.is_none() {
        clear.push("postpone_predicate".to_string());
    }
    commit_sugar(
        &t.cwd,
        id,
        crate::apply::Fragment {
            postponed_until: until,
            postpone_predicate: pred,
            clear: (!clear.is_empty()).then_some(clear),
            ..Default::default()
        },
        "postpone",
    )
}

fn handle_rehash(cwd: &str, reason: &str) -> ToolResult {
    let path = discover_path(cwd)?;
    ensure_mutation_allowed(&path, cwd)?;
    let reason = reason.trim();
    if reason.is_empty() {
        return coded_err(
            api::ErrorCode::InvalidArgs,
            "op=rehash requires non-empty reason",
        );
    }
    check_persisted_string("reason", reason).map_err(tool_err)?;
    // Load, append audit to a synthetic note? Just rewrite with fresh hash via save.
    let file = store::load(&path).map_err(|e| tool_err(e.to_string()))?;
    store::save(&path, &file).map_err(tool_err)?;
    // Append reason to a local audit by reloading and setting nothing — log in body.
    let hash = store::compute_content_hash(&file);
    let front = api::frontier_ids_from_path(&path);
    text_result(api::format_mutation_result(
        "rehash",
        &[],
        &[],
        &front,
        &path,
        &format!(
            "Recomputed content_hash=sha256:{hash} after authorized direct edit.
Reason: {reason}"
        ),
    ))
}

/// Plan-only check expansion (rename of verify semantics).
pub fn handle_plan_checks(t: crate::tools::PlanChecksTool) -> ToolResult {
    handle_verify(crate::tools::VerifyTool {
        cwd: t.cwd,
        id: t.id,
    })
}

// ─── Compatibility shims & extended tools ─────────────────────────────

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

    // 🎯T64: list answers regardless of validation state, but an
    // invalid target is called out inline so a reader who never runs
    // `view=validate` still sees it.
    let mut issues_by_target: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for issue in graph::validate_issues(&file) {
        issues_by_target
            .entry(issue.target)
            .or_default()
            .push(issue.message);
    }

    let mut out = format!("# Targets ({})\nFile: {}\n\n", t.filter, path.display());
    for (id, target) in &sorted {
        out.push_str(&format!(
            "🎯{id} {name}\n  status: {status:?}  value: {v}, cost: {c}\n",
            name = target.name,
            status = target.status,
            v = target.value,
            c = target.cost,
        ));
        for message in issues_by_target.get(*id).into_iter().flatten() {
            out.push_str(&format!("  INVALID: {message}\n"));
        }
        if let Some(reason) = &target.set_aside_reason {
            out.push_str(&format!("  reason: {reason}\n"));
        }
        if let Some(att) = &target.attestation {
            out.push_str(&format!("  attestation: {att}\n"));
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
    // 🎯T64: a single-target read always answers. Errors on *this*
    // target are appended as a note; errors elsewhere are irrelevant here.
    let mut out = format!("🎯{} {}\n\n{yaml}", t.id, target.name);
    let own_issues: Vec<String> = graph::validate_issues(&file)
        .into_iter()
        .filter(|i| i.target == t.id)
        .map(|i| i.message)
        .collect();
    if !own_issues.is_empty() {
        out.push_str(&format!("\nINVALID:\n- {}\n", own_issues.join("\n- ")));
    }
    text_result(out)
}

pub fn handle_put(t: crate::tools::PutTool) -> ToolResult {
    // Sugar over `apply` (🎯T76). `track` was always an upsert; the
    // engine is now the only thing that knows how to perform one, so
    // this reduces to naming the target and forwarding the fields.
    if t.id.is_some() && t.child_of.is_some() {
        return coded_err(
            api::ErrorCode::InvalidArgs,
            "`id` and `child_of` are mutually exclusive — provide `id` only when the exact \
             target ID is intentional, or omit it and set `child_of` to let Bullseye \
             allocate the next child",
        );
    }
    // A key beginning with `_` asks the engine to allocate; an explicit
    // id addresses that target directly.
    let key = t.id.clone().unwrap_or_else(|| "_new".to_string());
    let frag = crate::apply::Fragment {
        name: t.name,
        status: t.status,
        value: t.value,
        cost: t.cost,
        acceptance: t.acceptance,
        context: t.context,
        tags: t.tags,
        depends_on: t.depends_on,
        blocks: t.blocks,
        origin: t.origin,
        child_of: t.child_of,
        reason: t.reason,
        ..Default::default()
    };
    commit_sugar(&t.cwd, key, frag, "track")
}

pub fn handle_retire(t: crate::tools::RetireTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;
    ensure_mutation_allowed(&path, &t.cwd)?;

    // Write-boundary guards (🎯T20 / 🎯T40) before trim/empty check.
    check_explicit_target_id("id", &t.id).map_err(tool_err)?;
    check_persisted_string("attestation", &t.attestation).map_err(tool_err)?;

    let attestation = match crate::apply::normalize_attestation(&t.attestation) {
        Ok(a) => a,
        Err(msg) => {
            return Err(tool_err(api::format_error(
                api::ErrorCode::InvalidArgs,
                msg,
            )));
        }
    };

    enum Outcome {
        AlreadyAchieved,
        Retired { name: String, cost: f64 },
    }

    let outcome = store::with_locked_mutation(&path, |file| -> Result<Outcome, String> {
        let existing = file
            .targets
            .get(&t.id)
            .ok_or_else(|| format!("target {} not found", t.id))?;
        if existing.status == Status::Achieved {
            return Ok(Outcome::AlreadyAchieved);
        }
        ops::refuse_active_family(file, &t.id)?;
        let target = file
            .targets
            .get_mut(&t.id)
            .ok_or_else(|| format!("target {} not found", t.id))?;
        target.status = Status::Achieved;
        // Clear what the previous status owned before writing the
        // achieved-only fields (🎯T64) — e.g. a `set_aside_reason` from
        // an earlier disposition, which used to survive into the
        // achievement and make the target permanently invalid.
        target.clear_illegal_status_scoped_fields();
        let today = Local::now().date_naive();
        target.achieved = Some(today);
        target.attestation = Some(attestation.clone());
        // Context audit line so the note is visible even when readers
        // only skim context (dedicated field remains SoT for round-trip).
        let entry = format!("Achieved {today}: {attestation}");
        if target.context.is_empty() {
            target.context = entry;
        } else {
            target.context.push_str("\n\n");
            target.context.push_str(&entry);
        }
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
            let mut out = format!("Retired 🎯{} \"{name}\"\nAttestation: {attestation}", t.id);
            if let Some(actual) = t.actual_cost {
                out.push_str(&format!("\nCost: estimated {cost}, actual {actual}"));
            }
            mutation_text(
                &path,
                "achieve",
                std::slice::from_ref(&t.id),
                std::slice::from_ref(&t.id),
                out,
            )
        }
    }
}

/// Re-open a previously-retired target (🎯T25). Replaces the v4
/// verify→rework retry-budget loop. Refuses to revert a target that
/// is not currently achieved — the operation is achievement-only.
/// To resume a set-aside target, or to move an active one backwards,
/// use `bullseye_apply` with `status: identified` and a `reason`.
pub fn handle_revert(t: crate::tools::RevertTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;
    ensure_mutation_allowed(&path, &t.cwd)?;

    // Write-boundary guards (🎯T20 / 🎯T40). Validate before
    // trim/empty check so a reason that's "just a leaked tag" or
    // control byte reports the actionable corruption.
    check_explicit_target_id("id", &t.id).map_err(tool_err)?;
    check_persisted_string("reason", &t.reason).map_err(tool_err)?;

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

    let body = format!(
        "Reverted 🎯{} \"{}\" — status moved Achieved → Converging.\nReason: {reason}\nFile: {}",
        t.id,
        result.name,
        path.display(),
    );
    mutation_text(
        &path,
        "reopen",
        std::slice::from_ref(&t.id),
        std::slice::from_ref(&t.id),
        body,
    )
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

    // Write-boundary guards (🎯T20 / 🎯T40). Validate before
    // trim/empty check so a reason that's "just a leaked tag" or
    // control byte reports the actionable corruption.
    check_explicit_target_id("id", &t.id).map_err(tool_err)?;
    check_persisted_string("reason", &t.reason).map_err(tool_err)?;

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
        // 🎯T64: drop achieved-only / active-only residue before
        // recording the new disposition.
        target.clear_illegal_status_scoped_fields();
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
             delivered. To revise the achievement record, reopen it and patch in one \
             `bullseye_apply`: `targets: {{{id}: {{status: identified, reason: <why>, \
             ...}}}}`.",
            id = t.id,
        ))),
        Outcome::AlreadySetAside { existing_reason } => text_result(format!(
            "🎯{id} is already set aside.\nExisting reason: {existing_reason}\nNew reason \
             (not applied): {reason}",
            id = t.id,
        )),
        Outcome::SetAside { name, prior } => {
            let out = format!(
                "Set aside 🎯{id} \"{name}\" (was {prior:?})\nReason: {reason}",
                id = t.id,
            );
            mutation_text(
                &path,
                "defer",
                std::slice::from_ref(&t.id),
                std::slice::from_ref(&t.id),
                out,
            )
        }
    }
}

/// Split a parent target into children with one of three dependent
/// -rewiring modes (🎯T27.1). See `ops::subdivide` for full semantics.
pub fn handle_subdivide(t: crate::tools::SubdivideTool) -> ToolResult {
    let path = discover_path(&t.cwd)?;
    ensure_mutation_allowed(&path, &t.cwd)?;

    // Write-boundary guards (🎯T20 / 🎯T40): every caller-controlled
    // string, including those nested inside child specs, must be clean
    // before we enter the locked mutation.
    check_explicit_target_id("parent", &t.parent).map_err(tool_err)?;
    check_persisted_string("mode", &t.mode).map_err(tool_err)?;
    if let Some(s) = &t.retire_reason {
        check_persisted_string("retire_reason", s).map_err(tool_err)?;
    }
    if let Some(items) = &t.tail {
        for (j, s) in items.iter().enumerate() {
            check_explicit_target_id(&format!("tail[{j}]"), s).map_err(tool_err)?;
        }
    }
    for (idx, child) in t.children.iter().enumerate() {
        if let Some(s) = &child.id {
            check_explicit_target_id(&format!("children[{idx}].id"), s).map_err(tool_err)?;
        }
        check_persisted_string(&format!("children[{idx}].name"), &child.name).map_err(tool_err)?;
        for (j, a) in child.acceptance.iter().enumerate() {
            check_persisted_string(&format!("children[{idx}].acceptance[{j}]"), a)
                .map_err(tool_err)?;
        }
        if let Some(s) = &child.context {
            check_persisted_string(&format!("children[{idx}].context"), s).map_err(tool_err)?;
        }
        if let Some(items) = &child.tags {
            for (j, s) in items.iter().enumerate() {
                check_persisted_string(&format!("children[{idx}].tags[{j}]"), s)
                    .map_err(tool_err)?;
            }
        }
        if let Some(items) = &child.depends_on {
            for (j, s) in items.iter().enumerate() {
                check_explicit_target_id(&format!("children[{idx}].depends_on[{j}]"), s)
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
    let tail = t.tail.clone();

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
            tail.as_deref(),
            &historical,
        )
        .map_err(|e| e.to_string())
    })
    .map_err(|e| tool_err(e.to_string()))?;

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
    let mut ids = result.created_children.clone();
    ids.insert(0, result.parent_id.clone());
    let mut changed = result.created_children.clone();
    changed.push(result.parent_id.clone());
    changed.extend(result.rewired_dependents.iter().cloned());
    mutation_text(&path, "split", &ids, &changed, out)
}

fn handle_frontier(t: crate::tools::FrontierTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    // 🎯T64: a validation error on one target names that target and
    // drops it from the ready set — it does not replace the answer.
    // Returning only the errors here is what made a one-field mistake
    // in the jevons ledger brick every read the PO had.
    let tolerant = graph::frontier_tolerant(&file);
    let ranked = graph::rank_frontier(&file, &tolerant.targets);

    let mut out = format!("# Frontier\nFile: {}\n\n", path.display());
    out.push_str(&graph::degraded_read_banner(&tolerant));
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
    let opts = mermaid_opts_from_params(t.scope.as_deref(), t.nodes, t.seeds, t.expand)
        .map_err(tool_err)?;
    let mermaid = graph::mermaid_with_opts(&file, &opts);
    text_result(format!("```mermaid\n{mermaid}\n```"))
}

/// Build [`graph::MermaidOpts`] from MCP/CLI graph params (🎯T57).
fn mermaid_opts_from_params(
    scope: Option<&str>,
    nodes: Option<Vec<String>>,
    seeds: Option<Vec<String>>,
    expand: Option<Vec<String>>,
) -> Result<graph::MermaidOpts, String> {
    let scope = match scope {
        Some(s) => graph::MermaidScope::parse(s)?,
        None => graph::MermaidScope::default(),
    };
    let expand = match expand {
        Some(parts) if !parts.is_empty() => {
            let joined = parts.join(",");
            graph::MermaidExpand::parse_list(&joined)?
        }
        _ => graph::MermaidExpand::default(),
    };
    Ok(graph::MermaidOpts {
        scope,
        nodes: nodes.unwrap_or_default(),
        seeds: seeds.unwrap_or_default(),
        expand,
    })
}

pub fn handle_init(t: crate::tools::InitTool) -> ToolResult {
    let dir = Path::new(&t.cwd);

    // Per-call location overrides server default_location; omit both → prompt (🎯T61).
    let location = config::resolve_create_location(t.location.as_deref()).map_err(tool_err)?;

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

    text_result(format!(
        "Created starter targets file at {} (location: {}).\n\
         Contains 1 sample target (🎯T1) — edit or replace it with your own.",
        path.display(),
        location.as_str(),
    ))
}

pub fn handle_import(t: crate::tools::ImportTool) -> ToolResult {
    let dir = Path::new(&t.cwd);

    // Same create-location resolution as init (🎯T61).
    let location = config::resolve_create_location(t.location.as_deref()).map_err(tool_err)?;

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
            let mut out = graph::startup_context(&file, &path.display().to_string(), recent_days);
            // 🎯T41 content-hash check
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let check = store::check_content_hash(&raw, &file);
                if let Some(w) = store::hash_mismatch_warning(&path, &check) {
                    out.push_str("## Store integrity\n\n");
                    out.push_str(&w);
                    out.push_str("\n\n");
                }
            }
            // 🎯T52 issuepipe env UX
            out.push_str(&issuepipe_env_status());
            // 🎯T54 binary/schema already in startup_context header
            text_result(out)
        }
        Err(e @ store::LoadError::VersionTooNew { .. }) => err(e.to_string()),
        Err(e @ (store::LoadError::Io(_) | store::LoadError::Parse(_))) => text_result(
            graph::startup_context_broken_file(&path.display().to_string(), &e.to_string()),
        ),
    }
}

/// Report complete / half / absent issuepipe event-path env (🎯T52).
fn issuepipe_env_status() -> String {
    let url = std::env::var("BULLSEYE_ISSUEPIPE_URL").unwrap_or_default();
    let token = std::env::var("BULLSEYE_ISSUEPIPE_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .unwrap_or_default();
    let opt_in = std::env::var("BULLSEYE_ISSUEPIPE_OPT_IN").unwrap_or_default();
    let interval = std::env::var("BULLSEYE_ISSUEPIPE_INTERVAL").unwrap_or_else(|_| "5".into());
    issuepipe_env_status_from(&url, &token, &opt_in, &interval)
}

/// Pure issuepipe env UX (testable without mutating process env).
fn issuepipe_env_status_from(url: &str, token: &str, opt_in: &str, interval: &str) -> String {
    let set = [
        (!url.is_empty(), "BULLSEYE_ISSUEPIPE_URL"),
        (!token.is_empty(), "BULLSEYE_ISSUEPIPE_TOKEN|GITHUB_TOKEN"),
        (!opt_in.is_empty(), "BULLSEYE_ISSUEPIPE_OPT_IN"),
    ];
    let n = set.iter().filter(|(ok, _)| *ok).count();
    if n == 0 {
        return String::new();
    }
    let mut out = String::from("## Issuepipe event-path env\n\n");
    if n == 3 {
        let interval = if interval.is_empty() { "5" } else { interval };
        out.push_str(&format!(
            "Complete: continuous consumer should run (default interval {interval}s; \
             `bullseye issues-poll --interval {interval}` or MCP spawn with `--features github-issues`).\n\n"
        ));
    } else {
        let missing: Vec<&str> = set.iter().filter(|(ok, _)| !*ok).map(|(_, n)| *n).collect();
        out.push_str(&format!(
            "HALF-CONFIGURED WARNING: missing {} — continuous consumer will not start; \
             silent no-op avoided. Set all of URL, token, and OPT_IN.\n\n",
            missing.join(", ")
        ));
    }
    out
}

pub fn handle_resolve(t: crate::tools::ResolveTool) -> ToolResult {
    use crate::resolve;
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/marcelo".to_string());
    let default_root = std::path::PathBuf::from(format!("{home}/work"));
    let root = t
        .workspace_root
        .as_deref()
        .map(|s| config::expand_tilde(Path::new(s)))
        .unwrap_or(default_root);
    if !root.is_dir() {
        return err(format!(
            "workspace_root {} is not a directory",
            root.display()
        ));
    }
    match resolve::resolve(&root, &t.reference) {
        Ok(path) => text_result(path.display().to_string()),
        Err(e) => err(e.to_string()),
    }
}

pub fn handle_portfolio(t: crate::tools::PortfolioTool) -> ToolResult {
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

/// Validate every caller-controlled string in an apply request before
/// the locked mutation, so the file is never written with protocol or
/// control-byte corruption (🎯T20 / 🎯T40). `apply` must not become a
/// bypass of the guards the per-verb handlers apply.
fn check_apply_strings(req: &crate::apply::ApplyRequest) -> Result<(), String> {
    for (key, frag) in &req.targets {
        if !crate::apply::is_allocation_slot(key) {
            check_explicit_target_id("target key", key)?;
        }
        let label = |f: &str| format!("{key}.{f}");
        for (field, value) in [
            ("name", &frag.name),
            ("context", &frag.context),
            ("origin", &frag.origin),
            ("attestation", &frag.attestation),
            ("reason", &frag.reason),
            ("owner", &frag.owner),
            ("postpone_predicate", &frag.postpone_predicate),
        ] {
            if let Some(s) = value {
                check_persisted_string(&label(field), s)?;
            }
        }
        for (field, items) in [("acceptance", &frag.acceptance), ("tags", &frag.tags)] {
            if let Some(items) = items {
                for (i, s) in items.iter().enumerate() {
                    check_persisted_string(&format!("{key}.{field}[{i}]"), s)?;
                }
            }
        }
        for (field, items) in [("depends_on", &frag.depends_on), ("blocks", &frag.blocks)] {
            if let Some(items) = items {
                for (i, s) in items.iter().enumerate() {
                    check_explicit_target_id(&format!("{key}.{field}[{i}]"), s)?;
                }
            }
        }
        if let Some(parent) = &frag.child_of {
            check_explicit_target_id(&label("child_of"), parent)?;
        }
    }
    for (i, id) in req.remove.iter().enumerate() {
        check_explicit_target_id(&format!("remove[{i}]"), id)?;
    }
    Ok(())
}

/// Run an apply request against the ledger discovered from `cwd`.
///
/// The one write path. Both surfaces — the MCP `bullseye_apply` tool
/// and `bullseye apply` — funnel through here, which is what makes
/// surface parity structural rather than a thing to remember (🎯T76).
pub fn apply_request(cwd: &str, req: crate::apply::ApplyRequest) -> ToolResult {
    apply_request_as(cwd, req, "apply")
}

/// [`apply_request`], reporting `op` in the result envelope.
///
/// The eleven `commit --op` verbs are sugar over apply, but each keeps
/// its own label so a caller (and the test suite) still sees the verb
/// it asked for rather than the mechanism underneath.
pub fn apply_request_as(cwd: &str, req: crate::apply::ApplyRequest, op: &str) -> ToolResult {
    let path = discover_path(cwd)?;
    ensure_mutation_allowed(&path, cwd)?;
    check_apply_strings(&req).map_err(tool_err)?;

    // Scan git history for every ID ever assigned (🎯T28), outside the
    // lock: the subprocess is expensive on first call and must not be
    // run while holding the file lock.
    let historical = id_alloc::historical_ids(&path);

    // `with_locked_mutation` flattens the closure's error into a
    // string, which would drop the stable error code. Carry the code
    // out alongside so a refusal reaches the caller as (code, message)
    // rather than as bare prose.
    let mut refusal: Option<api::ErrorCode> = None;
    let report = store::with_locked_mutation(&path, |file| {
        crate::apply::apply(file, &req, &historical).map_err(|e| {
            refusal = Some(e.code);
            e.message
        })
    });
    let report = match report {
        Ok(r) => r,
        Err(e) => {
            return match refusal {
                Some(code) => coded_err(code, e.to_string()),
                None => Err(tool_err(e.to_string())),
            };
        }
    };

    let mut lines: Vec<String> = Vec::new();
    for id in &report.created {
        let name = read_target_name(&path, id);
        lines.push(format!("Created 🎯{id}{name}"));
    }
    for id in &report.updated {
        let name = read_target_name(&path, id);
        lines.push(format!("Updated 🎯{id}{name}"));
    }
    for id in &report.removed {
        lines.push(format!("Removed 🎯{id}"));
    }
    for (blocker, blocked) in &report.injected {
        lines.push(format!(
            "🎯{blocker} injected as a dependency of 🎯{blocked}"
        ));
    }
    if lines.is_empty() {
        lines.push("No changes — the fragment already matched the ledger.".to_string());
    }
    lines.push(format!("File: {}", path.display()));

    let ids = report.created.clone();
    mutation_text(&path, op, &ids, &report.changed(), lines.join("\n"))
}

/// Best-effort target name for the result body. A missing name is a
/// display concern only, never a reason to fail a completed write.
fn read_target_name(path: &Path, id: &str) -> String {
    match store::load(path) {
        Ok(file) => file
            .targets
            .get(id)
            .map(|t| format!(" \"{}\"", t.name))
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// MCP entry point for the single write verb (🎯T76). Parses the
/// fragment text, then hands off to the shared [`apply_request`] that
/// the CLI also calls.
pub fn handle_apply(t: crate::tools::ApplyTool) -> ToolResult {
    let mut req: crate::apply::ApplyRequest =
        serde_yaml_ng::from_str(&t.fragment).map_err(|e| {
            tool_err(api::format_error(
                api::ErrorCode::InvalidArgs,
                format!(
                    "fragment is not a valid apply document: {e}\n\
                     Expected shape: targets: {{T55: {{value: 8}}}} — see the tool \
                     description for the full field list."
                ),
            ))
        })?;
    if t.base.is_some() {
        req.base = t.base;
    }
    if t.reason.is_some() {
        req.reason = t.reason;
    }
    apply_request(&t.cwd, req)
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
            reason: None,
            cwd: cwd.to_string(),
            id: Some(id.to_string()),
            child_of: None,
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
        // 🎯T76 tightened this: re-opening an achieved target requires a
        // reason on every path. `put --status identified` used to be a
        // back door around the rule `commit --op reopen` always applied.
        t.reason = Some("acceptance turned out to be wrong".to_string());
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
        t.reason = Some("fat-fingered historical ID".to_string());
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
            attestation: None,
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
            owned_by: None,
            postponed_until: None,
            postpone_predicate: None,
        };
        let err = check_target_no_envelope_leaks("T99", &target).unwrap_err();
        assert!(err.contains("T99.cross_depends[0].note"), "got: {err}");
        assert!(err.contains("</parameter>"), "got: {err}");
    }

    #[test]
    fn issuepipe_env_half_config_warns() {
        let msg = super::issuepipe_env_status_from("https://example.invalid/events", "", "", "5");
        assert!(
            msg.contains("HALF-CONFIGURED"),
            "expected half-config warning, got: {msg}"
        );
        assert!(msg.contains("BULLSEYE_ISSUEPIPE_OPT_IN"), "{msg}");
        assert!(
            msg.contains("BULLSEYE_ISSUEPIPE_TOKEN|GITHUB_TOKEN"),
            "{msg}"
        );
    }

    #[test]
    fn issuepipe_env_complete_recommends_continuous() {
        let msg =
            super::issuepipe_env_status_from("https://example.invalid/events", "tok", "1", "7");
        assert!(
            msg.contains("Complete:") && msg.contains("continuous") && msg.contains("interval 7s"),
            "expected complete continuous recommend, got: {msg}"
        );
    }

    #[test]
    fn issuepipe_env_absent_is_silent() {
        assert_eq!(super::issuepipe_env_status_from("", "", "", "5"), "");
    }
}
