// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::process::Command;

use bullseye::handler::handle_query;
use bullseye::tools::QueryTool;

fn mcp_text(cwd: &str, full: bool) -> String {
    let response = handle_query(QueryTool {
        cwd: cwd.into(),
        view: Some("summary".into()),
        id: None,
        filter: None,
        recent_days: None,
        momentum: None,
        frontier_details: Some(full),
        scope: None,
        nodes: None,
        seeds: None,
        expand: None,
    })
    .unwrap();
    format!("{response:?}")
}

#[test]
fn query_cli_and_mcp_select_same_bounded_and_full_projection() {
    let cwd = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    for (full, flag) in [(false, None), (true, Some("--frontier-details"))] {
        let mut args = vec!["query", "--view", "summary", "--cwd", cwd];
        if let Some(flag) = flag {
            args.push(flag);
        }
        let output = Command::new(env!("CARGO_BIN_EXE_bullseye"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let cli = String::from_utf8(output.stdout).unwrap();
        let mcp = mcp_text(cwd, full);
        let marker = if full {
            "## Active targets by group"
        } else {
            "# Summary (bounded)"
        };
        assert!(cli.contains(marker) && mcp.contains(marker));
        assert_eq!(cli.contains("## Active targets by group"), full);
        assert!(mcp.contains("🎯T1"));
    }
}
