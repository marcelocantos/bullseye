// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

#[test]
fn every_tool_emits_valid_json_schema() {
    // Regression test for the `bullseye_summary.momentum: BTreeMap`
    // incident: the rust-mcp-sdk JsonSchema derive silently fell
    // back to `type: "unknown"` for a field it couldn't schema-ify,
    // and the resulting tools/list response was rejected by the
    // Anthropic API as non-Draft-2020-12-compliant, blocking every
    // turn of every session that had bullseye registered. The bug
    // shipped as far as v0.9.0 before a user hit it.
    //
    // Assert that no tool's input schema contains any forbidden
    // patterns: `type: "unknown"` (the specific fallback), plus
    // empty/null types (also invalid).
    use bullseye::tools::TargetTools;

    let tools = TargetTools::tools();
    assert!(!tools.is_empty(), "expected non-empty tool list");

    for tool in &tools {
        let schema_json =
            serde_json::to_string(&tool.input_schema).expect("input_schema must serialize");

        // Forbidden: `type: "unknown"` anywhere in the schema.
        assert!(
            !schema_json.contains("\"type\":\"unknown\""),
            "tool `{}` emits a schema containing `\"type\":\"unknown\"`, which the \
             Anthropic API rejects: {schema_json}",
            tool.name,
        );
        // Forbidden: `type: null` or `type: ""` (both invalid).
        assert!(
            !schema_json.contains("\"type\":null") && !schema_json.contains("\"type\":\"\""),
            "tool `{}` emits a schema with a null or empty `type`: {schema_json}",
            tool.name,
        );
    }
}

/// 🎯T36: the CLI-only subcommands are also exposed as MCP tools so the
/// agent — which mostly drives the MCP surface — can trigger them.
/// Parity between the CLI and MCP surfaces.
#[test]
fn cli_subcommands_are_exposed_as_mcp_tools() {
    use bullseye::tools::TargetTools;
    let names: Vec<String> = TargetTools::tools()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    for expected in ["bullseye_github_sync", "bullseye_sync_priorities"] {
        assert!(
            names.iter().any(|n| n == expected),
            "MCP tool list must expose {expected} (CLI/MCP parity); got: {names:?}"
        );
    }
}
