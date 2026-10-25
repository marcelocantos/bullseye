//! 🎯T106 — repair dedented `context: |` literal lines.

use std::path::PathBuf;

use bullseye::context_indent_repair::{
    ContextIndentRepairRequest, prepare_repaired_ledger, sha256_hex,
};
use bullseye::handler::context_indent_repair_request;
use bullseye::store::load;

const BROKEN: &str = r#"# bullseye.yaml — convergence targets for this project.
# content_hash: sha256:deadbeef

schema_version: 3
targets:
  T177:
    name: specimen readable after repair
    status: identified
    value: 0.0
    cost: 0.0
    acceptance:
    - ok
    discovered: 2026-10-25
    context: |
      leading context line
claudia-po / root: **WITHHOLD achieve** @ tip
      trailing context line
"#;

fn write_broken_fixture() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("bullseye.yaml");
    std::fs::write(&path, BROKEN).expect("write");
    (tmp, path)
}

#[test]
fn repair_context_indent_cli_path_repairs_fixture_and_query_succeeds() {
    let (_tmp, path) = write_broken_fixture();
    let cwd = path.parent().expect("parent").to_string_lossy().to_string();
    let raw = std::fs::read_to_string(&path).expect("read");
    let pre = sha256_hex(raw.as_bytes());

    let result = context_indent_repair_request(
        &cwd,
        ContextIndentRepairRequest {
            pre_repair_sha256: pre,
            reason: "T106 hermetic fixture".into(),
            dry_run: false,
        },
    )
    .expect("repair");
    use rust_mcp_sdk::schema::ContentBlock;
    let body = result
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::TextContent(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(body.contains("Repaired context literal"), "{body}");

    let file = load(&path).expect("ledger must parse after repair");
    let t177 = file.targets.get("T177").expect("T177");
    assert!(t177.context.contains("WITHHOLD achieve"));
    assert!(t177.context.contains("leading context"));
    assert_eq!(t177.name, "specimen readable after repair");
}

#[test]
fn prepare_repaired_ledger_matches_claudia_t226_line_class() {
    let snippet = "    context: |\n      Companion line one\nclaudia-po / root 2026-10-25: **WITHHOLD achieve** @ private tip\n      Companion line two\n";
    let wrapped = format!(
        "schema_version: 3\ntargets:\n  T226:\n    name: n\n    status: identified\n    value: 0.0\n    cost: 0.0\n    acceptance:\n    - ok\n    discovered: 2026-10-25\n{snippet}"
    );
    let hash = sha256_hex(wrapped.as_bytes());
    let (fixed, lines, _) = prepare_repaired_ledger(&wrapped, &hash).expect("repair");
    assert_eq!(lines, vec![13]);
    assert!(fixed.contains("      claudia-po / root"));
}
