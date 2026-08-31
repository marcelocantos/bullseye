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
    // The probe must be able to fail, or it proves nothing. A mistyped
    // subcommand and a mistyped flag are separate diagnoses that send the
    // reader looking in different places (🎯T67), so each must name its own
    // kind and not the other's.
    let (code, out) = run(&["not-a-real-subcommand", "--help"]);
    assert_ne!(code, 0, "unknown subcommand must exit non-zero: {out}");
    assert!(
        out.contains("unknown subcommand: not-a-real-subcommand"),
        "a mistyped subcommand must be reported as an unknown subcommand: {out}"
    );
    assert!(
        !out.contains("unknown flag"),
        "a mistyped subcommand must not be blamed on a flag: {out}"
    );

    let (code, out) = run(&["--not-a-real-flag"]);
    assert_ne!(code, 0, "unknown flag must exit non-zero: {out}");
    assert!(
        out.contains("unknown flag: --not-a-real-flag"),
        "a mistyped flag must still be reported as an unknown flag: {out}"
    );
    assert!(
        !out.contains("unknown subcommand"),
        "a mistyped flag must not be blamed on a subcommand: {out}"
    );

    for sub in subcommands() {
        let (code, out) = run(&[sub, "--help"]);
        assert_eq!(
            code, 0,
            "`bullseye {sub} --help` must succeed — CLI_ROUTES names a \
             subcommand src/main.rs does not dispatch. Output:\n{out}"
        );
        assert!(
            !out.contains("unknown flag") && !out.contains("unknown subcommand"),
            "`bullseye {sub}` fell through to an unrecognised-argument arm:\n{out}"
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

// --- Documented-surface drift (🎯T76) --------------------------------
//
// The audit that produced `apply` found that `commit --help` advertised
// a create-only `track`, while `track` was in fact a full upsert. The
// capability existed and was undiscoverable, so agents hand-edited the
// YAML for fields the help denied them. These tests make that class of
// drift a build failure: the documented field list on each surface is
// checked against the schema, not against a copy of itself.

/// The field names `Fragment` actually accepts, recovered from serde's
/// own unknown-field error so the list is derived from the struct
/// rather than hand-maintained alongside it.
fn fragment_schema_fields() -> Vec<String> {
    let err = serde_yaml_ng::from_str::<bullseye::apply::Fragment>("zzz_not_a_field: 1\n")
        .expect_err("unknown field must be rejected")
        .to_string();
    // "unknown field `zzz_not_a_field`, expected one of `name`, `status`, …"
    err.split('`')
        .skip(1)
        .step_by(2)
        .skip(1)
        .map(str::to_string)
        .collect()
}

#[test]
fn field_help_covers_every_fragment_field() {
    let schema = fragment_schema_fields();
    assert!(
        schema.len() > 5,
        "failed to recover the field list from serde: {schema:?}"
    );
    let documented: Vec<&str> = bullseye::apply::FIELD_HELP.iter().map(|f| f.name).collect();

    // Aliases are alternate spellings of a documented field, not extra
    // surface, so they are allowed to appear in the schema list only.
    const ALIASES: &[&str] = &["set_aside_reason"];

    for field in &schema {
        assert!(
            documented.contains(&field.as_str()) || ALIASES.contains(&field.as_str()),
            "Fragment accepts `{field}` but FIELD_HELP does not document it — \
             the exact drift 🎯T76 exists to prevent"
        );
    }
    for field in &documented {
        assert!(
            schema.contains(&field.to_string()),
            "FIELD_HELP documents `{field}` but Fragment does not accept it"
        );
    }
}

#[test]
fn apply_help_lists_every_documented_field() {
    let (code, help) = run(&["apply", "--help"]);
    assert_eq!(code, 0, "apply --help failed: {help}");
    for f in bullseye::apply::FIELD_HELP {
        assert!(
            help.contains(f.name),
            "`bullseye apply --help` omits field `{}` — no field may be reachable \
             only by reading source. Help was:\n{help}",
            f.name
        );
    }
}

#[test]
fn apply_help_states_every_evidence_obligation() {
    let (_, help) = run(&["apply", "--help"]);
    for ob in bullseye::apply::POLICY {
        assert!(
            help.contains(ob.requires),
            "`apply --help` never mentions `{}`, required by {}",
            ob.requires,
            ob.transition
        );
    }
}

#[test]
fn mcp_apply_description_lists_every_documented_field() {
    let tools = TargetTools::tools();
    let apply = tools
        .iter()
        .find(|t| t.name == "bullseye_apply")
        .expect("bullseye_apply must be registered");
    let description = apply.description.clone().unwrap_or_default();
    for f in bullseye::apply::FIELD_HELP {
        assert!(
            description.contains(f.name),
            "the bullseye_apply tool description omits field `{}` — an MCP agent \
             cannot use what the description does not mention",
            f.name
        );
    }
}

#[test]
fn apply_rejects_an_unrecognised_flag_instead_of_reporting_success() {
    let (code, out) = run(&["apply", "--id", "T1", "--valu", "5"]);
    assert_ne!(code, 0, "an unrecognised flag must fail, got:\n{out}");
    assert!(
        out.contains("unrecognised flag"),
        "expected an unrecognised-flag error, got:\n{out}"
    );
    assert!(
        !out.contains("ok: true"),
        "a rejected call must never report success:\n{out}"
    );
}
