// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, HashSet, VecDeque};

use crate::schema::{Kind, Status, TargetsFile};

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

/// A target in the frontier: unblocked and ready for work.
#[derive(Debug, Clone)]
pub struct FrontierTarget {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub status: Status,
    pub verifies: Vec<String>,
    pub tags: Vec<String>,
}

/// Compute the frontier: active leaf targets with all dependencies satisfied.
///
/// A target is in the frontier if:
/// - It is active (not achieved).
/// - It has no unachieved dependencies (depends_on all achieved or absent).
/// - It has no active children (it's a leaf in the active graph).
pub fn frontier(file: &TargetsFile) -> Vec<FrontierTarget> {
    let active = file.active();

    // Build set of targets that have active children.
    let mut has_active_children: HashSet<&str> = HashSet::new();
    for (_, t) in &active {
        if let Some(ref parent) = t.parent {
            if active.contains_key(parent.as_str()) {
                has_active_children.insert(parent.as_str());
            }
        }
    }

    active
        .iter()
        .filter(|(id, t)| {
            // Must be a leaf (no active children).
            if has_active_children.contains(*id) {
                return false;
            }
            // All dependencies must be achieved.
            t.depends_on.iter().all(|dep| {
                file.targets
                    .get(dep.as_str())
                    .is_some_and(|d| d.status == Status::Achieved)
            })
        })
        .map(|(id, t)| FrontierTarget {
            id: id.to_string(),
            name: t.name.clone(),
            kind: t.kind,
            status: t.status,
            verifies: t.verifies.clone(),
            tags: t.tags.clone(),
        })
        .collect()
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

/// A tunnel warning: a work target that is too far from verification.
#[derive(Debug, Clone)]
pub struct TunnelWarning {
    /// The work target at the start of the unverified chain.
    pub target_id: String,
    pub target_name: String,
    /// Minimum hops to the nearest verify target (None = no verify reachable).
    pub depth: Option<usize>,
    /// The nearest verify target (if any).
    pub nearest_verify: Option<String>,
}

/// Detect tunnels: active work targets that are far from verification.
///
/// A tunnel exists when a work target has no verify target reachable
/// within `max_depth` hops along the forward dependency graph (targets
/// that depend on it, or verify targets whose `verifies` list includes it).
/// Default max_depth is 2.
pub fn tunnels(file: &TargetsFile, max_depth: usize) -> Vec<TunnelWarning> {
    let active = file.active();

    // For each active work target, find the shortest distance to a verify
    // target that covers it (directly or transitively).
    //
    // "Covers" means: a verify target V where this target is in V.verifies,
    // or a verify target V that verifies some target downstream of this one.
    //
    // We build a forward graph: target → targets that depend on it.
    // Then BFS from each work target looking for a verify target.

    // Build forward adjacency: id → set of active targets that list id in depends_on.
    let mut forward: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, t) in &active {
        for dep in &t.depends_on {
            if active.contains_key(dep.as_str()) {
                forward.entry(dep.as_str()).or_default().push(id);
            }
        }
    }

    // Also add edges from work targets to verify targets that list them in verifies.
    // This is the "verification covers me" relationship.
    for (id, t) in &active {
        if t.kind == Kind::Verify {
            for v in &t.verifies {
                if active.contains_key(v.as_str()) {
                    forward.entry(v.as_str()).or_default().push(id);
                }
            }
        }
    }

    let mut warnings = Vec::new();

    for (id, t) in &active {
        if t.kind != Kind::Work {
            continue;
        }

        // BFS from this work target through forward edges.
        let mut visited: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<(&str, usize)> = VecDeque::new();
        queue.push_back((id, 0));
        visited.insert(id);

        let mut nearest: Option<(usize, String)> = None;

        while let Some((current, depth)) = queue.pop_front() {
            // Check if current is a verify target that covers us.
            if let Some(ct) = active.get(current) {
                if ct.kind == Kind::Verify && current != *id {
                    nearest = Some((depth, current.to_string()));
                    break;
                }
            }

            // Don't expand beyond max_depth + 1 (we need to check nodes at depth max_depth+1).
            if depth > max_depth {
                continue;
            }

            if let Some(neighbors) = forward.get(current) {
                for &next in neighbors {
                    if visited.insert(next) {
                        queue.push_back((next, depth + 1));
                    }
                }
            }
        }

        match nearest {
            Some((depth, verify_id)) if depth > max_depth => {
                warnings.push(TunnelWarning {
                    target_id: id.to_string(),
                    target_name: t.name.clone(),
                    depth: Some(depth),
                    nearest_verify: Some(verify_id),
                });
            }
            None => {
                warnings.push(TunnelWarning {
                    target_id: id.to_string(),
                    target_name: t.name.clone(),
                    depth: None,
                    nearest_verify: None,
                });
            }
            _ => {} // Within max_depth, no warning.
        }
    }

    warnings
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

    // Verifies edges.
    for (id, t) in &active {
        for v in &t.verifies {
            if active.contains_key(v.as_str()) {
                lines.push(format!(
                    "    {} -.->|verifies| {}",
                    mermaid_node(id),
                    mermaid_node(v)
                ));
            }
        }
    }

    // Rework edges (backward, shown in red).
    for (id, t) in &active {
        if let Some(ref rework) = t.rework {
            if active.contains_key(rework.as_str()) {
                lines.push(format!(
                    "    {} -.->|rework| {}",
                    mermaid_node(id),
                    mermaid_node(rework)
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

        // Verifies references must exist.
        for v in &t.verifies {
            if !file.targets.contains_key(v) {
                errors.push(format!("{id}: verifies target {v} does not exist"));
            }
        }

        // Verify targets must have verifies non-empty; work targets must not.
        if t.kind == Kind::Verify && t.verifies.is_empty() {
            errors.push(format!("{id}: verify target must have non-empty verifies"));
        }
        if t.kind == Kind::Work && !t.verifies.is_empty() {
            errors.push(format!("{id}: work target must not have verifies"));
        }

        // Rework validation.
        if let Some(ref rework) = t.rework {
            if t.kind != Kind::Verify {
                errors.push(format!("{id}: only verify targets can have rework"));
            }
            if !file.targets.contains_key(rework) {
                errors.push(format!("{id}: rework target {rework} does not exist"));
            } else if !t.verifies.contains(rework) {
                errors.push(format!(
                    "{id}: rework target {rework} must be in verifies list"
                ));
            }
        }

        // Retry budget only on targets that could be rework destinations.
        // (Advisory — we don't enforce this strictly, just warn if retries > budget.)
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
