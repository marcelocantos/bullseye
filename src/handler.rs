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

use crate::graph;
use crate::import;
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
            TargetTools::ValidateTool(t) => handle_validate(t),
            TargetTools::GraphTool(t) => handle_graph(t),
            TargetTools::RenderTool(t) => handle_render(t),
            TargetTools::InitTool(t) => handle_init(t),
            TargetTools::ImportTool(t) => handle_import(t),
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
    let path = store::discover(dir).ok_or_else(|| tool_err("no targets.yaml found"))?;
    let file = store::load(&path).map_err(tool_err)?;
    Ok((path, file))
}

/// Load the targets file, or create an empty one if none exists.
fn load_or_create_file(
    cwd: &str,
) -> Result<(std::path::PathBuf, crate::schema::TargetsFile), CallToolError> {
    let dir = Path::new(cwd);
    if let Some(path) = store::discover(dir) {
        let file = store::load(&path).map_err(tool_err)?;
        Ok((path, file))
    } else {
        let path = store::create_default(dir).map_err(tool_err)?;
        let file = store::load(&path).map_err(tool_err)?;
        Ok((path, file))
    }
}

/// Save the YAML and re-render the markdown view.
fn save_and_render(path: &Path, file: &crate::schema::TargetsFile) -> Result<(), CallToolError> {
    store::save(path, file).map_err(tool_err)?;
    render::render_to_file(path, file).map_err(tool_err)?;
    Ok(())
}

fn handle_list(t: crate::tools::ListTool) -> ToolResult {
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

fn handle_add(t: crate::tools::AddTool) -> ToolResult {
    let (path, mut file) = load_or_create_file(&t.cwd)?;

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
        "Created 🎯{next_id} \"{name}\"\nValue: {v}, Cost: {c}\nFile: {path}",
        name = t.name,
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

    text_result(format!("Updated 🎯{}:\n{}", t.id, changes.join("\n")))
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

    let result =
        ops::rework(&mut file, &t.id, &t.diagnosis).map_err(|e| tool_err(e.to_string()))?;

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

fn handle_init(t: crate::tools::InitTool) -> ToolResult {
    let dir = Path::new(&t.cwd);

    // Refuse if a targets file already exists.
    if store::discover(dir).is_some() {
        return err("targets.yaml already exists — use bullseye_add to add targets");
    }

    let project = t.project_name.unwrap_or_else(|| {
        dir.file_name()
            .map_or("my-project".into(), |n| n.to_string_lossy().into_owned())
    });

    let path = store::create_starter(dir, &project).map_err(tool_err)?;
    let file = store::load(&path).map_err(tool_err)?;
    render::render_to_file(&path, &file).map_err(tool_err)?;

    text_result(format!(
        "Created starter targets file at {}\n\
         Contains 1 sample target (🎯T1) — edit or replace it with your own.\n\
         Markdown view rendered alongside.",
        path.display(),
    ))
}

fn handle_render(t: crate::tools::RenderTool) -> ToolResult {
    let (path, file) = load_file(&t.cwd)?;
    render::render_to_file(&path, &file).map_err(tool_err)?;
    let md_path = render::markdown_path(&path);
    text_result(format!("Rendered {}", md_path.display()))
}

fn handle_import(t: crate::tools::ImportTool) -> ToolResult {
    let dir = Path::new(&t.cwd);

    // Refuse to overwrite unless force is set.
    if !t.force && store::discover(dir).is_some() {
        return err(
            "targets.yaml already exists — use force: true to overwrite, \
             or use bullseye_add/update to modify existing targets",
        );
    }

    // Find the markdown file.
    let md_path = if let Some(ref explicit) = t.path {
        std::path::PathBuf::from(explicit)
    } else {
        discover_markdown(dir).ok_or_else(|| tool_err("no targets.md found"))?
    };

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

    // Write the YAML file.
    let docs = dir.join("docs");
    std::fs::create_dir_all(&docs)
        .map_err(|e| tool_err(format!("failed to create {}: {e}", docs.display())))?;
    let yaml_path = docs.join("targets.yaml");
    store::save(&yaml_path, &file).map_err(tool_err)?;

    // Re-render the markdown from the YAML (canonical formatting).
    render::render_to_file(&yaml_path, &file).map_err(tool_err)?;

    text_result(format!(
        "Imported {} targets from {}\n\
         Written to {}\n\
         Markdown re-rendered alongside.\n\
         Validation: OK",
        file.targets.len(),
        md_path.display(),
        yaml_path.display(),
    ))
}

/// Discover a targets.md by walking up from start_dir.
fn discover_markdown(start_dir: &Path) -> Option<std::path::PathBuf> {
    let mut dir = start_dir.to_path_buf();
    for _ in 0..64 {
        for candidate in &["docs/targets.md", "targets.md"] {
            let path = dir.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
    None
}
