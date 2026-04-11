// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, HashSet, VecDeque};

use crate::schema::{Kind, Status, TargetsFile};

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

/// Compute the frontier: active targets with all dependencies satisfied.
///
/// A target is in the frontier if:
/// - It is active (not achieved).
/// - It has no unachieved dependencies (depends_on all achieved or absent).
pub fn frontier(file: &TargetsFile) -> Vec<FrontierTarget> {
    let active = file.active();

    active
        .iter()
        .filter(|(_, t)| {
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
            if let Some(ct) = active.get(current)
                && ct.kind == Kind::Verify
                && current != *id
            {
                nearest = Some((depth, current.to_string()));
                break;
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
        if let Some(ref rework) = t.rework
            && active.contains_key(rework.as_str())
        {
            lines.push(format!(
                "    {} -.->|rework| {}",
                mermaid_node(id),
                mermaid_node(rework)
            ));
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
            errors.push(format!(
                "{id}: invalid target ID format (expected T<N> or T<N>.<M>)"
            ));
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

    // Cycle detection on depends_on graph (DFS).
    let mut permanent: HashSet<&str> = HashSet::new();
    let mut temporary: HashSet<&str> = HashSet::new();

    fn dfs<'a>(
        id: &'a str,
        targets: &'a std::collections::BTreeMap<String, crate::schema::Target>,
        permanent: &mut HashSet<&'a str>,
        temporary: &mut HashSet<&'a str>,
        errors: &mut Vec<String>,
    ) {
        if permanent.contains(id) {
            return;
        }
        if !temporary.insert(id) {
            errors.push(format!("{id}: cycle in depends_on graph"));
            return;
        }
        if let Some(t) = targets.get(id) {
            for dep in &t.depends_on {
                dfs(dep.as_str(), targets, permanent, temporary, errors);
            }
        }
        temporary.remove(id);
        permanent.insert(id);
    }

    for id in file.targets.keys() {
        dfs(
            id.as_str(),
            &file.targets,
            &mut permanent,
            &mut temporary,
            &mut errors,
        );
    }

    errors
}

/// Produce a concise startup context summary for agent consumption.
pub fn startup_context(file: &TargetsFile, file_path: &str, recent_days: u32) -> String {
    let cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(recent_days as i64);

    let active = file.active();
    let active_count = active.len();

    let errors = validate(file);
    let front = if errors.is_empty() {
        frontier(file)
    } else {
        Vec::new()
    };

    // Recently achieved targets.
    let mut recent_achieved: Vec<(&str, &crate::schema::Target)> = file
        .achieved()
        .into_iter()
        .filter(|(_, t)| t.achieved.is_some_and(|d| d >= cutoff))
        .collect();
    recent_achieved.sort_by(|a, b| b.1.achieved.cmp(&a.1.achieved));

    let tuns = if errors.is_empty() {
        tunnels(file, 2)
    } else {
        Vec::new()
    };

    let mut out = String::new();

    out.push_str(&format!(
        "# Startup context\nFile: {file_path}\nActive: {active_count} target(s), Frontier: {} ready for work\n\n",
        front.len(),
    ));

    if !front.is_empty() {
        out.push_str("## Frontier (unblocked, ready for work)\n\n");
        for ft in &front {
            let kind_label = match ft.kind {
                Kind::Work => "",
                Kind::Verify => " [verify]",
            };
            out.push_str(&format!("🎯{} {}{kind_label}\n", ft.id, ft.name));
            if !ft.tags.is_empty() {
                out.push_str(&format!("  tags: {}\n", ft.tags.join(", ")));
            }
        }
        out.push('\n');
    }

    if !recent_achieved.is_empty() {
        out.push_str(&format!(
            "## Recently achieved (last {recent_days} days)\n\n"
        ));
        for (id, target) in &recent_achieved {
            let date = target.achieved.map_or("?".to_string(), |d| d.to_string());
            out.push_str(&format!("🎯{id} {} (achieved {date})\n", target.name));
        }
        out.push('\n');
    }

    if !errors.is_empty() {
        out.push_str("## Warnings\n\n");
        out.push_str(&format!("Validation errors: {}\n\n", errors.join("; ")));
    }

    if !tuns.is_empty() {
        if errors.is_empty() {
            out.push_str("## Warnings\n\n");
        }
        out.push_str(&format!(
            "Tunnels: {} work target(s) lack nearby verification\n",
            tuns.len()
        ));
    }

    out
}

/// Produce the startup-context response for a project that has no
/// `targets.yaml`. Unlike most tools, startup_context is often called
/// automatically at session start before the caller knows whether the
/// repo uses bullseye, so it needs to degrade gracefully instead of
/// failing the tool call.
pub fn startup_context_no_file(cwd: &str) -> String {
    format!(
        "# Startup context\n\
         File: (no targets.yaml found under {cwd})\n\
         \n\
         This project is not using bullseye yet. Run `bullseye_init` to \
         create a starter `docs/targets.yaml`, or ignore this notice if \
         targets aren't appropriate for this repo.\n",
    )
}

/// Produce the startup-context response for a project whose
/// `targets.yaml` exists but can't be read or parsed. Surfaces the
/// underlying error for the user to diagnose, but intentionally
/// does **not** make the tool call fail — session start should
/// continue regardless of whether the targets file is momentarily
/// broken (e.g. mid-edit, rebase conflict, permission glitch).
///
/// The error text itself comes from [`crate::store::LoadError`].
pub fn startup_context_broken_file(file_path: &str, error: &str) -> String {
    format!(
        "# Startup context\n\
         File: {file_path}\n\
         \n\
         ⚠ targets.yaml could not be loaded: {error}\n\
         \n\
         Session start is continuing without target context. Fix the \
         file (common causes: YAML syntax error, unresolved rebase \
         marker, permission issue) and re-run `bullseye_startup_context` \
         to recover.\n",
    )
}

/// Produce a consolidated status overview for agent consumption.
///
/// The frontier section is ordered by `focus = value × momentum`
/// descending. If `momentum` is `Some`, each frontier target's value
/// is multiplied by its listed multiplier (default 1.0 for targets
/// not in the map) before sorting, so recently-active targets rise
/// and stale ones sink. The multiplier formula itself is the caller's
/// responsibility — bullseye never calls out to mnemo.
///
/// When `frontier_details` is true, each frontier entry is expanded
/// with its full acceptance criteria, context, tags, and related edges.
/// This is what `bullseye_convergence` uses to avoid a `bullseye_get`
/// loop on the frontier; plain `bullseye_summary` leaves it off.
pub fn summary(
    file: &TargetsFile,
    file_path: &str,
    momentum: Option<&BTreeMap<String, f64>>,
    frontier_details: bool,
) -> String {
    let mut out = String::new();

    let errors = validate(file);
    let all_targets = &file.targets;
    let active = file.active();
    let achieved = file.achieved();

    out.push_str(&format!(
        "# Summary\nFile: {file_path}\nTotal: {} target(s) — {} active, {} achieved\n\n",
        all_targets.len(),
        active.len(),
        achieved.len(),
    ));

    // --- 1. Active targets grouped by parent ---
    out.push_str("## Active targets by group\n\n");

    // Derive parent/child from ID convention: T1.2 is child of T1.
    // Use all targets (not just active) so we can detect stale parents
    // whose children are all achieved.
    let mut parent_children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut is_child: HashSet<String> = HashSet::new();

    for id in all_targets.keys() {
        if let Some(dot_pos) = id.rfind('.') {
            let parent_id = &id[..dot_pos];
            parent_children
                .entry(parent_id.to_string())
                .or_default()
                .push(id.to_string());
            is_child.insert(id.to_string());
        }
    }

    // Top-level targets: active targets that are not children of another active target.
    let mut top_level: Vec<&str> = active
        .keys()
        .filter(|id| !is_child.contains(**id))
        .copied()
        .collect();
    top_level.sort();

    for id in &top_level {
        let target = active[*id];
        // Only show active children in the group display.
        let children: Vec<&str> = parent_children
            .get(*id)
            .map(|c| {
                c.iter()
                    .filter(|cid| active.contains_key(cid.as_str()))
                    .map(|s| s.as_str())
                    .collect()
            })
            .unwrap_or_default();

        let all_children = parent_children.get(*id);
        let has_children = all_children.is_some_and(|c| !c.is_empty());

        if !has_children {
            out.push_str(&format!(
                "🎯{id} {} [{:?}]  v={}, c={}\n",
                target.name, target.status, target.value, target.cost,
            ));
        } else {
            // Count achieved children (from all targets, not just active).
            let total_children = all_targets
                .keys()
                .filter(|cid| {
                    cid.starts_with(*id)
                        && cid.len() > id.len()
                        && cid.as_bytes().get(id.len()) == Some(&b'.')
                })
                .count();
            let achieved_children = all_targets
                .iter()
                .filter(|(cid, t)| {
                    cid.starts_with(*id)
                        && cid.len() > id.len()
                        && cid.as_bytes().get(id.len()) == Some(&b'.')
                        && t.status == Status::Achieved
                })
                .count();

            out.push_str(&format!(
                "🎯{id} {} [{:?}]  ({achieved_children}/{total_children} achieved)\n",
                target.name, target.status,
            ));
            for cid in &children {
                let ct = active[cid];
                out.push_str(&format!(
                    "  🎯{cid} {} [{:?}]  v={}, c={}\n",
                    ct.name, ct.status, ct.value, ct.cost,
                ));
            }
        }
    }
    out.push('\n');

    // --- 2. Frontier (ordered by focus = value × momentum) ---
    //
    // Momentum folds directly into frontier ordering instead of a
    // separate ranking section. `focus = value × momentum_lookup(id, 1.0)`,
    // sorted descending. When momentum is absent, the multiplier
    // defaults to 1.0 for every target, so the sort reduces to
    // "by value desc" with stable ID-order tiebreaks.
    let has_momentum = momentum.is_some();
    if errors.is_empty() {
        let front = frontier(file);
        let mut scored: Vec<(FrontierTarget, f64, f64, f64)> = front
            .iter()
            .map(|ft| {
                let value = all_targets.get(&ft.id).map(|t| t.value).unwrap_or(0.0);
                let m = momentum
                    .and_then(|mm| mm.get(&ft.id).copied())
                    .unwrap_or(1.0);
                let focus = value * m;
                (ft.clone(), value, m, focus)
            })
            .collect();
        scored.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));

        out.push_str("## Frontier (unblocked, ready for work)\n\n");
        if scored.is_empty() {
            out.push_str("(no targets ready for work)\n");
        } else {
            for (ft, value, m, focus) in &scored {
                let kind_label = match ft.kind {
                    Kind::Work => "",
                    Kind::Verify => " [verify]",
                };
                // Annotate format depends on whether a momentum map was
                // provided at all. When it was, every entry shows its
                // focus score so the caller can see the full ordering
                // math; when it wasn't, render the plain baseline shape.
                if has_momentum {
                    if (m - 1.0).abs() < 1e-6 {
                        out.push_str(&format!(
                            "🎯{id} {name}{kind}  [{status:?}] — focus {focus:.1} (v={value})\n",
                            id = ft.id,
                            name = ft.name,
                            kind = kind_label,
                            status = ft.status,
                        ));
                    } else {
                        out.push_str(&format!(
                            "🎯{id} {name}{kind}  [{status:?}] — focus {focus:.1} (v={value} × momentum {m:.2})\n",
                            id = ft.id,
                            name = ft.name,
                            kind = kind_label,
                            status = ft.status,
                        ));
                    }
                } else {
                    out.push_str(&format!(
                        "🎯{id} {name}{kind}  [{status:?}]  v={value}\n",
                        id = ft.id,
                        name = ft.name,
                        kind = kind_label,
                        status = ft.status,
                    ));
                }
                if frontier_details {
                    render_frontier_detail(&mut out, &ft.id, all_targets);
                }
            }
        }
        out.push('\n');

        // --- 3. Blocked targets ---
        let front_ids: HashSet<&str> = scored.iter().map(|(f, _, _, _)| f.id.as_str()).collect();
        let blocked: Vec<(&str, &crate::schema::Target)> = active
            .iter()
            .filter(|(id, _)| !front_ids.contains(**id))
            .map(|(&id, t)| (id, *t))
            .collect();

        if !blocked.is_empty() {
            out.push_str("## Blocked targets\n\n");
            for (id, target) in &blocked {
                let unmet: Vec<String> = target
                    .depends_on
                    .iter()
                    .filter(|dep| {
                        all_targets
                            .get(dep.as_str())
                            .is_none_or(|d| d.status != Status::Achieved)
                    })
                    .map(|dep| format!("🎯{dep}"))
                    .collect();
                if unmet.is_empty() {
                    out.push_str(&format!("🎯{id} {}\n", target.name));
                } else {
                    out.push_str(&format!(
                        "🎯{id} {}  blocked by: {}\n",
                        target.name,
                        unmet.join(", "),
                    ));
                }
            }
            out.push('\n');
        }
    } else {
        out.push_str("## Validation errors\n\n");
        for e in &errors {
            out.push_str(&format!("- {e}\n"));
        }
        out.push('\n');
    }

    // --- 4. Stale targets ---
    let mut stale: Vec<String> = Vec::new();

    for (id, target) in &active {
        // Parent still converging/identified but all children achieved.
        if let Some(children) = parent_children.get(*id) {
            let all_children_achieved = children.iter().all(|cid| {
                all_targets
                    .get(cid.as_str())
                    .is_some_and(|t| t.status == Status::Achieved)
            });
            if all_children_achieved && !children.is_empty() && target.status != Status::Achieved {
                stale.push(format!(
                    "🎯{id} {}: all sub-targets achieved but parent is {:?}",
                    target.name, target.status,
                ));
            }
        }

        // Target marked identified but has converging/achieved children.
        if target.status == Status::Identified
            && let Some(children) = parent_children.get(*id)
        {
            let has_progressed_child = children.iter().any(|cid| {
                all_targets
                    .get(cid.as_str())
                    .is_some_and(|t| t.status != Status::Identified)
            });
            if has_progressed_child {
                stale.push(format!(
                    "🎯{id} {}: still identified but has progressed sub-targets",
                    target.name,
                ));
            }
        }

        // Stale discovery: identified with no activity and old discovered date (>90 days).
        if target.status == Status::Identified {
            let age = chrono::Local::now().date_naive() - target.discovered;
            if age.num_days() > 90 {
                stale.push(format!(
                    "🎯{id} {}: identified for {} days with no progress",
                    target.name,
                    age.num_days(),
                ));
            }
        }
    }

    if !stale.is_empty() {
        out.push_str("## Stale targets\n\n");
        for s in &stale {
            out.push_str(&format!("- {s}\n"));
        }
        out.push('\n');
    }

    out
}

/// Render the detail block for a single frontier target — acceptance
/// criteria, context, tags, and relevant edges. Used when
/// `frontier_details` is true on [`summary`] (via `bullseye_convergence`),
/// so the caller gets the same information a `bullseye_get` would
/// return for each frontier entry, without round-tripping.
fn render_frontier_detail(
    out: &mut String,
    id: &str,
    all_targets: &BTreeMap<String, crate::schema::Target>,
) {
    let Some(t) = all_targets.get(id) else {
        return;
    };
    if !t.acceptance.is_empty() {
        out.push_str("    Acceptance:\n");
        for line in &t.acceptance {
            out.push_str(&format!("      - {line}\n"));
        }
    }
    if !t.context.is_empty() {
        // Indent context to keep it visually nested under its target.
        // Multi-line context is flattened to a single indented block.
        let indented = t
            .context
            .lines()
            .map(|l| format!("      {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&format!("    Context:\n{indented}\n"));
    }
    if !t.depends_on.is_empty() {
        let deps: Vec<String> = t.depends_on.iter().map(|d| format!("🎯{d}")).collect();
        out.push_str(&format!("    Depends on: {}\n", deps.join(", ")));
    }
    if !t.verifies.is_empty() {
        let vs: Vec<String> = t.verifies.iter().map(|v| format!("🎯{v}")).collect();
        out.push_str(&format!("    Verifies: {}\n", vs.join(", ")));
    }
    if let Some(ref rework) = t.rework {
        out.push_str(&format!("    Rework: 🎯{rework}\n"));
    }
    if !t.tags.is_empty() {
        out.push_str(&format!("    Tags: {}\n", t.tags.join(", ")));
    }
    out.push('\n');
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
