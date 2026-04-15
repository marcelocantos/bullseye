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

use crate::config::{self, Config, ConfigError, Mode, Storage};
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
            TargetTools::ConfigureTool(t) => handle_configure(t),
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

/// Load the machine-wide storage config, or translate its absence /
/// brokenness into a tool error the agent can act on. The
/// `NotConfigured` variant embeds the locked first-run prompt and
/// directs the agent to call `bullseye_configure`.
fn ensure_configured() -> Result<Config, CallToolError> {
    config::load().map_err(|e| tool_err(e.to_string()))
}

fn load_file(cwd: &str) -> Result<(std::path::PathBuf, crate::schema::TargetsFile), CallToolError> {
    let cfg = ensure_configured()?;
    let dir = Path::new(cwd);
    let path = store::discover_with_config(dir, &cfg)
        .ok_or_else(|| tool_err(no_targets_file_message(dir, &cfg)))?;
    let file = store::load(&path).map_err(|e| tool_err(e.to_string()))?;
    Ok((path, file))
}

/// Load the targets file, or create an empty one if none exists.
fn load_or_create_file(
    cwd: &str,
) -> Result<(std::path::PathBuf, crate::schema::TargetsFile), CallToolError> {
    let cfg = ensure_configured()?;
    let dir = Path::new(cwd);
    if let Some(path) = store::discover_with_config(dir, &cfg) {
        let file = store::load(&path).map_err(|e| tool_err(e.to_string()))?;
        Ok((path, file))
    } else {
        let path = store::create_default_with_config(dir, &cfg).map_err(tool_err)?;
        let file = store::load(&path).map_err(|e| tool_err(e.to_string()))?;
        Ok((path, file))
    }
}

/// Explanatory text when `discover_with_config` finds nothing. Names
/// the expected location so the agent (and the user) can see where
/// bullseye looked.
fn no_targets_file_message(cwd: &Path, cfg: &Config) -> String {
    match cfg.storage.mode {
        Mode::InRepo => format!(
            "no bullseye.yaml found for {} (in_repo mode; expected somewhere at or above the cwd)",
            cwd.display()
        ),
        Mode::External => format!(
            "no bullseye.yaml found for {} (external mode; expected under shadow root {})",
            cwd.display(),
            cfg.effective_root().display()
        ),
    }
}

fn save_file(path: &Path, file: &crate::schema::TargetsFile) -> Result<(), CallToolError> {
    store::save(path, file).map_err(tool_err)
}

pub fn handle_list(t: crate::tools::ListTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let targets: Vec<(&str, &Target)> = match t.filter.as_str() {
        "active" => file.active().into_iter().collect(),
        "achieved" => file.achieved().into_iter().collect(),
        "all" => file.targets.iter().map(|(k, v)| (k.as_str(), v)).collect(),
        other => {
            return err(format!(
                "unknown filter: {other} (use active, achieved, all)"
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

fn parse_status(s: &str) -> Result<Status, CallToolError> {
    match s {
        "identified" => Ok(Status::Identified),
        "converging" => Ok(Status::Converging),
        "achieved" => Ok(Status::Achieved),
        other => Err(tool_err(format!(
            "unknown status: {other} (use identified, converging, achieved)"
        ))),
    }
}

fn parse_kind(s: &str) -> Result<crate::schema::Kind, CallToolError> {
    match s {
        "work" => Ok(crate::schema::Kind::Work),
        "verify" => Ok(crate::schema::Kind::Verify),
        other => Err(tool_err(format!(
            "unknown kind: {other} (use work or verify)"
        ))),
    }
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

fn handle_put(t: crate::tools::PutTool) -> ToolResult {
    let (path, mut file) = load_or_create_file(&t.cwd)?;

    // Resolve target ID. None → auto-assign a new top-level ID.
    let (id, is_create) = match t.id.clone() {
        Some(explicit) => {
            let exists = file.targets.contains_key(&explicit);
            (explicit, !exists)
        }
        None => (next_top_level_id(&file), true),
    };

    if is_create {
        // Creation path — name/value/cost/acceptance are required.
        let name = t
            .name
            .clone()
            .ok_or_else(|| tool_err("name is required when creating a target"))?;
        let value = t
            .value
            .ok_or_else(|| tool_err("value is required when creating a target"))?;
        let cost = t
            .cost
            .ok_or_else(|| tool_err("cost is required when creating a target"))?;
        let acceptance = t
            .acceptance
            .clone()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| tool_err("acceptance is required when creating a target"))?;

        let kind = match t.kind.as_deref() {
            Some(k) => parse_kind(k)?,
            None => crate::schema::Kind::Work,
        };
        let status = match t.status.as_deref() {
            Some(s) => parse_status(s)?,
            None => Status::Identified,
        };

        let target = Target {
            name,
            kind,
            status,
            value,
            cost,
            observable: t.observable.unwrap_or(false),
            actual_cost: None,
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
            return err("kind can only be set when creating a target");
        }

        // Parse the optional new status upfront so the
        // achieved-immutability check below and the field application
        // below share a single parse.
        let new_status: Option<Status> = match t.status.as_deref() {
            Some(s) => Some(parse_status(s)?),
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
            || t.observable.is_some();
        if target_currently_achieved && would_remain_achieved && content_edits_present {
            return err(format!(
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
        if let Some(observable) = t.observable {
            target.observable = observable;
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
                return err(format!("target {id} cannot block itself"));
            }
            let other = file.targets.get_mut(other_id).ok_or_else(|| {
                tool_err(format!(
                    "blocks target {other_id} does not exist (cannot add dependency)"
                ))
            })?;
            if other.status == Status::Achieved {
                return err(format!(
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

    save_file(&path, &file)?;

    let verb = if is_create { "Created" } else { "Updated" };
    let mut out = format!("{verb} 🎯{id}");
    if let Some(target) = file.targets.get(&id) {
        out.push_str(&format!(" \"{}\"", target.name));
    }
    if !injected_into.is_empty() {
        out.push_str(&format!(
            "\nInjected as dependency into: {}",
            injected_into
                .iter()
                .map(|s| format!("🎯{s}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push_str(&format!("\nFile: {}", path.display()));
    text_result(out)
}

fn handle_retire(t: crate::tools::RetireTool) -> ToolResult {
    let (path, mut file) = load_file(&t.cwd)?;

    let target = file
        .targets
        .get_mut(&t.id)
        .ok_or_else(|| tool_err(format!("target {} not found", t.id)))?;

    if target.status == Status::Achieved {
        return text_result(format!("🎯{} is already achieved", t.id));
    }

    let target = file.targets.get_mut(&t.id).unwrap();
    target.status = Status::Achieved;
    target.achieved = Some(Local::now().date_naive());
    if let Some(actual) = t.actual_cost {
        target.actual_cost = Some(actual);
    }

    let name = target.name.clone();
    let cost = target.cost;

    save_file(&path, &file)?;

    let mut out = format!("Retired 🎯{} \"{name}\"", t.id);
    if let Some(actual) = t.actual_cost {
        out.push_str(&format!("\nCost: estimated {cost}, actual {actual}"));
    }
    text_result(out)
}

fn handle_frontier(t: crate::tools::FrontierTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let errors = graph::validate(&file);
    if !errors.is_empty() {
        return text_result(format!("# Validation errors\n\n{}", errors.join("\n")));
    }

    let targets = graph::frontier(&file);

    let mut out = format!("# Frontier\nFile: {}\n\n", path.display());
    if targets.is_empty() {
        out.push_str("(no targets ready for work)\n");
    }
    for ft in &targets {
        let kind_label = match ft.kind {
            crate::schema::Kind::Work => "",
            crate::schema::Kind::Verify => " [verify]",
        };
        out.push_str(&format!(
            "🎯{id} {name}{kind}\n  status: {status:?}\n",
            id = ft.id,
            name = ft.name,
            kind = kind_label,
            status = ft.status,
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
    out.push_str(&format!("{} target(s) ready for work", targets.len()));

    text_result(out)
}

fn handle_rework(t: crate::tools::ReworkTool) -> ToolResult {
    let (path, mut file) = load_file(&t.cwd)?;

    let result =
        ops::rework(&mut file, &t.id, &t.diagnosis).map_err(|e| tool_err(e.to_string()))?;

    save_file(&path, &file)?;

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

    let errors = graph::validate(&file);
    if errors.is_empty() {
        text_result(format!(
            "Valid: {} ({} targets)",
            path.display(),
            file.targets.len()
        ))
    } else {
        text_result(format!(
            "# Validation errors in {}\n\n{}",
            path.display(),
            errors.join("\n")
        ))
    }
}

fn handle_graph(t: crate::tools::GraphTool) -> ToolResult {
    let (_path, file) = load_file(&t.cwd)?;
    let mermaid = graph::mermaid(&file);
    text_result(format!("```mermaid\n{mermaid}\n```"))
}

pub fn handle_init(t: crate::tools::InitTool) -> ToolResult {
    let cfg = ensure_configured()?;
    let dir = Path::new(&t.cwd);

    // Refuse if a targets file already exists for this cwd under the
    // configured mode.
    if store::discover_with_config(dir, &cfg).is_some() {
        return err("bullseye.yaml already exists — use bullseye_put to add targets");
    }

    let project = t.project_name.unwrap_or_else(|| {
        dir.file_name()
            .map_or("my-project".into(), |n| n.to_string_lossy().into_owned())
    });

    let path = store::create_starter_with_config(dir, &cfg, &project).map_err(tool_err)?;
    let _ = store::load(&path).map_err(|e| tool_err(e.to_string()))?;

    text_result(format!(
        "Created starter targets file at {}\n\
         Contains 1 sample target (🎯T1) — edit or replace it with your own.",
        path.display(),
    ))
}

fn handle_import(t: crate::tools::ImportTool) -> ToolResult {
    let cfg = ensure_configured()?;
    let dir = Path::new(&t.cwd);

    // Refuse to overwrite unless force is set.
    if !t.force && store::discover_with_config(dir, &cfg).is_some() {
        return err(
            "bullseye.yaml already exists — use force: true to overwrite, \
             or use bullseye_put to modify existing targets",
        );
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

    // Validate the parsed result.
    let errors = graph::validate(&file);
    if !errors.is_empty() {
        return err(format!(
            "Parsed {} targets but validation failed:\n{}",
            file.targets.len(),
            errors.join("\n")
        ));
    }

    // Write to the configured target path (in-repo or shadow).
    let yaml_path = store::target_path_for_new(dir, &cfg);
    if let Some(parent) = yaml_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| tool_err(format!("failed to create {}: {e}", parent.display())))?;
    }
    store::save(&yaml_path, &file).map_err(tool_err)?;

    text_result(format!(
        "Imported {} targets from {}\n\
         Written to {}\n\
         Validation: OK",
        file.targets.len(),
        md_path.display(),
        yaml_path.display(),
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
    // startup_context is a graceful-degradation tool — but config
    // state is a different kind of problem: a session that begins
    // without the agent knowing about the missing config will silently
    // misbehave on the *next* call. Surface NotConfigured as a hard
    // error here so the agent is prompted to run bullseye_configure
    // before anything else.
    let cfg = ensure_configured()?;
    let dir = Path::new(&t.cwd);
    let Some(path) = store::discover_with_config(dir, &cfg) else {
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
    let cfg = ensure_configured()?;
    // Default root depends on mode: in_repo mode defaults to ~/work as
    // before; external mode defaults to the configured shadow root so
    // the portfolio scan walks the same tree that discover does.
    let default_root: std::path::PathBuf = match cfg.storage.mode {
        Mode::InRepo => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/marcelo".to_string());
            std::path::PathBuf::from(format!("{home}/work"))
        }
        Mode::External => cfg.effective_root(),
    };
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

    let cfg = ensure_configured()?;
    let dir = Path::new(&t.cwd);
    let Some(path) = store::discover_with_config(dir, &cfg) else {
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

pub fn handle_configure(t: crate::tools::ConfigureTool) -> ToolResult {
    let mode = Mode::parse(&t.mode).map_err(tool_err)?;

    // Validate root: only meaningful for external mode; a supplied
    // root in in_repo mode is a user mistake worth flagging rather
    // than silently keeping.
    let root = match (mode, t.root.as_deref()) {
        (Mode::InRepo, Some(r)) if !r.is_empty() => {
            return err(format!(
                "root is only meaningful with mode: external (got root={r:?} with mode: in_repo). \
                 Re-call with mode: external, or omit root."
            ));
        }
        (Mode::External, Some(r)) if !r.is_empty() => Some(config::expand_tilde(Path::new(r))),
        _ => None,
    };

    let cfg = Config {
        storage: Storage { mode, root },
    };
    config::save(&cfg).map_err(tool_err)?;

    let summary = match cfg.storage.mode {
        Mode::InRepo => format!(
            "Configured bullseye: mode=in_repo.\nConfig written to {}.\n\
             Targets will be stored as bullseye.yaml inside each repo you work on.",
            config::config_path().display(),
        ),
        Mode::External => format!(
            "Configured bullseye: mode=external, root={}.\nConfig written to {}.\n\
             Targets will be stored under the shadow tree at that root.",
            cfg.effective_root().display(),
            config::config_path().display(),
        ),
    };
    text_result(summary)
}

/// The top-level `ConfigError` type is useful to callers who need to
/// discriminate paths; the handler here treats every variant as a
/// tool error. Kept as a thin wrapper so the compiler checks we
/// handle each variant if more are added.
#[allow(dead_code)]
fn format_config_error(e: &ConfigError) -> String {
    e.to_string()
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

    /// RAII guard: clears the thread-local config override on drop so
    /// a test panic doesn't leak state into the next test sharing the
    /// same worker thread.
    pub(super) struct ConfigScope {
        _config_tmp: tempfile::TempDir,
    }

    impl Drop for ConfigScope {
        fn drop(&mut self) {
            config::set_config_dir_override(None);
        }
    }

    /// Stand up an in-repo-mode config in an isolated config dir, plus
    /// a tempdir with the fixture `bullseye.yaml` at its root. Handler
    /// tests drop both guards together to guarantee isolation.
    fn fixture() -> (tempfile::TempDir, ConfigScope, String) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bullseye.yaml"), FIXTURE_YAML).unwrap();
        let cwd = tmp.path().to_string_lossy().to_string();

        let cfg_tmp = tempfile::tempdir().unwrap();
        config::set_config_dir_override(Some(cfg_tmp.path().to_path_buf()));
        config::save(&Config {
            storage: Storage {
                mode: Mode::InRepo,
                root: None,
            },
        })
        .expect("save in-repo config");

        (
            tmp,
            ConfigScope {
                _config_tmp: cfg_tmp,
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
            observable: None,
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
}
