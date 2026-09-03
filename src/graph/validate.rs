// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! The blocking-validation oracle plus advisory graph-hygiene warnings.
//! [`validate_blocking`] / [`validate_issues`] are the single source of
//! truth for "is this graph structurally sound"; everything else here
//! (`graph_hygiene_warnings` and friends) is cosmetic and never gates a
//! mutation.

use std::collections::HashSet;

use crate::ops;
use crate::schema::{Status, Target, TargetsFile};

use super::frontier::{frontier_on, is_postponed, unblocking_fanout};

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

/// A blocking validation error, attributed to the target that carries
/// it (🎯T64).
///
/// Every blocking error bullseye produces is target-scoped — there is
/// no whole-file error class — which is what makes degraded reads
/// possible: the offending targets can be named and excluded while the
/// rest of the graph answers normally. See
/// [`crate::graph::frontier_tolerant`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// ID of the target the error belongs to.
    pub target: String,
    /// The error text, without the `"{id}: "` prefix that [`Display`]
    /// adds.
    ///
    /// [`Display`]: std::fmt::Display
    pub message: String,
}

impl ValidationIssue {
    fn new(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.target, self.message)
    }
}

/// Structural errors that block downstream graph operations, as flat
/// strings. Thin wrapper over [`validate_issues`] for callers that only
/// want text; anything that needs to know *which* target is at fault
/// (degraded reads) should call [`validate_issues`] directly.
///
/// Stylistic warnings are reported separately by [`validate_warnings`].
pub fn validate_blocking(file: &TargetsFile) -> Vec<String> {
    validate_issues(file)
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// The still-open targets standing between `target` and work on it.
///
/// `depends_on` is a hard blocking edge — "must be achieved before work
/// on this one begins" — but until 🎯T80 the read surfaces never said
/// so. A blocked target rendered as `status: identified`, which reads
/// as *ready to start*, and an agent had to fetch every dependency and
/// compare statuses to learn otherwise. The frontier knew; the view an
/// agent actually reads denied it. Terminal dependencies (achieved or
/// set_aside) do not block, and unknown IDs are reported by validation
/// rather than counted here.
pub fn open_blockers(file: &TargetsFile, target: &Target) -> Vec<String> {
    target
        .depends_on
        .iter()
        .filter(|d| {
            file.targets
                .get(*d)
                .is_some_and(|dep| !dep.status.is_terminal())
        })
        .cloned()
        .collect()
}

/// Structural errors that block downstream graph operations, each
/// attributed to its target.
///
/// A non-empty result means part of the graph is broken. It does **not**
/// mean the whole file is unreadable: `view=frontier` / `list` / `target`
/// degrade around the named targets rather than refusing to answer
/// (🎯T64). `view=validate` is the one surface whose job *is* to report
/// these, so it reports them and nothing else.
pub fn validate_issues(file: &TargetsFile) -> Vec<ValidationIssue> {
    let mut errors = Vec::new();
    let mut seen_ids: HashSet<&str> = HashSet::new();

    for (id, t) in &file.targets {
        let mut push = |message: String| errors.push(ValidationIssue::new(id, message));

        // Duplicate check.
        if !seen_ids.insert(id.as_str()) {
            push("duplicate target ID".to_string());
        }
        if id_ends_in_zero_dotted_segment(id) {
            push(
                "dotted target IDs whose final segment is zero are disallowed \
                 because humans conflate T4 and T4.0"
                    .to_string(),
            );
        }

        // An achieved target whose declared blockers are still open is
        // a ledger that contradicts itself: `depends_on` asserts they
        // must be achieved before work here begins, so the attestation
        // claims work the graph says could not have started. Nothing
        // enforced this on the write path until 🎯T79, so existing
        // ledgers carry violations that are otherwise invisible —
        // reported here so they can be found rather than waited for.
        if t.status == Status::Achieved {
            let open: Vec<&str> = t
                .depends_on
                .iter()
                .filter(|d| {
                    file.targets
                        .get(*d)
                        .is_some_and(|dep| !dep.status.is_terminal())
                })
                .map(String::as_str)
                .collect();
            if !open.is_empty() {
                push(format!(
                    "achieved while depending on open target(s): {}",
                    open.join(", ")
                ));
            }
        }

        // Value/cost: 0.0 means "not set at repo scope" (portfolio-scope
        // metadata is optional). Only reject explicitly negative values,
        // which are always a mistake, and non-zero sub-1 values that
        // would produce meaningless WSJF ratios.
        if t.value < 0.0 {
            push(format!("value must be non-negative, got {}", t.value));
        }
        if t.cost < 0.0 {
            push(format!("cost must be non-negative, got {}", t.cost));
        }

        // Acceptance must be non-empty.
        if t.acceptance.is_empty() {
            push("acceptance criteria must not be empty".to_string());
        }

        // Depends-on references must exist.
        for dep in &t.depends_on {
            if !file.targets.contains_key(dep) {
                push(format!("depends_on target {dep} does not exist"));
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
                push("cross-repo edge has empty repo".to_string());
            }
            if edge.target.as_deref().unwrap_or("").is_empty()
                && edge.capability.as_deref().unwrap_or("").is_empty()
            {
                push(format!(
                    "cross-repo edge to {} must set `target` or `capability`",
                    edge.repo,
                ));
            }
        }

        // Status-scoped fields (🎯T64). One loop over
        // `schema::STATUS_SCOPED_FIELDS` covers every field whose
        // presence depends on status — set_aside_reason, attestation,
        // achieved, owned_by, and the postpone pair. The same table
        // drives `Target::clear_illegal_status_scoped_fields`, so what
        // validation rejects here is exactly what a status transition
        // clears; neither side can grow a field the other forgets.
        for field in t.illegal_status_scoped_fields() {
            push(format!(
                "{} is only valid on {} (status is {:?})",
                field.name, field.legal_where, t.status,
            ));
        }

        // Set-aside disposition requires a non-empty rationale (🎯T18).
        // The rationale is the load-bearing artefact: it carries the
        // parked / deferred / wont_fix nuance that the schema deliberately
        // doesn't taxonomise. Empty or whitespace-only reasons are
        // rejected. (The converse — a reason on a non-set-aside status —
        // is covered by the status-scoped loop above.)
        if t.status == Status::SetAside
            && t.set_aside_reason
                .as_deref()
                .is_none_or(|r| r.trim().is_empty())
        {
            push("status set_aside requires a non-empty set_aside_reason".to_string());
        }

        // Achieve attestation (🎯T58): soft words-in-a-box on retirement.
        // Required by the achieve API path; legacy achieved targets may
        // lack the field. Empty / whitespace-only values are invalid when
        // present.
        if t.attestation
            .as_deref()
            .is_some_and(|a| a.trim().is_empty())
        {
            push("attestation must be non-empty when present".to_string());
        }

        // Ownership exclusion (🎯T43): both owner and reason must be
        // non-empty when the field is present.
        if let Some(ob) = &t.owned_by {
            if ob.owner.trim().is_empty() {
                push("owned_by.owner must be non-empty".to_string());
            }
            if ob.reason.trim().is_empty() {
                push("owned_by.reason must be non-empty".to_string());
            }
        }

        // 🎯T39.1: an active parent must list every live dotted child
        // in depends_on. Dotted IDs are a family, not a display prefix;
        // expand:children walks the prefix for rendering only.
        if !t.status.is_terminal() {
            for child in ops::direct_dotted_children(file, id) {
                if !t.depends_on.iter().any(|d| d == child) {
                    push(format!(
                        "dotted child {child} is not in depends_on — a dotted family is an \
                         umbrella (🎯T39.1); the parent cannot retire until every direct child \
                         does. Add {child} to depends_on (child_of and split add/aggregate do \
                         this automatically)"
                    ));
                }
            }
        }

        // Strategy validation: command and trigger must be non-empty.
        if let Some(ref strategy) = t.strategy {
            if strategy.command.trim().is_empty() {
                push("strategy.command must not be empty".to_string());
            }
            if strategy.trigger.trim().is_empty() {
                push("strategy.trigger must not be empty".to_string());
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
        errors: &mut Vec<ValidationIssue>,
    ) {
        if permanent.contains(id) {
            return;
        }
        if !temporary.insert(id) {
            errors.push(ValidationIssue::new(id, "cycle in depends_on graph"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::test_support::tgt;
    use crate::graph::{frontier, summary};
    use crate::schema::TargetsFile;
    use chrono::NaiveDate;
    use std::collections::BTreeMap;

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
