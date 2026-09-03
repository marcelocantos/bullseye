// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Frontier computation, blocking validation, Mermaid export, and
//! agent-facing text rendering over a loaded [`crate::schema::TargetsFile`].
//!
//! Split by role, not by layer (entropy audit ENT-002, 🎯T74.2):
//! [`frontier`] computes the parallelisable work set, [`validate`] is the
//! single blocking oracle plus advisory hygiene warnings, [`mermaid`]
//! exports dependency diagrams, and [`render`] turns those into the
//! markdown text agents read (`bullseye_startup_context`,
//! `bullseye_summary`). Every public name below re-exports from its
//! owning submodule so existing callers (`api`, `handler`, `convergence`,
//! `portfolio`) keep resolving `graph::frontier`, `graph::validate_blocking`,
//! `graph::mermaid`, etc. unchanged.

mod frontier;
mod mermaid;
mod render;
mod validate;

pub use frontier::{
    FrontierTarget, REPO_SCOPE_BANNER, RankedFrontier, TolerantFrontier, frontier, frontier_on,
    frontier_tolerant, is_postponed, owned_elsewhere, rank_frontier, unblocking_fanout,
};
pub use mermaid::{
    MermaidExpand, MermaidOpts, MermaidScope, mermaid, mermaid_with_opts, select_mermaid_nodes,
};
pub use render::{
    degraded_read_banner, startup_context, startup_context_broken_file, startup_context_no_file,
    summary,
};
pub use validate::{
    ValidationIssue, graph_hygiene_warnings, open_blockers, validate, validate_blocking,
    validate_issues, validate_warnings,
};

/// Shared test-only target builder used by unit tests across the
/// `frontier` / `validate` / `render` submodules.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::schema::{Status, Target};
    use chrono::NaiveDate;

    pub fn tgt(name: &str, deps: &[&str]) -> Target {
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
}
