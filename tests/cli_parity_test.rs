// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! CLI ⇄ MCP surface-parity oracle (🎯T65).
//!
//! `AGENTS.md`: "Every capability must be reachable from **both**
//! surfaces — MCP tools and CLI — sharing one entry point." These tests
//! turn that sentence into a gate. The load-bearing property is that a
//! new MCP tool added to `tools.rs` with no CLI route **fails**, rather
//! than passing by silent omission:
//!
//! 1. [`every_mcp_tool_has_a_cli_route`] — the route table covers the
//!    tool list exactly, in both directions.
//! 2. [`every_routed_subcommand_is_accepted_by_the_binary`] — every
//!    subcommand the table names is one the shipped binary dispatches,
//!    proven by running it. A companion assertion shows the probe
//!    discriminates: a bogus subcommand is rejected.
//! 3. [`every_routed_subcommand_appears_in_help`] — the subcommands are
//!    discoverable from `bullseye --help`.
//! 4. [`every_shim_value_is_documented_by_its_subcommand`] — a shim's
//!    discriminating flag value (`--view frontier`, `--op track`) is
//!    still one the core subcommand advertises.

use std::process::Command;

use bullseye::cli::{CLI_ROUTES, CliRoute, route_for, subcommands};
use bullseye::tools::TargetTools;

const BIN: &str = env!("CARGO_BIN_EXE_bullseye");

/// Run the binary and return (exit code, stdout+stderr).
fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(BIN)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {BIN} {args:?}: {e}"));
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

fn mcp_tool_names() -> Vec<String> {
    TargetTools::tools()
        .iter()
        .map(|t| t.name.to_string())
        .collect()
}

/// The invariant itself: the route table and the MCP tool list describe
/// the same set. An unrouted tool fails here — the failure mode 🎯T65
/// exists to prevent.
#[test]
fn every_mcp_tool_has_a_cli_route() {
    let tools = mcp_tool_names();
    assert!(!tools.is_empty(), "MCP tool list must not be empty");

    let unrouted: Vec<&String> = tools
        .iter()
        .filter(|name| route_for(name).is_none())
        .collect();
    assert!(
        unrouted.is_empty(),
        "MCP tools with no CLI route (AGENTS.md surface parity): {unrouted:?}\n\
         Add a subcommand in src/main.rs sharing the tool's handler, then an \
         entry in src/cli.rs CLI_ROUTES — or, if it reaches the CLI through a \
         core subcommand, record that as a Shim/Alias route."
    );

    // The other direction: a stale entry would let a deleted tool keep a
    // route, and a typo'd `tool` field would silently satisfy nothing.
    let stale: Vec<&str> = CLI_ROUTES
        .iter()
        .map(|r| r.tool)
        .filter(|t| !tools.iter().any(|name| name == t))
        .collect();
    assert!(
        stale.is_empty(),
        "CLI_ROUTES names tools that no longer exist in tools.rs: {stale:?}"
    );

    let mut seen: Vec<&str> = Vec::new();
    for r in CLI_ROUTES {
        assert!(
            !seen.contains(&r.tool),
            "CLI_ROUTES has a duplicate entry for {}",
            r.tool
        );
        seen.push(r.tool);
    }
}

/// A route is only worth as much as the dispatch behind it: run the
/// shipped binary and require it to accept each routed subcommand.
#[test]
fn every_routed_subcommand_is_accepted_by_the_binary() {
    // The probe must be able to fail, or it proves nothing.
    let (code, out) = run(&["not-a-real-subcommand", "--help"]);
    assert_ne!(code, 0, "unknown subcommand must exit non-zero: {out}");
    assert!(
        out.contains("unknown flag"),
        "unknown subcommand must be reported as unknown: {out}"
    );

    for sub in subcommands() {
        let (code, out) = run(&[sub, "--help"]);
        assert_eq!(
            code, 0,
            "`bullseye {sub} --help` must succeed — CLI_ROUTES names a \
             subcommand src/main.rs does not dispatch. Output:\n{out}"
        );
        assert!(
            !out.contains("unknown flag"),
            "`bullseye {sub}` fell through to the unknown-flag arm:\n{out}"
        );
    }
}

/// Acceptance: each subcommand appears in `print_help()` output.
#[test]
fn every_routed_subcommand_appears_in_help() {
    let (code, help) = run(&["--help"]);
    assert_eq!(code, 0, "`bullseye --help` must succeed");
    for sub in subcommands() {
        assert!(
            help.contains(&format!("bullseye {sub}")),
            "`bullseye --help` does not mention the `{sub}` subcommand:\n{help}"
        );
    }
}

/// A shim route claims a specific flag value on a core subcommand.
/// Renaming a view or an op without updating the shim map is caught
/// here, against the subcommand's own advertised surface.
#[test]
fn every_shim_value_is_documented_by_its_subcommand() {
    for r in CLI_ROUTES {
        let CliRoute::Shim {
            subcommand,
            flag,
            value,
        } = r.route
        else {
            continue;
        };
        let (code, out) = run(&[subcommand, "--help"]);
        assert_eq!(code, 0, "`bullseye {subcommand} --help` must succeed");
        assert!(
            out.contains(flag),
            "{}: `bullseye {subcommand}` help does not document `{flag}`:\n{out}",
            r.tool
        );
        assert!(
            out.contains(value),
            "{}: `bullseye {subcommand} {flag} {value}` — `{value}` is not among \
             the values `bullseye {subcommand} --help` advertises:\n{out}",
            r.tool
        );
    }
}

/// The four L2 tools 🎯T65 was raised for now have real subcommands
/// (not shims), each exiting non-zero on failure so a script can branch.
#[test]
fn l2_subcommands_exist_and_report_failure() {
    for tool in [
        "bullseye_convergence",
        "bullseye_portfolio",
        "bullseye_import",
        "bullseye_resolve",
    ] {
        assert!(
            matches!(route_for(tool), Some(CliRoute::Direct { .. })),
            "{tool} must have its own CLI subcommand, not a shim: {:?}",
            route_for(tool)
        );
    }

    // 🎯T62 drives `bullseye convergence` from a script: a bad run must
    // be distinguishable from a good one by exit status alone.
    let (code, out) = run(&["convergence", "--cwd", "/no/such/directory"]);
    assert_ne!(code, 0, "convergence on a missing cwd must fail: {out}");

    let (code, out) = run(&["resolve", "--reference", "no-such-repo-anywhere-xyzzy"]);
    assert_ne!(
        code, 0,
        "resolve of an unmatched reference must fail: {out}"
    );

    let (code, out) = run(&["import", "--cwd", "/no/such/directory", "--path", "x.md"]);
    assert_ne!(code, 0, "import into a missing cwd must fail: {out}");
}

/// `bullseye convergence` runs end-to-end against a real ledger and
/// shares the MCP handler's output — the unblocker for 🎯T62, which
/// needs to profile this from the shell rather than through a 120s MCP
/// budget.
#[test]
fn convergence_subcommand_runs_against_a_real_ledger() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("bullseye.yaml"),
        "schema_version: 5\ntargets:\n  T1:\n    name: ship it\n    status: identified\n    \
         value: 1.0\n    cost: 1.0\n    discovered: 2026-08-10\n    \
         acceptance:\n      - it ships\n",
    )
    .unwrap();

    let (code, out) = run(&[
        "convergence",
        "--cwd",
        tmp.path().to_str().unwrap(),
        "--skip-invariants",
    ]);
    assert_eq!(
        code, 0,
        "convergence must succeed on a valid ledger:\n{out}"
    );
    assert!(
        out.contains("🎯T1"),
        "convergence must report targets:\n{out}"
    );
    assert!(
        out.contains("skip_invariants=true"),
        "--skip-invariants must reach the shared handler:\n{out}"
    );
}
