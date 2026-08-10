// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, HashSet};

use crate::schema::{Status, TargetsFile};

/// Banner + legend describing repo-scope frontier ordering. Rendered
/// at the top of the frontier section in every repo-scope output
/// (`bullseye_frontier`, `bullseye_summary`, `bullseye_convergence`)
/// so agents reading the annotations see the correct framing inline
/// and don't default to WSJF/SAFe reasoning from training-data habit.
/// Portfolio-scope ranking lives in [`crate::portfolio`] and *does*
/// use WSJF — the banner exists to stop that framing leaking into
/// repo-scope decisions.
///
/// Schema v5 (🎯T25) collapsed the verify / checkpoint / tunnel
/// apparatus. The frontier is now the parallelisable set, ordered by
/// `depends_on` shape alone: a target's unblocking fanout (how many
/// active targets depend on it) is the only ranking signal, with the
/// target ID as a deterministic tiebreak.
pub const REPO_SCOPE_BANNER: &str = "\
> **Repo-scope ordering**: max unblocking fanout, then target ID. The frontier is the \
parallelisable set — work it in parallel where possible. \
WSJF/value/cost/SAFe framing is portfolio-scope only — do not use it at repo scope.
>
> **Legend**: `fanout=N` = N active downstream target(s) blocked by this one.

";

/// A target in the frontier: unblocked and ready for work.
#[derive(Debug, Clone)]
pub struct FrontierTarget {
    pub id: String,
    pub name: String,
    pub status: Status,
    pub tags: Vec<String>,
}

/// Compute the frontier: active targets with all dependencies satisfied.
///
/// A target is in the frontier if:
/// - It is active (not terminal — neither achieved nor set aside).
/// - It is not ownership-excluded (`owned_by` is None) — 🎯T43.
/// - It has no in-flight dependencies (every `depends_on` ID resolves
///   to a terminal target — achieved or set-aside — or is absent).
///
/// Ownership exclusion does **not** make a target terminal: dependents
/// stay blocked until the target is achieved or set aside.
pub fn frontier(file: &TargetsFile) -> Vec<FrontierTarget> {
    frontier_on(file, chrono::Utc::now().date_naive())
}

/// Frontier as of `today` (🎯T50 date-gated postponement).
pub fn frontier_on(file: &TargetsFile, today: chrono::NaiveDate) -> Vec<FrontierTarget> {
    let active = file.active();

    active
        .iter()
        .filter(|(_, t)| {
            // Owned-elsewhere targets stay active for dependency
            // blocking but are not work for *this* owner.
            if t.owned_by.is_some() {
                return false;
            }
            // Date gate: still postponed until a future calendar day.
            if let Some(until) = t.postponed_until
                && until > today
            {
                return false;
            }
            // Opaque predicate still set: excluded until agent clears via wake.
            if t.postpone_predicate
                .as_ref()
                .is_some_and(|p| !p.trim().is_empty())
            {
                // If date was set and is now due (or absent), predicate alone
                // still holds the target off the frontier.
                if t.postponed_until.is_none_or(|u| u <= today) {
                    return false;
                }
            }
            // All dependencies must be in a terminal disposition
            // (achieved or set aside).
            t.depends_on.iter().all(|dep| {
                file.targets
                    .get(dep.as_str())
                    .is_some_and(|d| d.status.is_terminal())
            })
        })
        .map(|(id, t)| FrontierTarget {
            id: id.to_string(),
            name: t.name.clone(),
            status: t.status,
            tags: t.tags.clone(),
        })
        .collect()
}

/// Whether an active target is currently postponed off the frontier.
pub fn is_postponed(t: &crate::schema::Target, today: chrono::NaiveDate) -> bool {
    if let Some(until) = t.postponed_until
        && until > today
    {
        return true;
    }
    if t.postpone_predicate
        .as_ref()
        .is_some_and(|p| !p.trim().is_empty())
        && t.postponed_until.is_none_or(|u| u <= today)
    {
        return true;
    }
    false
}

/// Active targets excluded from the local frontier because another
/// owner is driving them (🎯T43).
pub fn owned_elsewhere(file: &TargetsFile) -> Vec<(&str, &crate::schema::Target)> {
    file.active()
        .into_iter()
        .filter(|(_, t)| t.owned_by.is_some())
        .collect()
}

/// Number of currently-active targets that list `id` in their
/// `depends_on` — the "unblocking fanout" score used as the primary
/// signal in repo-level frontier ordering. Higher means finishing
/// this target unblocks more downstream work.
pub fn unblocking_fanout(file: &TargetsFile, id: &str) -> usize {
    file.active()
        .values()
        .filter(|t| t.depends_on.iter().any(|d| d == id))
        .count()
}

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
/// - **children** — hierarchical children by ID convention (`T1.1` of `T1`).
/// - **parents** — hierarchical parent by ID convention (`T1` of `T1.1`).
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

/// Validate the targets file. Returns the union of structural errors
/// and stylistic warnings — used by `bullseye_validate` so the report
/// surfaces every issue. Downstream tools that need a hard gate
/// (frontier, convergence, portfolio) should call
/// [`validate_blocking`] instead, which excludes warnings so a
/// cosmetic violation (e.g. a non-conforming ID format from a typo or
/// from tool-call experimentation) doesn't strand the rest of the
/// graph and lock the user out of retiring or closing the offending
/// target.
pub fn validate(file: &TargetsFile) -> Vec<String> {
    let mut all = validate_blocking(file);
    all.extend(validate_warnings(file));
    all
}

/// Whether a target ID conforms to a recognised namespace. `T<N>` /
/// `T<N>.<M>` are hand-authored / allocated targets (🎯T28); `GH<N>`
/// (and the reserved multi-repo `GH<slug>-<N>`) are GitHub-issue
/// mirrors whose number IS the upstream issue number, so no
/// local↔remote mapping is needed (🎯T34). Non-conforming IDs still
/// function everywhere — this only governs the advisory display warning.
fn id_is_conforming(id: &str) -> bool {
    if let Some(rest) = id.strip_prefix('T') {
        return !rest.is_empty() && rest.split('.').all(|p| p.parse::<u32>().is_ok());
    }
    if let Some(rest) = id.strip_prefix("GH") {
        // The trailing '-'-delimited component is the issue number; an
        // optional leading repo slug (reserved for multi-repo
        // mirroring) may precede it.
        return rest
            .rsplit('-')
            .next()
            .is_some_and(|n| !n.is_empty() && n.parse::<u32>().is_ok());
    }
    false
}

/// Stylistic warnings — issues worth flagging in a report but not
/// severe enough to block frontier/convergence operations. Today this
/// is just the ID-format check; new entries belong here when the
/// invariant is "advisory" (cosmetic, not load-bearing for graph
/// logic).
pub fn validate_warnings(file: &TargetsFile) -> Vec<String> {
    let mut warnings = Vec::new();
    for id in file.targets.keys() {
        // ID format. Bullseye keys the entire graph by the ID string
        // itself; non-conforming IDs render fine and participate in
        // depends_on / cross-edge resolution like any other string.
        // The only thing the format buys is consistency in displays.
        // Treat as advisory so a user who ended up with a non-conforming
        // ID (bad tool call, hand edit, import quirk) can still retire
        // or set-aside it via the normal tools.
        if !id_is_conforming(id) {
            warnings.push(format!(
                "{id}: invalid target ID format (expected T<N>, T<N>.<M>, or GH<N>) — \
                 advisory; the target is fully operable, retire or rename it via direct \
                 YAML edit if you want the warning gone"
            ));
        }
    }
    warnings.extend(graph_hygiene_warnings(file));
    warnings
}

/// Advisory graph-shape risks (🎯T53 empty-frontier / buried-leaf;
/// 🎯T59 merge completeness; 🎯T60 fake-edge). Never hard-block mutations.
pub fn graph_hygiene_warnings(file: &TargetsFile) -> Vec<String> {
    let mut warnings = Vec::new();
    let active = file.active();
    if active.is_empty() {
        return warnings;
    }
    let today = chrono::Utc::now().date_naive();
    let front = frontier_on(file, today);
    if front.is_empty() {
        let blocked = active
            .iter()
            .filter(|(_, t)| t.owned_by.is_none() && !is_postponed(t, today))
            .count();
        if blocked > 0 {
            warnings.push(format!(
                "graph hygiene: {blocked} active target(s) but frontier is empty \
                 (all blocked on unfinished depends_on) — review dependencies or \
                 achieve/defer blockers; advisory only"
            ));
        }
    }
    // Leaves: active, unblocked, no dependents, and no acceptance? Skip.
    // Tunnel-ish: active target whose every path to a terminal is blocked by
    // many active deps — simpler: active with depends_on all active (not on
    // frontier) and no active target depends on it (work leaf buried).
    for (id, t) in &active {
        if t.owned_by.is_some() || is_postponed(t, today) {
            continue;
        }
        let on_front = front.iter().any(|f| f.id == *id);
        if on_front {
            continue;
        }
        // Buried active node: not frontier, still active.
        let has_active_dep = t.depends_on.iter().any(|d| {
            file.targets
                .get(d.as_str())
                .is_some_and(|x| !x.status.is_terminal())
        });
        let fanout = unblocking_fanout(file, id);
        if has_active_dep && fanout == 0 {
            warnings.push(format!(
                "{id}: graph hygiene: active leaf blocked by unfinished deps and \
                 unblocks nothing — possible tunnel/dead branch; advisory only"
            ));
        }
    }
    warnings.extend(merge_completeness_warnings(file));
    warnings.extend(fake_edge_warnings(file));
    warnings
}

/// Advisory merge-step completeness for multi-predecessor nodes (🎯T59).
///
/// When a target has 2+ `depends_on` edges and some but not all
/// predecessors are terminal (achieved/set_aside), "almost green"
/// partial fan-in can look complete while unfinished preds remain.
/// Counts expected vs terminal; never hard-blocks.
pub fn merge_completeness_warnings(file: &TargetsFile) -> Vec<String> {
    let mut warnings = Vec::new();
    for (id, t) in file.active() {
        let expected = t.depends_on.len();
        if expected < 2 {
            continue;
        }
        let mut terminal = 0usize;
        let mut unfinished = 0usize;
        for dep in &t.depends_on {
            match file.targets.get(dep.as_str()) {
                Some(d) if d.status.is_terminal() => terminal += 1,
                // Missing deps are unfinished (validate_blocking will also
                // hard-error; still count for a consistent advisory).
                _ => unfinished += 1,
            }
        }
        // Partial fan-in only: some progress and some remaining work.
        if terminal > 0 && unfinished > 0 {
            warnings.push(format!(
                "{id}: merge completeness: {terminal}/{expected} predecessors \
                 terminal, {unfinished} still active — partial fan-in (almost \
                 green); advisory only"
            ));
        }
    }
    warnings
}

/// Advisory fake-edge detection for sequential-only `depends_on` (🎯T60).
///
/// **Heuristic** (prefer precision over spam): for each edge A→B where B
/// is active and A exists, join B's acceptance lines + context
/// (case-folded). The edge is treated as **consuming** (no warning) if
/// any of the following hold:
///
/// 1. B's text mentions A's id (`T1`, `🎯T1`, hierarchical forms) with
///    simple token boundaries so `T1` does not match `T10` / `T1.2`.
/// 2. B's text contains a **significant token** from A's name
///    (alphanumeric runs of length ≥ 4, minus a small stopword list).
/// 3. B's text contains a significant token from any of A's acceptance
///    lines (same token rules) — a lexical stand-in for "outcome".
///
/// Otherwise the edge looks like typed order only (**fake edge**):
/// candidates for parallel work or for dropping the edge. Advisory only;
/// never hard-blocks. Origin:
/// `docs/analysis/graph-engineering-evaluation-2026-08.md` §3.1.
///
/// Missing predecessors are skipped (validate_blocking hard-errors).
/// Terminal (achieved/set_aside) dependents are skipped.
pub fn fake_edge_warnings(file: &TargetsFile) -> Vec<String> {
    let mut warnings = Vec::new();
    for (id, t) in file.active() {
        if t.depends_on.is_empty() {
            continue;
        }
        let consumer = dependent_consumer_text(t);
        for dep_id in &t.depends_on {
            let Some(pred) = file.targets.get(dep_id.as_str()) else {
                continue;
            };
            if edge_looks_consuming(&consumer, dep_id, pred) {
                continue;
            }
            warnings.push(format!(
                "{id}: fake edge: depends_on {dep_id} looks sequential-only \
                 (acceptance/context does not reference predecessor \
                 id/name/outcome) — candidates for parallel or drop edge; \
                 advisory only"
            ));
        }
    }
    warnings
}

/// Acceptance + context of a dependent, lowercased for matching.
fn dependent_consumer_text(t: &crate::schema::Target) -> String {
    let mut s = String::new();
    for a in &t.acceptance {
        s.push_str(a);
        s.push('\n');
    }
    s.push_str(&t.context);
    s.to_ascii_lowercase()
}

/// True when B's consumer text appears to consume predecessor A.
fn edge_looks_consuming(consumer_lower: &str, pred_id: &str, pred: &crate::schema::Target) -> bool {
    if text_mentions_target_id(consumer_lower, pred_id) {
        return true;
    }
    if significant_tokens(&pred.name)
        .into_iter()
        .any(|tok| consumer_lower.contains(&tok))
    {
        return true;
    }
    for acc in &pred.acceptance {
        if significant_tokens(acc)
            .into_iter()
            .any(|tok| consumer_lower.contains(&tok))
        {
            return true;
        }
    }
    false
}

/// Whether `haystack` (already lowercased) mentions `id` with token-ish
/// boundaries. Accepts an optional leading `🎯`. Rejects longer IDs that
/// share a prefix (`T1` vs `T10`, `T1` vs `T1.2`).
fn text_mentions_target_id(haystack: &str, id: &str) -> bool {
    let id_l = id.to_ascii_lowercase();
    let needles = [id_l.clone(), format!("🎯{id_l}")];
    for needle in &needles {
        let mut start = 0;
        while let Some(rel) = haystack[start..].find(needle.as_str()) {
            let abs = start + rel;
            let end = abs + needle.len();
            let left_ok = abs == 0 || !haystack.as_bytes()[abs - 1].is_ascii_alphanumeric();
            let right_ok = match haystack.as_bytes().get(end) {
                None => true,
                Some(b) if b.is_ascii_alphanumeric() => false,
                Some(b) if *b == b'.' => {
                    // Hierarchical continuation: T1.2 when matching T1.
                    !haystack
                        .as_bytes()
                        .get(end + 1)
                        .is_some_and(|c| c.is_ascii_alphanumeric())
                }
                Some(_) => true,
            };
            if left_ok && right_ok {
                return true;
            }
            // Step past the first char of the rejected hit. Advancing a
            // single byte would land inside a multi-byte char (the `🎯`
            // needle prefix) and panic on the next `haystack[start..]`.
            // No char left means an empty needle matched at end-of-string:
            // nothing further can match, so stop rather than step past the
            // end.
            let Some(hit_char) = haystack[abs..].chars().next() else {
                break;
            };
            start = abs + hit_char.len_utf8();
        }
    }
    false
}

/// Alphanumeric runs of length ≥ 4, lowercased, minus common stopwords.
/// Used as a low-spam proxy for name/outcome consumption.
fn significant_tokens(text: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "with", "from", "that", "this", "when", "have", "does", "only", "into", "over", "after",
        "before", "must", "will", "should", "target", "tests", "test", "work", "file", "docs",
        "code", "when", "than", "then", "them", "they", "their", "there", "about", "above",
        "below", "under", "each", "other", "such", "also", "just", "more", "most", "some", "same",
        "both", "been", "being", "were", "what", "which", "while", "where", "your", "ours", "into",
        "onto", "via", "using", "used", "make", "made", "need", "needs", "able",
    ];
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 4)
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

/// Structural errors that block downstream graph operations. Frontier,
/// convergence, portfolio, and startup-context all gate on this — if
/// it returns non-empty, the graph is broken in ways that would make
/// the next-action recommendation meaningless. Stylistic warnings are
/// reported separately by [`validate_warnings`].
pub fn validate_blocking(file: &TargetsFile) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen_ids: HashSet<&str> = HashSet::new();

    for (id, t) in &file.targets {
        // Duplicate check.
        if !seen_ids.insert(id.as_str()) {
            errors.push(format!("{id}: duplicate target ID"));
        }
        if id_ends_in_zero_dotted_segment(id) {
            errors.push(format!(
                "{id}: dotted target IDs whose final segment is zero are disallowed \
                 because humans conflate T4 and T4.0"
            ));
        }

        // Value/cost: 0.0 means "not set at repo scope" (portfolio-scope
        // metadata is optional). Only reject explicitly negative values,
        // which are always a mistake, and non-zero sub-1 values that
        // would produce meaningless WSJF ratios.
        if t.value < 0.0 {
            errors.push(format!("{id}: value must be non-negative, got {}", t.value));
        }
        if t.cost < 0.0 {
            errors.push(format!("{id}: cost must be non-negative, got {}", t.cost));
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

        // Cross-repo edges: format-only validation. We intentionally
        // do NOT check that the referenced repo or target exists — the
        // whole point of cross-repo edges is to track references that
        // live outside this repo's graph, possibly outside any scanned
        // portfolio. But each edge must carry a non-empty `repo` and
        // at least one of `target` / `capability`, otherwise it's
        // structurally meaningless.
        for edge in t.cross_depends.iter().chain(t.cross_enables.iter()) {
            if edge.repo.trim().is_empty() {
                errors.push(format!("{id}: cross-repo edge has empty repo"));
            }
            if edge.target.as_deref().unwrap_or("").is_empty()
                && edge.capability.as_deref().unwrap_or("").is_empty()
            {
                errors.push(format!(
                    "{id}: cross-repo edge to {} must set `target` or `capability`",
                    edge.repo,
                ));
            }
        }

        // Set-aside disposition requires a non-empty rationale (🎯T18).
        // The rationale is the load-bearing artefact: it carries the
        // parked / deferred / wont_fix nuance that the schema deliberately
        // doesn't taxonomise. Empty or whitespace-only reasons are
        // rejected. Conversely, set_aside_reason on a non-set-aside
        // status is meaningless — flag it rather than silently
        // preserving a stale reason from a prior disposition.
        match t.status {
            Status::SetAside => {
                if t.set_aside_reason
                    .as_deref()
                    .is_none_or(|r| r.trim().is_empty())
                {
                    errors.push(format!(
                        "{id}: status set_aside requires a non-empty set_aside_reason",
                    ));
                }
            }
            _ => {
                if t.set_aside_reason.is_some() {
                    errors.push(format!(
                        "{id}: set_aside_reason is only valid on status set_aside (status is {:?})",
                        t.status,
                    ));
                }
            }
        }

        // Achieve attestation (🎯T58): soft words-in-a-box on retirement.
        // Required by the achieve API path; legacy achieved targets may
        // lack the field. Empty / whitespace-only values are invalid when
        // present. Attestation on a non-achieved status is stale noise.
        if let Some(a) = &t.attestation {
            if a.trim().is_empty() {
                errors.push(format!("{id}: attestation must be non-empty when present",));
            } else if t.status != Status::Achieved {
                errors.push(format!(
                    "{id}: attestation is only valid on status achieved (status is {:?})",
                    t.status,
                ));
            }
        }

        // Ownership exclusion (🎯T43): both owner and reason must be
        // non-empty when the field is present. Terminal targets should
        // not carry ownership — clear it first.
        if let Some(ob) = &t.owned_by {
            if ob.owner.trim().is_empty() {
                errors.push(format!("{id}: owned_by.owner must be non-empty"));
            }
            if ob.reason.trim().is_empty() {
                errors.push(format!("{id}: owned_by.reason must be non-empty"));
            }
            if t.status.is_terminal() {
                errors.push(format!(
                    "{id}: owned_by is only valid on active targets (status is {:?})",
                    t.status,
                ));
            }
        }

        // Strategy validation: command and trigger must be non-empty.
        if let Some(ref strategy) = t.strategy {
            if strategy.command.trim().is_empty() {
                errors.push(format!("{id}: strategy.command must not be empty"));
            }
            if strategy.trigger.trim().is_empty() {
                errors.push(format!("{id}: strategy.trigger must not be empty"));
            }
        }
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

fn id_ends_in_zero_dotted_segment(id: &str) -> bool {
    id.strip_prefix('T')
        .and_then(|rest| rest.rsplit('.').next().filter(|_| rest.contains('.')))
        == Some("0")
}

/// Produce a concise startup context summary for agent consumption.
pub fn startup_context(file: &TargetsFile, file_path: &str, recent_days: u32) -> String {
    let cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(recent_days as i64);

    let active = file.active();
    let active_count = active.len();

    let errors = validate_blocking(file);
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
    recent_achieved.sort_by_key(|b| std::cmp::Reverse(b.1.achieved));

    let mut out = String::new();

    out.push_str(&format!(
        "# Startup context\nFile: {file_path}\nBinary: bullseye {}\nSchema: file={} binary_supports={}\nActive: {active_count} target(s), Frontier: {} ready for work\n\n",
        env!("CARGO_PKG_VERSION"),
        file.schema_version
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unset".into()),
        crate::schema::CURRENT_SCHEMA_VERSION,
        front.len(),
    ));

    let hy = graph_hygiene_warnings(file);
    if !hy.is_empty() {
        out.push_str("## Graph hygiene (advisory)\n\n");
        for w in &hy {
            out.push_str(&format!("- {w}\n"));
        }
        out.push('\n');
    }

    if !front.is_empty() {
        out.push_str("## Frontier (unblocked, ready for work)\n\n");
        for ft in &front {
            out.push_str(&format!("🎯{} {}\n", ft.id, ft.name));
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
            if let Some(att) = &target.attestation {
                out.push_str(&format!("  attestation: {att}\n"));
            }
        }
        out.push('\n');
    }

    if !errors.is_empty() {
        out.push_str("## Warnings\n\n");
        out.push_str(&format!("Validation errors: {}\n\n", errors.join("; ")));
    }

    out
}

/// Produce the startup-context response for a project that has no
/// `bullseye.yaml`. Unlike most tools, startup_context is often called
/// automatically at session start before the caller knows whether the
/// repo uses bullseye, so it needs to degrade gracefully instead of
/// failing the tool call.
pub fn startup_context_no_file(cwd: &str) -> String {
    format!(
        "# Startup context\n\
         File: (no bullseye.yaml found under {cwd})\n\
         \n\
         This project is not using bullseye yet. Run `bullseye_init` to \
         create a starter `bullseye.yaml`, or ignore this notice if \
         targets aren't appropriate for this repo.\n",
    )
}

/// Produce the startup-context response for a project whose
/// `bullseye.yaml` exists but can't be read or parsed. Surfaces the
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
         ⚠ bullseye.yaml could not be loaded: {error}\n\
         \n\
         Session start is continuing without target context. Fix the \
         file (common causes: YAML syntax error, unresolved rebase \
         marker, permission issue) and re-run `bullseye_startup_context` \
         to recover.\n",
    )
}

/// An annotated frontier entry in repo-level ordering. Carries the
/// unblocking fanout score so renderers can display it without
/// recomputing.
pub struct RankedFrontier<'a> {
    pub target: &'a FrontierTarget,
    /// Count of active targets that have this one in their
    /// `depends_on` list — the "unblocking fanout" score.
    pub fanout: usize,
}

/// Order a frontier by repo-level prioritisation.
///
/// Sort keys, in order:
///   1. Descending unblocking fanout — finishing a high-fanout
///      target frees more downstream work.
///   2. Ascending target ID — pure determinism for reproducible
///      output.
///
/// This function intentionally does NOT consume `value`, `cost`, or
/// `momentum`. Those are portfolio-scope inputs (see
/// [`crate::schema::Target::value`]); repo-level ordering is driven
/// purely by the graph shape, and within the frontier the agent is
/// expected to fan work out in parallel rather than execute
/// strictly in rank order.
pub fn rank_frontier<'a>(
    file: &TargetsFile,
    frontier_targets: &'a [FrontierTarget],
) -> Vec<RankedFrontier<'a>> {
    let mut ranked: Vec<RankedFrontier<'a>> = frontier_targets
        .iter()
        .map(|ft| RankedFrontier {
            target: ft,
            fanout: unblocking_fanout(file, &ft.id),
        })
        .collect();

    ranked.sort_by(|a, b| b.fanout.cmp(&a.fanout).then(a.target.id.cmp(&b.target.id)));

    ranked
}

/// Produce a consolidated status overview for agent consumption.
///
/// The frontier section is ordered by repo-level prioritisation
/// (🎯T7): ascending distance to the nearest checkpoint,
/// tiebroken by descending unblocking fanout, then by target ID.
///
/// The `momentum` parameter is retained for wire compatibility with
/// the previous (portfolio-style) ranking but is **not consumed**
/// for repo-level ordering — momentum is a portfolio-scope signal
/// and belongs in [`crate::portfolio`], not here. Passing a momentum
/// map has no effect on the frontier order. Callers targeting
/// portfolio-level work should use [`crate::portfolio`] directly.
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
    // Momentum is intentionally ignored at repo scope; see doc
    // comment. Silence the unused-parameter warning without
    // changing the public API.
    let _ = momentum;
    let mut out = String::new();

    let errors = validate_blocking(file);
    let all_targets = &file.targets;
    let active = file.active();
    let achieved = file.achieved();
    let set_aside_count = file.set_aside().len();

    let disposition_parts = {
        let mut parts = vec![
            format!("{} active", active.len()),
            format!("{} achieved", achieved.len()),
        ];
        if set_aside_count > 0 {
            parts.push(format!("{set_aside_count} set aside"));
        }
        parts.join(", ")
    };

    out.push_str(&format!(
        "# Summary\nFile: {file_path}\nTotal: {} target(s) — {disposition_parts}\n\n",
        all_targets.len(),
    ));

    // Advisory graph hygiene (🎯T53 / 🎯T59) — same surface as validate
    // warnings and startup_context. Non-blocking; partial fan-in and
    // empty-frontier shape risks belong here so summary does not hide them.
    let hy = graph_hygiene_warnings(file);
    if !hy.is_empty() {
        out.push_str("## Graph hygiene (advisory)\n\n");
        for w in &hy {
            out.push_str(&format!("- {w}\n"));
        }
        out.push('\n');
    }

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

    // --- 2. Frontier (ordered by repo-level prioritisation) ---
    //
    // Descending unblocking fanout, then by ID. See [`rank_frontier`]
    // for the full rule and rationale. Value/cost/momentum
    // intentionally not consumed here — those are portfolio-scope
    // signals.
    if errors.is_empty() {
        let front = frontier(file);
        let ranked = rank_frontier(file, &front);

        out.push_str("## Frontier (unblocked, ready for work)\n\n");
        out.push_str(REPO_SCOPE_BANNER);
        if ranked.is_empty() {
            out.push_str("(no targets ready for work)\n");
        } else {
            for rf in &ranked {
                let ft = rf.target;
                out.push_str(&format!(
                    "🎯{id} {name}  [{status:?}] — fanout={fan}\n",
                    id = ft.id,
                    name = ft.name,
                    status = ft.status,
                    fan = rf.fanout,
                ));
                if frontier_details {
                    render_frontier_detail(&mut out, &ft.id, all_targets);
                }
            }
        }
        out.push('\n');

        // --- 3. Blocked targets ---
        let front_ids: HashSet<&str> = ranked.iter().map(|r| r.target.id.as_str()).collect();
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
                            .is_none_or(|d| !d.status.is_terminal())
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

    // --- 5. Owned elsewhere (🎯T43) ---
    //
    // Active targets driven by someone else. Distinct from set_aside:
    // status is unchanged, dependents stay blocked, and the entry
    // names the other owner plus a reason.
    let mut elsewhere = owned_elsewhere(file);
    if !elsewhere.is_empty() {
        elsewhere.sort_by(|a, b| a.0.cmp(b.0));
        out.push_str("## Owned elsewhere\n\n");
        for (id, t) in &elsewhere {
            if let Some(ob) = &t.owned_by {
                out.push_str(&format!(
                    "🎯{id} {} — owner: {} — {}\n",
                    t.name, ob.owner, ob.reason
                ));
            }
        }
        out.push('\n');
    }

    // --- 6. Set aside targets ---
    //
    // Terminal but not achieved. Surfaced in their own group so a
    // reviewer can see what was decided not to do and why, without
    // those decisions inflating the achievement record. The reason
    // line is the load-bearing artefact: its absence would leave the
    // disposition unmotivated. See 🎯T18.
    let set_aside = file.set_aside();
    if !set_aside.is_empty() {
        out.push_str("## Set aside\n\n");
        for (id, t) in &set_aside {
            let reason = t
                .set_aside_reason
                .as_deref()
                .unwrap_or("(no reason recorded)");
            out.push_str(&format!("🎯{id} {} — {reason}\n", t.name));
        }
        out.push('\n');
    }

    // --- 7. Recently achieved (with attestation when present) ---
    //
    // Soft visibility for 🎯T58: achievements that carry a free-text
    // attestation show the note here so summary does not require a
    // round-trip to view=target. Legacy achievements without the field
    // still appear with date only. Window matches startup context default.
    let recent_cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(14);
    let mut recent_achieved: Vec<(&str, &crate::schema::Target)> = achieved
        .into_iter()
        .filter(|(_, t)| t.achieved.is_some_and(|d| d >= recent_cutoff))
        .collect();
    if !recent_achieved.is_empty() {
        recent_achieved.sort_by_key(|b| std::cmp::Reverse(b.1.achieved));
        out.push_str("## Recently achieved (last 14 days)\n\n");
        for (id, t) in &recent_achieved {
            let date = t.achieved.map_or("?".to_string(), |d| d.to_string());
            out.push_str(&format!("🎯{id} {} (achieved {date})\n", t.name));
            if let Some(att) = &t.attestation {
                out.push_str(&format!("  attestation: {att}\n"));
            }
        }
        out.push('\n');
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

#[cfg(test)]
mod t50_t53_tests {
    use super::*;
    use crate::schema::{Status, Target, TargetsFile};
    use chrono::NaiveDate;
    use std::collections::BTreeMap;

    fn tgt(name: &str, deps: &[&str]) -> Target {
        Target {
            name: name.into(),
            status: Status::Identified,
            value: 0.0,
            cost: 0.0,
            actual_cost: None,
            set_aside_reason: None,
            attestation: None,
            acceptance: vec!["a".into()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: deps.iter().map(|s| (*s).to_string()).collect(),
            cross_depends: vec![],
            cross_enables: vec![],
            tags: vec![],
            strategy: None,
            origin: "test".into(),
            discovered: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            achieved: None,
            owned_by: None,
            postponed_until: None,
            postpone_predicate: None,
        }
    }

    #[test]
    fn future_postponement_excludes_from_frontier() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        let mut t = tgt("p", &[]);
        t.postponed_until = Some(NaiveDate::from_ymd_opt(2099, 1, 1).unwrap());
        file.targets.insert("T1".into(), t);
        let today = NaiveDate::from_ymd_opt(2026, 7, 29).unwrap();
        assert!(frontier_on(&file, today).is_empty());
        // Past date + no predicate → on frontier
        file.targets.get_mut("T1").unwrap().postponed_until =
            Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap());
        assert_eq!(frontier_on(&file, today).len(), 1);
    }

    #[test]
    fn predicate_keeps_off_frontier_after_date_due() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        let mut t = tgt("p", &[]);
        t.postponed_until = Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap());
        t.postpone_predicate = Some("wait for release".into());
        file.targets.insert("T1".into(), t);
        let today = NaiveDate::from_ymd_opt(2026, 7, 29).unwrap();
        assert!(frontier_on(&file, today).is_empty());
        file.targets.get_mut("T1").unwrap().postpone_predicate = None;
        assert_eq!(frontier_on(&file, today).len(), 1);
    }

    #[test]
    fn hygiene_warns_when_all_active_blocked() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        file.targets.insert("T1".into(), tgt("blocker", &[]));
        file.targets.insert("T2".into(), tgt("blocked", &["T1"]));
        // Put T1 in converging with dep on missing terminal - make T1 depend on missing
        file.targets.get_mut("T1").unwrap().depends_on = vec!["T99".into()];
        // T99 doesn't exist - validate_blocking would error; for hygiene we need
        // active with empty frontier: T1 blocked on T99 (dangling), T2 on T1.
        // Actually dangling deps: is_some_and false means not terminal → blocks frontier
        let w = graph_hygiene_warnings(&file);
        assert!(
            w.iter()
                .any(|s| s.contains("frontier is empty") || s.contains("tunnel")),
            "warnings={w:?}"
        );
    }

    /// 🎯T59: multi-predecessor merge with mixed terminal/active deps
    /// surfaces expected vs terminal counts and partial-fan-in language.
    #[test]
    fn merge_completeness_flags_partial_fan_in() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        let mut a = tgt("pred-a", &[]);
        a.status = Status::Achieved;
        a.achieved = Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        file.targets.insert("T1".into(), a);
        // T2 still active (identified).
        file.targets.insert("T2".into(), tgt("pred-b", &[]));
        // Set-aside also counts as terminal.
        let mut c = tgt("pred-c", &[]);
        c.status = Status::SetAside;
        c.set_aside_reason = Some("parked".into());
        file.targets.insert("T3".into(), c);
        // Merge node with 3 preds: 2 terminal, 1 active.
        file.targets
            .insert("T4".into(), tgt("merge", &["T1", "T2", "T3"]));

        let w = merge_completeness_warnings(&file);
        assert_eq!(w.len(), 1, "warnings={w:?}");
        assert!(
            w[0].contains("merge completeness")
                && w[0].contains("2/3")
                && w[0].contains("partial fan-in")
                && w[0].contains("advisory only"),
            "warnings={w:?}"
        );

        // Folded into full hygiene / validate warnings.
        let hy = graph_hygiene_warnings(&file);
        assert!(
            hy.iter()
                .any(|s| s.contains("merge completeness") && s.contains("T4")),
            "hygiene={hy:?}"
        );
        let vw = validate_warnings(&file);
        assert!(
            vw.iter().any(|s| s.contains("merge completeness")),
            "validate_warnings={vw:?}"
        );

        // Summary surfaces the same advisory section.
        let sum = summary(&file, "test.yaml", None, false);
        assert!(
            sum.contains("## Graph hygiene (advisory)")
                && sum.contains("merge completeness")
                && sum.contains("2/3"),
            "summary={sum}"
        );
    }

    #[test]
    fn merge_completeness_skips_all_active_or_single_dep() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        file.targets.insert("T1".into(), tgt("a", &[]));
        file.targets.insert("T2".into(), tgt("b", &[]));
        // Multi-pred but zero terminal — not "almost green".
        file.targets
            .insert("T3".into(), tgt("merge-all-active", &["T1", "T2"]));
        // Single dep with terminal pred — not multi-pred.
        let mut done = tgt("done", &[]);
        done.status = Status::Achieved;
        done.achieved = Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        file.targets.insert("T4".into(), done);
        file.targets.insert("T5".into(), tgt("single-dep", &["T4"]));

        let w = merge_completeness_warnings(&file);
        assert!(w.is_empty(), "unexpected merge warnings: {w:?}");
    }

    #[test]
    fn merge_completeness_skips_fully_terminal_fan_in() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        for (id, name) in [("T1", "a"), ("T2", "b")] {
            let mut t = tgt(name, &[]);
            t.status = Status::Achieved;
            t.achieved = Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
            file.targets.insert(id.into(), t);
        }
        file.targets
            .insert("T3".into(), tgt("ready-merge", &["T1", "T2"]));
        let w = merge_completeness_warnings(&file);
        assert!(w.is_empty(), "fully terminal fan-in should not warn: {w:?}");
        // Merge node should be on the frontier.
        assert_eq!(frontier(&file).len(), 1);
        assert_eq!(frontier(&file)[0].id, "T3");
    }

    /// 🎯T60: sequential-only depends_on with no id/name/outcome reference
    /// in the dependent's acceptance/context → fake-edge advisory.
    #[test]
    fn fake_edge_flags_sequential_only_depends_on() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        // Short names + default acceptance "a" → no significant tokens.
        file.targets.insert("T1".into(), tgt("pred-xy", &[]));
        let mut b = tgt("dependent-only", &["T1"]);
        b.acceptance = vec!["ship the binary".into()];
        b.context = "unrelated prose about deployment order".into();
        file.targets.insert("T2".into(), b);

        let w = fake_edge_warnings(&file);
        assert_eq!(w.len(), 1, "warnings={w:?}");
        assert!(
            w[0].contains("fake edge")
                && w[0].contains("T2")
                && w[0].contains("T1")
                && w[0].contains("sequential-only")
                && w[0].contains("advisory only"),
            "warnings={w:?}"
        );

        let hy = graph_hygiene_warnings(&file);
        assert!(
            hy.iter()
                .any(|s| s.contains("fake edge") && s.contains("T2")),
            "hygiene={hy:?}"
        );
        let vw = validate_warnings(&file);
        assert!(
            vw.iter().any(|s| s.contains("fake edge")),
            "validate_warnings={vw:?}"
        );
        let sum = summary(&file, "test.yaml", None, false);
        assert!(
            sum.contains("## Graph hygiene (advisory)") && sum.contains("fake edge"),
            "summary={sum}"
        );
    }

    #[test]
    fn fake_edge_skips_when_dependent_mentions_pred_id() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        file.targets.insert("T1".into(), tgt("schema-work", &[]));
        let mut b = tgt("consumer", &["T1"]);
        b.acceptance = vec!["uses output of 🎯T1 in the merge step".into()];
        file.targets.insert("T2".into(), b);

        assert!(
            fake_edge_warnings(&file).is_empty(),
            "id reference should clear fake-edge: {:?}",
            fake_edge_warnings(&file)
        );
    }

    #[test]
    fn fake_edge_skips_when_dependent_mentions_pred_name_token() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        file.targets
            .insert("T1".into(), tgt("widget-renderer pipeline", &[]));
        let mut b = tgt("consumer", &["T1"]);
        b.acceptance = vec!["pipeline consumes widget-renderer artifacts".into()];
        file.targets.insert("T2".into(), b);

        assert!(
            fake_edge_warnings(&file).is_empty(),
            "name token should clear fake-edge: {:?}",
            fake_edge_warnings(&file)
        );
    }

    #[test]
    fn fake_edge_skips_when_dependent_mentions_pred_acceptance_outcome() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        let mut a = tgt("pred", &[]);
        a.acceptance = vec!["emit frobulator.schema.json for consumers".into()];
        file.targets.insert("T1".into(), a);
        let mut b = tgt("consumer", &["T1"]);
        b.acceptance = vec!["load frobulator.schema.json from pred output".into()];
        file.targets.insert("T2".into(), b);

        assert!(
            fake_edge_warnings(&file).is_empty(),
            "acceptance outcome token should clear fake-edge: {:?}",
            fake_edge_warnings(&file)
        );
    }

    #[test]
    fn fake_edge_id_boundary_does_not_match_longer_ids() {
        // T1 must not match text that only mentions T10 or T1.2.
        assert!(!text_mentions_target_id("depends on t10", "T1"));
        assert!(!text_mentions_target_id("after t1.2 lands", "T1"));
        assert!(text_mentions_target_id("after t1 lands", "T1"));
        assert!(text_mentions_target_id("uses 🎯t1.2 output", "T1.2"));
    }

    #[test]
    fn fake_edge_id_scan_survives_multibyte_utf8() {
        // A rejected `🎯t1` hit must not step into the middle of the 4-byte
        // emoji on the next iteration (the orthograph panic, 2026-08-08).
        assert!(!text_mentions_target_id("uses 🎯t1.2 output", "T1"));
        assert!(!text_mentions_target_id("blocked on 🎯t10", "T1"));
        assert!(!text_mentions_target_id(
            "🎯t1.2 and 🎯t7.2 and 🎯t8.1",
            "T1"
        ));
        // A later genuine mention is still found after a rejected hit.
        assert!(text_mentions_target_id("🎯t1.2 then 🎯t1 lands", "T1"));
        assert!(text_mentions_target_id("🎯t1.2 then t1 lands", "T1"));
        // Rejected hit ending the string, and non-emoji multi-byte text.
        assert!(!text_mentions_target_id("blocked on 🎯t1.2", "T1"));
        assert!(text_mentions_target_id("naïve — 🎯t1.2 → 🎯t1", "T1"));
        // Degenerate empty id (malformed ledger with an empty target key):
        // the empty needle is rejected at every position including
        // end-of-string, which must terminate rather than step past the end.
        assert!(!text_mentions_target_id("abc", ""));
        assert!(!text_mentions_target_id("abc🎯x9", ""));
    }

    #[test]
    fn fake_edge_skips_terminal_dependents_and_missing_preds() {
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: vec![],
            targets: BTreeMap::new(),
        };
        file.targets.insert("T1".into(), tgt("a", &[]));
        let mut done = tgt("done-chain", &["T1"]);
        done.status = Status::Achieved;
        done.achieved = Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        done.acceptance = vec!["shipped without referencing pred".into()];
        file.targets.insert("T2".into(), done);
        // Active with missing pred — skip that edge (blocking validate owns it).
        file.targets
            .insert("T3".into(), tgt("orphan-dep", &["T99"]));

        let w = fake_edge_warnings(&file);
        assert!(w.is_empty(), "unexpected fake-edge warnings: {w:?}");
    }
}
