// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Mermaid dependency-graph export (🎯T57): scope/seed/expand selection
//! and rendering to `graph TD` source.

use std::collections::{BTreeMap, HashSet};

use crate::schema::{Status, TargetsFile};

use super::frontier::frontier;

/// Which target statuses are candidates for the Mermaid diagram (🎯T57).
///
/// Default remains **active-only** — the historical `view=graph` behaviour —
/// so existing callers keep the same whole-active-graph diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MermaidScope {
    /// Identified + converging only (terminal statuses excluded). **Default.**
    #[default]
    Active,
    /// Every target regardless of status.
    All,
    /// Achieved targets only.
    Achieved,
    /// Set-aside targets only.
    SetAside,
}

impl MermaidScope {
    /// Parse CLI/MCP scope token: `active` | `all` | `achieved` | `set_aside`.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "all" => Ok(Self::All),
            "achieved" => Ok(Self::Achieved),
            "set_aside" | "set-aside" | "aside" => Ok(Self::SetAside),
            other => Err(format!(
                "unknown graph scope `{other}` (use active, all, achieved, set_aside)"
            )),
        }
    }

    fn includes(self, status: Status) -> bool {
        match self {
            Self::Active => !status.is_terminal(),
            Self::All => true,
            Self::Achieved => status == Status::Achieved,
            Self::SetAside => status == Status::SetAside,
        }
    }
}

/// Intelligent expansion steps from seed IDs (🎯T57).
///
/// Edge policy (documented for agents):
/// - **ancestors** — walk `depends_on` outward (deps the seed needs).
/// - **descendants** — reverse-blocks: targets that list the seed in
///   `depends_on` (what the seed unblocks).
/// - **children** — hierarchical children by ID convention (`T1.1` of
///   `T1`). Display-only: the blocking edge is `depends_on` (🎯T39.1).
/// - **parents** — hierarchical parent by ID convention (`T1` of `T1.1`).
///   Display-only; same caveat as children.
/// - **frontier** — among 1-hop dependency neighbors of the selection,
///   also include nodes currently on the frontier (unblocked leaves).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MermaidExpand {
    pub ancestors: bool,
    pub descendants: bool,
    pub children: bool,
    pub parents: bool,
    pub frontier: bool,
}

impl MermaidExpand {
    /// Parse a comma/space-separated expand list, e.g.
    /// `ancestors,descendants,children,parents,frontier`.
    pub fn parse_list(s: &str) -> Result<Self, String> {
        let mut e = Self::default();
        for part in s.split(|c: char| c == ',' || c.is_whitespace()) {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            match p.to_ascii_lowercase().as_str() {
                "ancestors" | "ancestor" | "deps" | "depends_on" => e.ancestors = true,
                "descendants" | "descendant" | "reverse" | "blocks" | "dependents" => {
                    e.descendants = true
                }
                "children" | "child" | "kids" => e.children = true,
                "parents" | "parent" => e.parents = true,
                "frontier" | "frontier_around" | "frontier-around" => e.frontier = true,
                other => {
                    return Err(format!(
                        "unknown expand step `{other}` (use ancestors, descendants, \
                         children, parents, frontier)"
                    ));
                }
            }
        }
        Ok(e)
    }

    fn any(self) -> bool {
        self.ancestors || self.descendants || self.children || self.parents || self.frontier
    }
}

/// Options for Mermaid export (🎯T57).
///
/// Selection modes:
/// 1. **Whole graph (default)** — no `nodes` / `seeds`: all targets in
///    [`MermaidOpts::scope`] (default active). Same as pre-T57 `mermaid`.
/// 2. **Explicit node list** — `nodes` only: those IDs (filtered by scope);
///    edges drawn only when both endpoints are selected.
/// 3. **Seed expansion** — `seeds` plus [`MermaidExpand`] flags walk the
///    documented edge policy. Disjoint components are fine.
///
/// When both `nodes` and `seeds` are set, the result is the **union**.
#[derive(Debug, Clone, Default)]
pub struct MermaidOpts {
    pub scope: MermaidScope,
    /// Explicit node IDs (naive mode).
    pub nodes: Vec<String>,
    /// Seed IDs for intelligent expansion.
    pub seeds: Vec<String>,
    pub expand: MermaidExpand,
}

/// Generate a Mermaid dependency graph of active targets (default scope).
///
/// Equivalent to [`mermaid_with_opts`] with [`MermaidOpts::default`] —
/// whole active graph, `depends_on` edges only. Preserves pre-T57 behaviour.
pub fn mermaid(file: &TargetsFile) -> String {
    mermaid_with_opts(file, &MermaidOpts::default())
}

/// Generate Mermaid source for the full graph or a selected subgraph (🎯T57).
///
/// Returns raw Mermaid (`graph TD` …) without fences; callers wrap in
/// ` ```mermaid ` for chat clients (e.g. jevons 🎯T59).
///
/// Never errors solely because selected nodes are disconnected or have no
/// edges — an empty selection yields `graph TD` with a single comment node.
pub fn mermaid_with_opts(file: &TargetsFile, opts: &MermaidOpts) -> String {
    let selected = select_mermaid_nodes(file, opts);
    render_mermaid(file, &selected)
}

/// Resolve which target IDs appear in the diagram under `opts`.
pub fn select_mermaid_nodes(file: &TargetsFile, opts: &MermaidOpts) -> BTreeMap<String, ()> {
    let in_scope: HashSet<&str> = file
        .targets
        .iter()
        .filter(|(_, t)| opts.scope.includes(t.status))
        .map(|(id, _)| id.as_str())
        .collect();

    let has_filter = !opts.nodes.is_empty() || !opts.seeds.is_empty();
    if !has_filter {
        // Whole-graph default for this scope.
        return in_scope
            .into_iter()
            .map(|id| (id.to_string(), ()))
            .collect();
    }

    let mut selected: HashSet<String> = HashSet::new();

    // Mode 1: explicit nodes (must exist and pass scope).
    for id in &opts.nodes {
        if in_scope.contains(id.as_str()) {
            selected.insert(id.clone());
        }
    }

    // Mode 2: seeds + optional expansion.
    let mut seeds: HashSet<String> = HashSet::new();
    for id in &opts.seeds {
        if in_scope.contains(id.as_str()) {
            seeds.insert(id.clone());
            selected.insert(id.clone());
        }
    }

    if !seeds.is_empty() && opts.expand.any() {
        expand_from_seeds(file, &in_scope, &seeds, opts.expand, &mut selected);
    }

    selected.into_iter().map(|id| (id, ())).collect()
}

fn expand_from_seeds(
    file: &TargetsFile,
    in_scope: &HashSet<&str>,
    seeds: &HashSet<String>,
    expand: MermaidExpand,
    selected: &mut HashSet<String>,
) {
    // Build reverse depends_on index once (descendants / frontier).
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, t) in &file.targets {
        for dep in &t.depends_on {
            dependents
                .entry(dep.as_str())
                .or_default()
                .push(id.as_str());
        }
    }

    // BFS over requested edge kinds. Work queue starts at seeds.
    let mut queue: Vec<String> = seeds.iter().cloned().collect();
    let mut seen: HashSet<String> = seeds.clone();

    while let Some(cur) = queue.pop() {
        let Some(t) = file.targets.get(&cur) else {
            continue;
        };

        if expand.ancestors {
            for dep in &t.depends_on {
                if in_scope.contains(dep.as_str()) && seen.insert(dep.clone()) {
                    selected.insert(dep.clone());
                    queue.push(dep.clone());
                }
            }
        }

        if expand.descendants
            && let Some(deps) = dependents.get(cur.as_str())
        {
            for &d in deps {
                if in_scope.contains(d) && seen.insert(d.to_string()) {
                    selected.insert(d.to_string());
                    queue.push(d.to_string());
                }
            }
        }

        if expand.children {
            let prefix = format!("{cur}.");
            for id in file.targets.keys() {
                if id.starts_with(&prefix)
                    && in_scope.contains(id.as_str())
                    && seen.insert(id.clone())
                {
                    selected.insert(id.clone());
                    queue.push(id.clone());
                }
            }
        }

        if expand.parents
            && let Some(dot) = cur.rfind('.')
        {
            let parent = &cur[..dot];
            if in_scope.contains(parent) && seen.insert(parent.to_string()) {
                selected.insert(parent.to_string());
                queue.push(parent.to_string());
            }
        }
    }

    if expand.frontier {
        // 1-hop dependency neighbors of current selection that are on the
        // frontier (active unblocked leaves). Seeds stay even if not frontier.
        let front: HashSet<String> = frontier(file).into_iter().map(|f| f.id).collect();
        let base: Vec<String> = selected.iter().cloned().collect();
        for id in base {
            if let Some(t) = file.targets.get(&id) {
                for dep in &t.depends_on {
                    if front.contains(dep) && in_scope.contains(dep.as_str()) {
                        selected.insert(dep.clone());
                    }
                }
            }
            if let Some(deps) = dependents.get(id.as_str()) {
                for &d in deps {
                    if front.contains(d) && in_scope.contains(d) {
                        selected.insert(d.to_string());
                    }
                }
            }
        }
    }
}

fn render_mermaid(file: &TargetsFile, selected: &BTreeMap<String, ()>) -> String {
    let mut lines = vec!["graph TD".to_string()];

    if selected.is_empty() {
        // Valid Mermaid, not an error — empty selection / empty scope.
        lines.push("    empty[\"(no targets in selection)\"]".to_string());
        return lines.join("\n");
    }

    for id in selected.keys() {
        let Some(t) = file.targets.get(id) else {
            continue;
        };
        let mut label = truncate(&t.name, 30);
        if t.status.is_terminal() {
            let tag = match t.status {
                Status::Achieved => "achieved",
                Status::SetAside => "set_aside",
                _ => "",
            };
            if !tag.is_empty() {
                label = format!("{label} ({tag})");
            }
        }
        let label = mermaid_escape(&label);
        lines.push(format!("    {}[\"{label}\"]", mermaid_node(id)));
    }

    // Depends-on edges only when both endpoints are selected.
    for id in selected.keys() {
        let Some(t) = file.targets.get(id) else {
            continue;
        };
        for dep in &t.depends_on {
            if selected.contains_key(dep) {
                lines.push(format!(
                    "    {} -.->|needs| {}",
                    mermaid_node(id),
                    mermaid_node(dep)
                ));
            }
        }
    }

    lines.join("\n")
}

/// Escape text for a Mermaid node label inside double quotes.
fn mermaid_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "#quot;")
        .replace('\n', " ")
}

fn mermaid_node(id: &str) -> String {
    id.replace('.', "_")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{end}…")
    }
}
