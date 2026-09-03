// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Agent-facing text rendering: startup context and the consolidated
//! `bullseye_summary` report. Both are read-only presentations over
//! [`super::frontier`] and [`super::validate`] — no graph computation
//! or validation logic belongs here.

use std::collections::{BTreeMap, HashSet};

use crate::schema::{Status, TargetsFile};

use super::REPO_SCOPE_BANNER;
use super::frontier::{TolerantFrontier, frontier_tolerant, owned_elsewhere, rank_frontier};
use super::validate::graph_hygiene_warnings;

/// Markdown banner naming the targets a degraded read skipped (🎯T64).
///
/// Empty string when there is nothing to report, so callers can push it
/// unconditionally.
pub fn degraded_read_banner(tf: &TolerantFrontier) -> String {
    if tf.is_clean() {
        return String::new();
    }
    let mut out = String::from(
        "## Validation errors (degraded read)\n\nThese targets are invalid and were skipped; \
         the rest of the graph is reported normally. Run `bullseye_query view=validate` for the \
         full report, and `bullseye_commit op=rehash` to rewrite the file if the errors are \
         stale status-scoped fields.\n\n",
    );
    for issue in &tf.issues {
        out.push_str(&format!("- {issue}\n"));
    }
    if !tf.excluded.is_empty() {
        out.push_str(&format!(
            "\nExcluded from the frontier: {}\n",
            tf.excluded
                .iter()
                .map(|id| format!("🎯{id}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push('\n');
    out
}

/// Produce a concise startup context summary for agent consumption.
pub fn startup_context(file: &TargetsFile, file_path: &str, recent_days: u32) -> String {
    let cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(recent_days as i64);

    let active = file.active();
    let active_count = active.len();

    // 🎯T64: degrade rather than blank the frontier when some target is
    // invalid — the healthy graph is still the agent's work queue.
    let tolerant = frontier_tolerant(file);
    let front = &tolerant.targets;

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
        crate::version::VERSION,
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

    out.push_str(&degraded_read_banner(&tolerant));

    if !front.is_empty() {
        out.push_str("## Frontier (unblocked, ready for work)\n\n");
        for ft in front {
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

    let tolerant = frontier_tolerant(file);
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
    // Descending unblocking fanout, then by ID. See
    // [`crate::graph::rank_frontier`] for the full rule and rationale.
    // Value/cost/momentum intentionally not consumed here — those are
    // portfolio-scope signals.
    // 🎯T64: invalid targets are named and skipped, not used as a
    // reason to withhold the frontier.
    out.push_str(&degraded_read_banner(&tolerant));

    let ranked = rank_frontier(file, &tolerant.targets);

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
