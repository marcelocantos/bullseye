// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn startup_context_shows_frontier_and_counts() {
    let file = load_fixture();
    let ctx = graph::startup_context(&file, "test/bullseye.yaml", 14);

    // Header with counts.
    assert!(ctx.contains("Active: 4 target(s)"));
    assert!(ctx.contains("Frontier:"));

    // Frontier section should include T1 and T2 (unblocked active targets).
    assert!(ctx.contains("## Frontier"));
    assert!(ctx.contains("🎯T1"));
    assert!(ctx.contains("🎯T2"));
}

#[test]
fn startup_context_shows_recently_achieved() {
    let mut file = load_fixture();
    // Set T4's achieved date to today so it appears in recent.
    let today = chrono::Local::now().date_naive();
    file.targets.get_mut("T4").unwrap().achieved = Some(today);

    let ctx = graph::startup_context(&file, "test", 14);
    assert!(ctx.contains("## Recently achieved"));
    assert!(ctx.contains("🎯T4"));
    assert!(ctx.contains("Documentation covers all public APIs"));
}

#[test]
fn startup_context_omits_old_achieved() {
    let file = load_fixture();
    // T4 was achieved on 2026-03-10, which is >14 days ago (test runs after that).
    let ctx = graph::startup_context(&file, "test", 14);
    // Should NOT have a recently achieved section (T4 is too old).
    assert!(!ctx.contains("## Recently achieved"));
}

// --- Summary tests ---

#[test]
fn summary_shows_totals_and_sections() {
    let file = load_fixture();
    let out = graph::summary(&file, "test/bullseye.yaml", None, false);

    // Header with counts.
    assert!(out.contains("Total: 5 target(s)"));
    assert!(out.contains("4 active"));
    assert!(out.contains("1 achieved"));

    // Has key sections. No WSJF ranking section or annotations — at
    // repo scope the frontier itself carries the focus ordering
    // (distance-to-observable, fanout). The banner introduced by
    // 🎯T16 names WSJF once to explicitly disavow it, so we check
    // for the absence of actual WSJF *ranking* signals rather than
    // the word itself.
    assert!(out.contains("## Active targets by group"));
    assert!(out.contains("## Frontier"));
    assert!(!out.contains("## WSJF ranking"));
    assert!(!out.contains("wsjf="));
}

#[test]
fn summary_shows_frontier_targets() {
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);

    // Frontier should include T1, T2, T3 (unblocked).
    let frontier_section = out.split("## Frontier").nth(1).unwrap();
    let frontier_end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..frontier_end];
    assert!(frontier_text.contains("🎯T1"));
    assert!(frontier_text.contains("🎯T2"));
    assert!(frontier_text.contains("🎯T3"));
}

#[test]
fn summary_frontier_section_opens_with_repo_scope_banner() {
    // 🎯T16 (v5): every repo-scope frontier rendering must lead with the
    // banner + legend so agents see the correct ordering framing
    // inline and don't default to WSJF/SAFe reasoning from
    // training-data habit. The banner has to sit inside the
    // `## Frontier` section (not before it) so it survives
    // convergence's summary-body splicing.
    //
    // v5 removed the verify/checkpoint/tunnel apparatus; the banner
    // now describes fanout-only ordering.
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);
    let frontier_section = out.split("## Frontier").nth(1).unwrap();
    let frontier_end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..frontier_end];

    assert!(
        frontier_text.contains("Repo-scope ordering"),
        "banner must name the repo-scope ordering function; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("max unblocking fanout"),
        "banner must describe the primary sort key; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("portfolio-scope"),
        "banner must disavow portfolio-scope framing at repo scope; got:\n{frontier_text}"
    );
    // Legend covers the per-entry annotation shapes used in the
    // rendered frontier.
    assert!(
        frontier_text.contains("`fanout=N`"),
        "legend must define the `fanout=N` annotation; got:\n{frontier_text}"
    );
}

#[test]
fn summary_shows_blocked_targets() {
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);

    // T5 depends on T1+T3 (not achieved), so it's blocked.
    assert!(out.contains("## Blocked targets"));
    assert!(out.contains("🎯T5"));
    assert!(out.contains("blocked by"));
}

#[test]
fn summary_frontier_ordered_by_fanout() {
    // Repo-level ordering (v5/🎯T25): descending unblocking fanout,
    // then ascending target ID.
    //
    // The fixture's frontier is T1, T2, T3. T5 depends on T1 and T3
    // (fanout=1 each). T2 has no dependants (fanout=0). Expected order:
    // T1, T3 (both fanout=1, T1 < T3 by ID), then T2 (fanout=0).
    //
    // Value/cost have no effect on repo-level ordering.
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);

    let frontier_section = out
        .split("## Frontier")
        .nth(1)
        .expect("frontier section exists");
    let end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..end];

    let t1_pos = frontier_text.find("🎯T1").expect("T1 in frontier");
    let t3_pos = frontier_text.find("🎯T3").expect("T3 in frontier");
    let t2_pos = frontier_text.find("🎯T2").expect("T2 in frontier");

    assert!(
        t1_pos < t3_pos,
        "T1 (fanout=1, id=T1) should rank above T3 (fanout=1, id=T3); got: {frontier_text}"
    );
    assert!(
        t3_pos < t2_pos,
        "T3 (fanout=1) should rank above T2 (fanout=0); got: {frontier_text}"
    );

    // Annotation format exposes only fanout, not dist/value/focus/momentum.
    assert!(frontier_text.contains("fanout="));
    assert!(!frontier_text.contains("dist="));
    assert!(!frontier_text.contains("v=8"));
    assert!(!frontier_text.contains("focus"));
    assert!(!frontier_text.contains("momentum"));
}

// --- Convergence integration tests ---

#[test]
fn summary_momentum_does_not_affect_repo_level_ordering() {
    use std::collections::BTreeMap;

    // 🎯T7 removed momentum (and value/cost) from repo-level
    // frontier ordering. The parameter is still accepted on the
    // wire for backward compatibility, but it must not perturb the
    // order — repo scope is driven purely by distance-to-observable
    // and unblocking fanout. Momentum lives at the portfolio layer
    // now (`src/portfolio.rs`), not here.
    let file = load_fixture();
    let mut momentum = BTreeMap::new();
    // Boost T2 (the dirty tunnel) with an absurd multiplier. In the
    // old value × momentum formula this would catapult T2 to the
    // top. Under repo-level ordering it must stay dead last — its
    // distance to an observable is `None`.
    momentum.insert("T2".to_string(), 100.0);
    momentum.insert("T1".to_string(), 0.01);

    let with = graph::summary(&file, "test", Some(&momentum), false);
    let without = graph::summary(&file, "test", None, false);

    let section = |s: &str| -> String {
        let start = s.split("## Frontier").nth(1).unwrap();
        let end = start.find("\n## ").unwrap_or(start.len());
        start[..end].to_string()
    };

    assert_eq!(
        section(&with),
        section(&without),
        "momentum map must not change repo-level frontier ordering"
    );
    // Deliberately absent: any WSJF ranking signal, focus label, or
    // momentum annotation in the repo-scope output. The 🎯T16 banner
    // legitimately names WSJF once in order to disavow it, so we
    // check for the ranking-annotation pattern (`wsjf=`) rather than
    // the word.
    assert!(!with.contains("wsjf="));
    assert!(!with.contains("focus"));
    assert!(!with.contains("× momentum"));
}

#[test]
fn summary_stale_parent_all_children_achieved() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    let mut file = load_fixture();
    let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();

    // Add sub-targets T1.1 and T1.2, both achieved.
    for sub in ["T1.1", "T1.2"] {
        file.targets.insert(
            sub.to_string(),
            Target {
                name: format!("Sub {sub}"),
                status: Status::Achieved,
                value: 2.0,
                cost: 1.0,
                actual_cost: None,
                attestation: None,
                set_aside_reason: None,
                acceptance: vec!["done".to_string()],
                checks: vec![],
                context: String::new(),
                gates: vec![],
                depends_on: vec![],
                cross_depends: vec![],
                cross_enables: vec![],
                tags: vec![],
                strategy: None,

                origin: "test".to_string(),
                discovered: date,
                achieved: Some(date),
                owned_by: None,
                postponed_until: None,
                postpone_predicate: None,
            },
        );
    }
    file.targets.get_mut("T1").unwrap().depends_on = vec!["T1.1".to_string(), "T1.2".to_string()];

    // T1 is converging but both children are achieved — stale.
    let out = graph::summary(&file, "test", None, false);
    assert!(out.contains("## Stale targets"));
    assert!(out.contains("all sub-targets achieved"));
}

#[test]
fn summary_shows_grouped_children() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    let mut file = load_fixture();
    let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();

    // Add sub-target T1.1 (active).
    file.targets.insert(
        "T1.1".to_string(),
        Target {
            name: "Sub-target of T1".to_string(),
            status: Status::Identified,
            value: 2.0,
            cost: 1.0,
            actual_cost: None,
            attestation: None,
            set_aside_reason: None,
            acceptance: vec!["done".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: vec![],
            cross_depends: vec![],
            cross_enables: vec![],
            tags: vec![],
            strategy: None,

            origin: "test".to_string(),
            discovered: date,
            achieved: None,
            owned_by: None,
            postponed_until: None,
            postpone_predicate: None,
        },
    );
    file.targets.get_mut("T1").unwrap().depends_on = vec!["T1.1".to_string()];

    let out = graph::summary(&file, "test", None, false);
    // T1 should show a rollup count.
    assert!(out.contains("achieved)"));
    // T1.1 should appear indented under T1.
    assert!(out.contains("🎯T1.1"));
}

/// 🎯T64: a validation error reports the offending target and keeps
/// answering. Before T64 this suppressed the frontier and blocked
/// sections entirely, which is how one stale field in the jevons ledger
/// left its PO with no readable frontier at all.
#[test]
fn summary_with_validation_errors_still_renders_frontier() {
    let mut file = load_fixture();
    // Create a dangling depends_on reference.
    file.targets
        .get_mut("T1")
        .unwrap()
        .depends_on
        .push("T99".to_string());

    let out = graph::summary(&file, "test", None, false);
    assert!(
        out.contains("## Validation errors (degraded read)"),
        "{out}"
    );
    assert!(out.contains("T99"), "{out}");
    // The rest of the graph still answers.
    assert!(out.contains("## Frontier"), "{out}");
    // T2 is unaffected by T1's dangling edge and remains readable.
    assert!(out.contains("🎯T2"), "{out}");
}

#[test]
fn summary_with_only_warnings_still_renders_frontier() {
    // Advisory warnings (e.g. non-conforming target IDs) must not strand
    // the frontier section. graph::summary gates on validate_blocking,
    // not the warning-inclusive validate, so a malformed-ID complaint
    // doesn't suppress the unblocked-targets list. See `validate_warnings`
    // doc comment and convergence.rs's separate "## Validation warnings"
    // rendering.
    let mut file = load_fixture();
    let target = file.targets.get("T1").unwrap().clone();
    file.targets.insert("Bogus".to_string(), target);

    let out = graph::summary(&file, "test", None, false);
    assert!(
        out.contains("## Frontier"),
        "warning-only validation should not suppress ## Frontier:\n{out}"
    );
    assert!(
        !out.contains("## Validation errors"),
        "warning-only validation should not produce ## Validation errors:\n{out}"
    );
}

#[test]
fn startup_context_no_file_is_graceful() {
    // A repo with no bullseye.yaml must not make startup_context fail
    // outright — the session-start hook that typically invokes it runs
    // before the agent knows whether the repo uses bullseye. Return a
    // friendly "not using bullseye yet" message instead.
    let tmp = tempfile::tempdir().unwrap();
    // Sanity check: discover returns None on a fresh empty dir.
    assert!(store::discover(tmp.path()).is_none());

    let out = graph::startup_context_no_file(&tmp.path().display().to_string());
    assert!(out.contains("# Startup context"));
    assert!(out.contains("no bullseye.yaml found"));
    assert!(out.contains("bullseye_init"));
    // Must not look like an error string — agents should be able to
    // keep going.
    assert!(!out.to_lowercase().contains("error"));
}

#[test]
fn startup_context_broken_file_is_graceful() {
    // The helper that formats the degraded response for a broken
    // bullseye.yaml must surface the error without looking like a
    // tool-call failure — session start should continue.
    let out = graph::startup_context_broken_file(
        "/tmp/fake/bullseye.yaml",
        "failed to parse /tmp/fake/bullseye.yaml: invalid YAML at line 4",
    );
    assert!(out.contains("# Startup context"));
    assert!(out.contains("/tmp/fake/bullseye.yaml"));
    assert!(out.contains("could not be loaded"));
    assert!(out.contains("invalid YAML at line 4"));
    assert!(out.contains("Session start is continuing"));
}

#[test]
fn startup_context_shows_validation_errors() {
    let mut file = load_fixture();
    // Create a dangling depends_on reference.
    file.targets
        .get_mut("T1")
        .unwrap()
        .depends_on
        .push("T99".to_string());

    let ctx = graph::startup_context(&file, "test", 14);
    assert!(ctx.contains("Validation errors"));
    assert!(ctx.contains("T99"));
}

// --- executable acceptance checks (🎯T1.1) --------------------------------

/// The summary header reports set-aside targets as a distinct count
/// (not lumped into achieved), and the `## Set aside` section lists
/// each target with its reason. See 🎯T18.
#[test]
fn summary_shows_set_aside_group_and_count() {
    use bullseye::config::{self, Location};
    use bullseye::graph;
    use bullseye::handler::handle_set_aside;
    use bullseye::schema::Status;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "summary-set-aside").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Set T1 aside.
    let reason = "out of scope for this cycle";
    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: reason.to_string(),
    })
    .unwrap();

    let file = store::load(&path).unwrap();
    let out = graph::summary(&file, "test/bullseye.yaml", None, false);

    // Header must name the set-aside count explicitly.
    assert!(
        out.contains("1 set aside"),
        "summary header must include set-aside count; output:\n{out}"
    );
    // Set-aside must NOT inflate the achieved count.
    assert!(
        !out.contains("1 achieved") || file.achieved().is_empty(),
        "set-aside must not inflate achieved count; output:\n{out}"
    );
    // A dedicated ## Set aside section must appear with the reason.
    assert!(
        out.contains("## Set aside"),
        "summary must have ## Set aside section; output:\n{out}"
    );
    assert!(
        out.contains(reason),
        "summary must include the set-aside reason; output:\n{out}"
    );
    // T1 must not appear in active targets.
    let file2 = store::load(&path).unwrap();
    assert_eq!(file2.targets["T1"].status, Status::SetAside);
    assert!(
        !file2.active().contains_key("T1"),
        "set-aside target must not appear in active()"
    );

    config::set_external_root_override(None);
}

/// `bullseye_list` with filter `"set_aside"` returns only set-aside
/// targets and shows the reason inline. See 🎯T18.
#[test]
fn list_set_aside_filter_returns_set_aside_targets() {
    use bullseye::config::{self, Location};
    use bullseye::handler::{handle_list, handle_set_aside};
    use bullseye::tools::{ListTool, SetAsideTool};

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "list-set-aside").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let reason = "won't fix — design changed";
    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: reason.to_string(),
    })
    .unwrap();

    let result = handle_list(ListTool {
        cwd: cwd.clone(),
        filter: "set_aside".to_string(),
    });
    assert!(result.is_ok(), "set_aside filter must succeed: {result:?}");

    let content = text_from_call_result(result.unwrap());

    assert!(
        content.contains("T1"),
        "set_aside list must include T1; content:\n{content}"
    );
    assert!(
        content.contains(reason),
        "set_aside list must show the reason; content:\n{content}"
    );
    // Active targets should not appear in set_aside filter.
    let file = store::load(&path).unwrap();
    for (id, t) in &file.targets {
        if t.status != bullseye::schema::Status::SetAside {
            assert!(
                !content.contains(&format!("🎯{id} ")),
                "active target {id} must not appear in set_aside filter; content:\n{content}"
            );
        }
    }

    config::set_external_root_override(None);
}

// --- 🎯T58: achieve requires free-text attestation ---
