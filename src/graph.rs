// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, HashSet};

use crate::schema::{Status, TargetsFile};

/// Ranked target with computed effective weight and blocking info.
#[derive(Debug, Clone)]
pub struct RankedTarget {
    pub id: String,
    pub name: String,
    pub status: Status,
    pub value: f64,
    pub cost: f64,
    pub weight: f64,
    pub blocked_by: Vec<String>,
    pub children: Vec<String>,
    pub gates: Vec<(String, f64)>,
    pub tags: Vec<String>,
}

/// Compute rankings for all active targets.
pub fn rank(file: &TargetsFile) -> Vec<RankedTarget> {
    let active: BTreeMap<&str, _> = file.active();

    // Build parent -> children map.
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, t) in &active {
        if let Some(ref parent) = t.parent {
            if active.contains_key(parent.as_str()) {
                children.entry(parent.as_str()).or_default().push(id);
            }
        }
    }

    // Compute blocked-by: depends_on targets that aren't achieved.
    let mut blocked_by: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (id, t) in &active {
        let blockers: Vec<String> = t
            .depends_on
            .iter()
            .filter(|dep| {
                file.targets
                    .get(dep.as_str())
                    .is_some_and(|d| d.status != Status::Achieved)
            })
            .cloned()
            .collect();
        blocked_by.insert(id, blockers);
    }

    // Build ranked list.
    let mut ranked: Vec<RankedTarget> = active
        .iter()
        .map(|(id, t)| {
            let child_ids: Vec<String> = children
                .get(id)
                .map(|cs| cs.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default();

            RankedTarget {
                id: id.to_string(),
                name: t.name.clone(),
                status: t.status,
                value: t.value,
                cost: t.cost,
                weight: t.weight(),
                blocked_by: blocked_by.get(id).cloned().unwrap_or_default(),
                children: child_ids,
                gates: t.gates.iter().map(|g| (g.target.clone(), g.criticality)).collect(),
                tags: t.tags.clone(),
            }
        })
        .collect();

    // Sort by weight descending.
    ranked.sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap_or(std::cmp::Ordering::Equal));

    ranked
}

/// Generate a Mermaid dependency graph of active targets.
pub fn mermaid(file: &TargetsFile) -> String {
    let active = file.active();
    let mut lines = vec!["graph TD".to_string()];

    // Nodes.
    for (id, t) in &active {
        let label = truncate(&t.name, 30);
        let node = mermaid_node(id);
        lines.push(format!("    {node}[\"{label}\"]"));
    }

    // Parent -> child edges.
    for (id, t) in &active {
        if let Some(ref parent) = t.parent {
            if active.contains_key(parent.as_str()) {
                lines.push(format!(
                    "    {} --> {}",
                    mermaid_node(parent),
                    mermaid_node(id)
                ));
            }
        }
    }

    // Gates edges.
    for (id, t) in &active {
        for gate in &t.gates {
            if active.contains_key(gate.target.as_str()) {
                let label = if gate.criticality < 1.0 {
                    format!("gates {}%", (gate.criticality * 100.0) as u32)
                } else {
                    "gates".to_string()
                };
                lines.push(format!(
                    "    {} -.->|{label}| {}",
                    mermaid_node(id),
                    mermaid_node(&gate.target)
                ));
            }
        }
    }

    // Depends-on edges.
    for (id, t) in &active {
        for dep in &t.depends_on {
            if active.contains_key(dep.as_str()) {
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

/// Validate the targets file. Returns a list of errors (empty = valid).
pub fn validate(file: &TargetsFile) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen_ids: HashSet<&str> = HashSet::new();

    for (id, t) in &file.targets {
        // Check ID format.
        if !id.starts_with('T') || id[1..].split('.').any(|p| p.parse::<u32>().is_err()) {
            errors.push(format!("{id}: invalid target ID format (expected T<N> or T<N>.<M>)"));
        }

        // Duplicate check.
        if !seen_ids.insert(id.as_str()) {
            errors.push(format!("{id}: duplicate target ID"));
        }

        // Value/cost must be positive.
        if t.value <= 0.0 {
            errors.push(format!("{id}: value must be positive, got {}", t.value));
        }
        if t.cost <= 0.0 {
            errors.push(format!("{id}: cost must be positive, got {}", t.cost));
        }

        // Acceptance must be non-empty.
        if t.acceptance.is_empty() {
            errors.push(format!("{id}: acceptance criteria must not be empty"));
        }

        // Parent reference must exist.
        if let Some(ref parent) = t.parent {
            if !file.targets.contains_key(parent) {
                errors.push(format!("{id}: parent {parent} does not exist"));
            }
        }

        // Gates references must exist.
        for gate in &t.gates {
            if !file.targets.contains_key(&gate.target) {
                errors.push(format!("{id}: gates target {} does not exist", gate.target));
            }
            if gate.criticality <= 0.0 || gate.criticality > 1.0 {
                errors.push(format!(
                    "{id}: criticality for {} must be in (0, 1], got {}",
                    gate.target, gate.criticality
                ));
            }
        }

        // Depends-on references must exist.
        for dep in &t.depends_on {
            if !file.targets.contains_key(dep) {
                errors.push(format!("{id}: depends_on target {dep} does not exist"));
            }
        }
    }

    // Cycle detection on parent hierarchy.
    for id in file.targets.keys() {
        let mut visited = HashSet::new();
        let mut current = Some(id.as_str());
        while let Some(c) = current {
            if !visited.insert(c) {
                errors.push(format!("{id}: cycle in parent hierarchy"));
                break;
            }
            current = file
                .targets
                .get(c)
                .and_then(|t| t.parent.as_deref());
        }
    }

    errors
}

fn mermaid_node(id: &str) -> String {
    id.replace('.', "_")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
