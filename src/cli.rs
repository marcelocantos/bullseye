// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! CLI ⇄ MCP surface-parity map (🎯T65).
//!
//! `AGENTS.md` declares surface parity as a project invariant: "Every
//! capability must be reachable from **both** surfaces — MCP tools and
//! CLI — sharing one entry point." The tool list splits three ways:
//! tools with their own subcommand, legacy shims that reach the CLI
//! through a core subcommand, and — the failure this module exists to
//! make impossible — tools with no CLI route at all.
//!
//! [`CLI_ROUTES`] is the machine-readable statement of which case each
//! tool is in. It is consumed by `tests/cli_parity_test.rs`, which
//! asserts (a) the table covers the MCP tool list exactly, so a new tool
//! added to `tools.rs` with no entry here fails the build, and (b) every
//! subcommand named here is one the shipped binary actually accepts.
//! A comment could not do either job.

/// How an MCP tool reaches the CLI surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliRoute {
    /// The tool has its own subcommand: `bullseye <subcommand> …`.
    Direct { subcommand: &'static str },

    /// The tool is a shim onto a core subcommand, selected by a
    /// discriminating flag value: `bullseye query --view frontier`.
    Shim {
        subcommand: &'static str,
        flag: &'static str,
        value: &'static str,
    },

    /// The tool is a shim whose entire surface is one core subcommand,
    /// with no discriminating flag (`bullseye_verify` ≡
    /// `bullseye plan-checks`).
    Alias { subcommand: &'static str },
}

impl CliRoute {
    /// The subcommand this route lands on — the token that must exist in
    /// the binary's dispatch and in `--help`.
    pub fn subcommand(&self) -> &'static str {
        match self {
            CliRoute::Direct { subcommand }
            | CliRoute::Shim { subcommand, .. }
            | CliRoute::Alias { subcommand } => subcommand,
        }
    }

    /// A human-readable invocation, for help text and failure messages.
    pub fn invocation(&self) -> String {
        match self {
            CliRoute::Direct { subcommand } | CliRoute::Alias { subcommand } => {
                format!("bullseye {subcommand}")
            }
            CliRoute::Shim {
                subcommand,
                flag,
                value,
            } => format!("bullseye {subcommand} {flag} {value}"),
        }
    }
}

/// One MCP tool and the CLI route that reaches its entry point.
#[derive(Debug, Clone, Copy)]
pub struct ToolRoute {
    /// MCP tool name, exactly as declared in `tools.rs`.
    pub tool: &'static str,
    pub route: CliRoute,
}

const fn direct(tool: &'static str, subcommand: &'static str) -> ToolRoute {
    ToolRoute {
        tool,
        route: CliRoute::Direct { subcommand },
    }
}

const fn shim(
    tool: &'static str,
    subcommand: &'static str,
    flag: &'static str,
    value: &'static str,
) -> ToolRoute {
    ToolRoute {
        tool,
        route: CliRoute::Shim {
            subcommand,
            flag,
            value,
        },
    }
}

const fn alias(tool: &'static str, subcommand: &'static str) -> ToolRoute {
    ToolRoute {
        tool,
        route: CliRoute::Alias { subcommand },
    }
}

/// Every MCP tool, and how it reaches the CLI.
///
/// Adding a tool to `tools.rs` without adding it here fails
/// `tests/cli_parity_test.rs`. Naming a subcommand here that the binary
/// does not accept fails the same test.
pub const CLI_ROUTES: &[ToolRoute] = &[
    // Core (🎯T45) — one subcommand each.
    direct("bullseye_open", "open"),
    direct("bullseye_query", "query"),
    direct("bullseye_commit", "commit"),
    direct("bullseye_apply", "apply"),
    direct("bullseye_plan_checks", "plan-checks"),
    // Read shims onto `query`.
    shim("bullseye_startup_context", "query", "--view", "context"),
    shim("bullseye_frontier", "query", "--view", "frontier"),
    shim("bullseye_get", "query", "--view", "target"),
    shim("bullseye_list", "query", "--view", "list"),
    shim("bullseye_summary", "query", "--view", "summary"),
    shim("bullseye_graph", "query", "--view", "graph"),
    shim("bullseye_validate", "query", "--view", "validate"),
    // Mutation shims onto `commit`.
    shim("bullseye_put", "commit", "--op", "track"),
    shim("bullseye_retire", "commit", "--op", "achieve"),
    shim("bullseye_set_aside", "commit", "--op", "defer"),
    shim("bullseye_revert", "commit", "--op", "reopen"),
    shim("bullseye_subdivide", "commit", "--op", "split"),
    // Whole-subcommand shims.
    alias("bullseye_init", "open"),
    alias("bullseye_verify", "plan-checks"),
    // Extended (L2) — one subcommand each.
    direct("bullseye_convergence", "convergence"),
    direct("bullseye_portfolio", "portfolio"),
    direct("bullseye_import", "import"),
    direct("bullseye_resolve", "resolve"),
    direct("bullseye_github_sync", "github"),
    direct("bullseye_sync_priorities", "sync-priorities"),
];

/// The CLI route for `tool`, or `None` when the tool is unreachable from
/// the CLI — the state 🎯T65 exists to keep out of the tree.
pub fn route_for(tool: &str) -> Option<CliRoute> {
    CLI_ROUTES.iter().find(|r| r.tool == tool).map(|r| r.route)
}

/// Distinct subcommands named by the table, in table order.
pub fn subcommands() -> Vec<&'static str> {
    let mut seen = Vec::new();
    for r in CLI_ROUTES {
        let sub = r.route.subcommand();
        if !seen.contains(&sub) {
            seen.push(sub);
        }
    }
    seen
}
