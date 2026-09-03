// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Frontier computation: which targets are unblocked and ready for work,
//! and repo-level ranking over that set. See `graph/validate.rs` for the
//! blocking oracle and `graph/render.rs` for the text that presents the
//! frontier to agents.

use std::collections::HashSet;

use crate::schema::{Status, TargetsFile};

use super::validate::{ValidationIssue, validate_issues};

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

/// A frontier computed over the healthy part of a graph that may carry
/// blocking validation errors (🎯T64).
#[derive(Debug, Clone)]
pub struct TolerantFrontier {
    /// Ready targets, with any invalid target removed.
    pub targets: Vec<FrontierTarget>,
    /// Every blocking validation error found, in report order.
    pub issues: Vec<ValidationIssue>,
    /// IDs that would have been in the frontier but were dropped
    /// because they carry a validation error. A subset of the IDs named
    /// in `issues`.
    pub excluded: Vec<String>,
}

impl TolerantFrontier {
    /// Whether the graph validated cleanly (nothing was degraded).
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Compute the frontier without letting one bad target hide the rest
/// (🎯T64).
///
/// The read path used to gate on `validate_blocking(file).is_empty()`
/// and return the errors *instead of* an answer. One stale field on one
/// target therefore made an entire ledger unreadable — frontier
/// included — with no supported op able to repair it (the jevons
/// incident, 2026-08-10).
///
/// The gate was always stronger than necessary: [`frontier`] is defined
/// over each target's own status and dependency edges, so an invalid
/// *other* target cannot corrupt it. A target with a dangling
/// `depends_on` stays blocked, a cyclic one never becomes ready, and a
/// target with illegal status-scoped residue is simply excluded here.
/// So we compute the frontier regardless and report the errors
/// alongside it.
///
/// Which surfaces degrade and which hard-fail:
/// - **degrade** — `view=frontier`, `view=context`, `view=summary`,
///   portfolio, convergence: answer over the healthy subgraph, with the
///   errors shown in a banner.
/// - **answer regardless** — `view=list`, `view=target`: per-target
///   reads never consulted validation and still don't; `list` annotates
///   the offending targets so the reader sees them.
/// - **hard-fail** — `view=validate`: reporting these errors *is* its
///   contract. It must never be degraded, or the ledger loses the one
///   surface that tells the truth about its own health.
pub fn frontier_tolerant(file: &TargetsFile) -> TolerantFrontier {
    let issues = validate_issues(file);
    let invalid: HashSet<&str> = issues.iter().map(|i| i.target.as_str()).collect();

    let mut excluded = Vec::new();
    let targets = frontier(file)
        .into_iter()
        .filter(|ft| {
            if invalid.contains(ft.id.as_str()) {
                excluded.push(ft.id.clone());
                false
            } else {
                true
            }
        })
        .collect();

    TolerantFrontier {
        targets,
        issues,
        excluded,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::test_support::tgt;
    use crate::schema::TargetsFile;
    use chrono::NaiveDate;
    use std::collections::BTreeMap;

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
}
