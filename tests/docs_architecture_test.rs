// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! ENT-004: historical architecture proposals must not read as shipped behavior.

use std::fs;
use std::path::Path;

fn design_warning_present(text: &str) -> bool {
    let Some((banner, _)) = text.split_once("## Motivation") else {
        return false;
    };
    banner.contains("superseded")
        && banner.contains("not implemented")
        && banner.contains("schema v5")
        && banner.contains("schema v6")
        && banner.contains("](shapes.md)")
        && banner.contains("](api-v1-core.md)")
}

fn triad_warning_present(text: &str) -> bool {
    let Some((_, proposal)) = text.split_once("### 1. Executable acceptance criteria") else {
        return false;
    };
    let Some((local, _)) = proposal.split_once("**Sawmill capabilities today:**") else {
        return false;
    };
    let Some((warning, execution)) = local.split_once("**Execution model:** `targets_verify`")
    else {
        return false;
    };
    warning.len() < 1400
        && warning.contains("superseded")
        && warning.contains("does not call sawmill or execute checks")
        && warning.contains("](shapes.md)")
        && warning.contains("](api-v1-core.md)")
        && execution.contains("iterates a target's")
}

#[test]
fn historical_architecture_warns_at_the_claim() {
    let docs_root = std::env::var("BULLSEYE_DOCS_ARCHITECTURE_DIR")
        .unwrap_or_else(|_| format!("{}/docs", env!("CARGO_MANIFEST_DIR")));
    let docs = Path::new(&docs_root);
    let design = fs::read_to_string(docs.join("design.md")).expect("read design.md");
    let triad = fs::read_to_string(docs.join("mcp-triad.md")).expect("read mcp-triad.md");
    assert!(
        design_warning_present(&design),
        "design banner must precede Motivation and name the shipped model and canonical references"
    );
    assert!(
        triad_warning_present(&triad),
        "triad warning must immediately precede the targets_verify proposal and name plan-only behavior and canonical references"
    );
}

#[test]
fn missing_warnings_are_rejected() {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let design = fs::read_to_string(docs.join("design.md")).expect("read design.md");
    let triad = fs::read_to_string(docs.join("mcp-triad.md")).expect("read mcp-triad.md");
    let without_design_banner = design.replacen(
        &design[design.find("> **Historical design").expect("banner")
            ..design.find("## Motivation").expect("motivation")],
        "",
        1,
    );
    let without_triad_warning = triad.replacen(
        &triad[triad.find("**Historical proposal").expect("warning")
            ..triad
                .find("**Execution model:** `targets_verify`")
                .expect("proposal")],
        "",
        1,
    );
    assert!(!design_warning_present(&without_design_banner));
    assert!(!triad_warning_present(&without_triad_warning));
}
