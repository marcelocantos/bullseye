// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Local;
use rust_mcp_sdk::mcp_server::ServerHandler;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use rust_mcp_sdk::schema::{
    CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams, RpcError,
};
use rust_mcp_sdk::McpServer;

use crate::graph;
use crate::ops;
use crate::render;
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
            TargetTools::AddTool(t) => handle_add(t),
            TargetTools::UpdateTool(t) => handle_update(t),
            TargetTools::RetireTool(t) => handle_retire(t),
            TargetTools::FrontierTool(t) => handle_frontier(t),
            TargetTools::ReworkTool(t) => handle_rework(t),
            TargetTools::TunnelsTool(t) => handle_tunnels(t),
            TargetTools::RankTool(t) => handle_rank(t),
            TargetTools::ValidateTool(t) => handle_validate(t),
            TargetTools::GraphTool(t) => handle_graph(t),
            TargetTools::RenderTool(t) => handle_render(t),
        }
    }
}

type ToolResult = Result<CallToolResult, CallToolError>;

fn text_result(text: String) -> ToolResult {
    Ok(CallToolResult::text_content(vec![text.into()]))
}

fn tool_err(msg: impl Into<String>) -> CallToolError {
    CallToolError::unknown_tool(msg.into())
}

fn err(msg: impl Into<String>) -> ToolResult {
    Err(tool_err(msg))
}

fn load_file(cwd: &str) -> Result<(std::path::PathBuf, crate::schema::TargetsFile), CallToolError> {
    let dir = Path::new(cwd);
    let path = store::discover(dir)
        .ok_or_else(|| tool_err("no targets.yaml found"))?;
    let file = store::load(&path).map_err(|e| tool_err(e))?;
    Ok((path, file))
}

/// Save the YAML and re-render the markdown view.
fn save_and_render(
    path: &Path,
    file: &crate::schema::TargetsFile,
) -> Result<(), CallToolError> {
    store::save(path, file).map_err(|e| tool_err(e))?;
    render::render_to_file(path, file).map_err(|e| tool_err(e))?;
    Ok(())
}

fn handle_list(t: crate::tools::ListTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let targets: Vec<(&str, &Target)> = match t.filter.as_str() {
        "active" => file.active().into_iter().collect(),
        "achieved" => file.achieved().into_iter().collect(),
        "all" => file.targets.iter().map(|(k, v)| (k.as_str(), v)).collect(),
        other => return err(format!("unknown filter: {other} (use active, achieved, all)")),
    };

    let mut sorted: Vec<_> = targets;
    sorted.sort_by(|a, b| b.1.weight().partial_cmp(&a.1.weight()).unwrap());

    let mut out = format!("# Targets ({})\nFile: {}\n\n", t.filter, path.display());
    for (id, target) in &sorted {
        let w = target.weight();
        out.push_str(&format!(
            "🎯{id} {name}\n  status: {status:?}  weight: {w:.0} (value {v} / cost {c})\n",
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

    let yaml = serde_yaml::to_string(target).map_err(|e| tool_err(e.to_string()))?;
    text_result(format!("🎯{} {}\n\n{yaml}", t.id, target.name))
}

fn handle_add(t: crate::tools::AddTool) -> ToolResult {
    let (path, mut file) = load_file(&t.cwd)?;

    // Determine next ID.
    let next_id = match &t.parent {
        Some(parent_id) => {
            let prefix = format!("{parent_id}.");
            let max_child = file
                .targets
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix))
                .filter_map(|suffix| suffix.split('.').next()?.parse::<u32>().ok())
                .max()
                .unwrap_or(0);
            format!("{parent_id}.{}", max_child + 1)
        }
        None => {
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
    };

    let kind = match t.kind.as_deref() {
        Some("verify") => crate::schema::Kind::Verify,
        Some("work") | None => crate::schema::Kind::Work,
        Some(other) => return err(format!("unknown kind: {other} (use work or verify)")),
    };

    let target = Target {
        name: t.name.clone(),
        kind,
        status: Status::Identified,
        value: t.value,
        cost: t.cost,
        actual_cost: None,
        acceptance: t.acceptance,
        context: t.context,
        parent: t.parent,
        gates: Vec::new(),
        depends_on: Vec::new(),
        verifies: t.verifies,
        rework: None,
        retry_budget: None,
        retries: 0,
        tags: t.tags,
        origin: t.origin,
        discovered: Local::now().date_naive(),
        achieved: None,
    };

    file.targets.insert(next_id.clone(), target);
    save_and_render(&path, &file)?;

    text_result(format!(
        "Created 🎯{next_id} \"{name}\"\nWeight: {w:.0} (value {v} / cost {c})\nFile: {path}",
        name = t.name,
        w = (t.value / t.cost).max(1.0),
        v = t.value,
        c = t.cost,
        path = path.display(),
    ))
}

fn handle_update(t: crate::tools::UpdateTool) -> ToolResult {
    let (path, mut file) = load_file(&t.cwd)?;

    let target = file
        .targets
        .get_mut(&t.id)
        .ok_or_else(|| tool_err(format!("target {} not found", t.id)))?;

    let mut changes = Vec::new();

    if let Some(ref status_str) = t.status {
        let status = match status_str.as_str() {
            "identified" => Status::Identified,
            "converging" => Status::Converging,
            "achieved" => Status::Achieved,
            other => return err(format!("unknown status: {other}")),
        };
        if target.status != status {
            changes.push(format!("status: {:?} → {status:?}", target.status));
            target.status = status;
            if status == Status::Achieved && target.achieved.is_none() {
                target.achieved = Some(Local::now().date_naive());
            }
        }
    }
    if let Some(value) = t.value {
        changes.push(format!("value: {} → {value}", target.value));
        target.value = value;
    }
    if let Some(cost) = t.cost {
        changes.push(format!("cost: {} → {cost}", target.cost));
        target.cost = cost;
    }
    if let Some(ref name) = t.name {
        changes.push(format!("name: \"{}\" → \"{name}\"", target.name));
        target.name = name.clone();
    }
    if let Some(ref acceptance) = t.acceptance {
        changes.push("acceptance: updated".to_string());
        target.acceptance = acceptance.clone();
    }
    if let Some(ref context) = t.context {
        changes.push("context: updated".to_string());
        target.context = context.clone();
    }
    if let Some(ref tags) = t.tags {
        changes.push(format!("tags: {:?} → {tags:?}", target.tags));
        target.tags = tags.clone();
    }

    if changes.is_empty() {
        return text_result(format!("🎯{}: no changes", t.id));
    }

    save_and_render(&path, &file)?;

    text_result(format!(
        "Updated 🎯{}:\n{}",
        t.id,
        changes.join("\n")
    ))
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

    // Check for unachieved children.
    let unachieved_children: Vec<String> = file
        .targets
        .iter()
        .filter(|(_, child)| {
            child.parent.as_deref() == Some(&t.id) && child.status != Status::Achieved
        })
        .map(|(id, _)| id.clone())
        .collect();

    let target = file.targets.get_mut(&t.id).unwrap();
    target.status = Status::Achieved;
    target.achieved = Some(Local::now().date_naive());
    if let Some(actual) = t.actual_cost {
        target.actual_cost = Some(actual);
    }

    let name = target.name.clone();
    let cost = target.cost;

    save_and_render(&path, &file)?;

    let mut out = format!("Retired 🎯{} \"{name}\"", t.id);
    if let Some(actual) = t.actual_cost {
        out.push_str(&format!("\nCost: estimated {cost}, actual {actual}"));
    }
    if !unachieved_children.is_empty() {
        out.push_str(&format!(
            "\n⚠ Unachieved children: {}",
            unachieved_children.join(", ")
        ));
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

    let result = ops::rework(&mut file, &t.id, &t.diagnosis)
        .map_err(|e| tool_err(e.to_string()))?;

    save_and_render(&path, &file)?;

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

fn handle_rank(t: crate::tools::RankTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let errors = graph::validate(&file);
    if !errors.is_empty() {
        return text_result(format!("# Validation errors\n\n{}", errors.join("\n")));
    }

    let ranked = graph::rank(&file);
    let (unblocked, blocked): (Vec<_>, Vec<_>) =
        ranked.iter().partition(|r| r.blocked_by.is_empty());

    let mut out = format!("# Ranking\nFile: {}\n", path.display());

    out.push_str("\n## Unblocked\n");
    if unblocked.is_empty() {
        out.push_str("(none)\n");
    }
    for r in &unblocked {
        format_ranked(&mut out, r, 0);
    }

    out.push_str("\n## Blocked\n");
    if blocked.is_empty() {
        out.push_str("(none)\n");
    }
    for r in &blocked {
        format_ranked(&mut out, r, 0);
    }

    text_result(out)
}

fn format_ranked(out: &mut String, r: &graph::RankedTarget, indent: usize) {
    let prefix = "  ".repeat(indent);
    out.push_str(&format!(
        "\n{prefix}🎯{id} {name}\n{prefix}  status: {status:?}  weight: {w:.0} (value {v} / cost {c})\n",
        id = r.id,
        name = r.name,
        status = r.status,
        w = r.weight,
        v = r.value,
        c = r.cost,
    ));
    if !r.tags.is_empty() {
        out.push_str(&format!("{prefix}  tags: {}\n", r.tags.join(", ")));
    }
    if !r.gates.is_empty() {
        let gs: Vec<String> = r
            .gates
            .iter()
            .map(|(id, c)| {
                if *c < 1.0 {
                    format!("{id} ({}%)", (*c * 100.0) as u32)
                } else {
                    id.clone()
                }
            })
            .collect();
        out.push_str(&format!("{prefix}  gates: {}\n", gs.join(", ")));
    }
    if !r.blocked_by.is_empty() {
        out.push_str(&format!(
            "{prefix}  blocked-by: {}\n",
            r.blocked_by.join(", ")
        ));
    }
}

fn handle_validate(t: crate::tools::ValidateTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;

    let errors = graph::validate(&file);
    if errors.is_empty() {
        text_result(format!("Valid: {} ({} targets)", path.display(), file.targets.len()))
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

fn handle_render(t: crate::tools::RenderTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;
    render::render_to_file(&path, &file).map_err(|e| tool_err(e))?;
    let md_path = render::markdown_path(&path);
    text_result(format!("Rendered {}", md_path.display()))
}
