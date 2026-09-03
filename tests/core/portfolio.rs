// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn portfolio_discovers_fixture() {
    use bullseye::portfolio;

    let fixture = fixture_path();
    let scan = portfolio::discover_repos(&fixture, 3, &[]);
    // The fixture has a bullseye.yaml at its root.
    assert_eq!(scan.repos.len(), 1);
    assert_eq!(scan.repos[0].active, 4);
    assert!(scan.repos[0].frontier > 0);
    assert_eq!(scan.repos[0].achieved, 1);
    // Fixture is clean — no warnings.
    assert!(scan.warnings.is_empty());
}

#[test]
fn portfolio_format_includes_frontier_targets() {
    use bullseye::portfolio;

    let fixture = fixture_path();
    let scan = portfolio::discover_repos(&fixture, 3, &[]);
    let out = portfolio::format_portfolio(&scan);
    assert!(out.contains("## Ready for work"));
    assert!(out.contains("🎯T1"));
}

#[test]
fn portfolio_reports_version_mismatch_as_warning() {
    use bullseye::portfolio::{self, RepoWarningKind};
    use std::io::Write;

    // A repo whose bullseye.yaml declares a newer schema_version than
    // this bullseye supports must appear as a warning in the scan
    // — NOT silently disappear from the repos list. This is the
    // whole reason the schema_version check exists: if portfolio
    // swallows the error, an outdated bullseye would hide the
    // "upgrade me" signal across every repo the user scans.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("org").join("future-repo");
    std::fs::create_dir_all(&repo).unwrap();
    let path = repo.join("bullseye.yaml");
    write!(
        std::fs::File::create(&path).unwrap(),
        "schema_version: 999\ntargets:\n  T1:\n    name: From the future\n    \
         status: identified\n    value: 3\n    cost: 2\n    acceptance:\n      \
         - ok\n    discovered: 2026-04-01\n"
    )
    .unwrap();

    let scan = portfolio::discover_repos(tmp.path(), 5, &[]);
    assert!(
        scan.repos.is_empty(),
        "broken repo should not appear in repos list"
    );
    assert_eq!(scan.warnings.len(), 1, "expected one warning");
    assert_eq!(scan.warnings[0].kind, RepoWarningKind::VersionMismatch);
    assert!(scan.warnings[0].message.contains("999"));

    // And the formatted output surfaces it prominently.
    let out = portfolio::format_portfolio(&scan);
    assert!(out.contains("## ⚠ Warnings"));
    assert!(out.contains("Schema version mismatch"));
    assert!(out.contains("upgrade bullseye"));
}

// --- Cross-repo edge tests (🎯T2.2) ---

#[test]
fn cross_repo_edges_yaml_roundtrip() {
    use bullseye::schema::CrossEdge;
    use std::io::Write;

    // A targets file with both cross_depends and cross_enables fields
    // must parse cleanly, survive a save/load roundtrip, and preserve
    // the edges field-for-field. This is the bedrock behaviour the
    // whole T2.2 feature sits on — if serde doesn't handle the shape,
    // nothing else works.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    let yaml = r#"
schema_version: 1
targets:
  T1:
    name: Has cross-repo edges
    status: identified
    value: 5
    cost: 3
    acceptance:
      - done
    cross_depends:
      - repo: marcelocantos/jevon
        capability: Manager API
        note: summarizer lifecycle
    cross_enables:
      - repo: marcelocantos/targets
        target: T1.4
    discovered: 2026-04-07
"#;
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();

    // Load and check the parsed edges.
    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.cross_depends.len(), 1);
    assert_eq!(t1.cross_enables.len(), 1);

    let dep = &t1.cross_depends[0];
    assert_eq!(dep.repo, "marcelocantos/jevon");
    assert_eq!(dep.capability.as_deref(), Some("Manager API"));
    assert_eq!(dep.target, None);
    assert_eq!(dep.note.as_deref(), Some("summarizer lifecycle"));

    let en = &t1.cross_enables[0];
    assert_eq!(en.repo, "marcelocantos/targets");
    assert_eq!(en.target.as_deref(), Some("T1.4"));
    assert_eq!(en.capability, None);
    assert_eq!(en.note, None);

    // Round-trip via save + reload: edges must survive unchanged.
    store::save(&path, &file).unwrap();
    let reloaded = store::load(&path).unwrap();
    assert_eq!(reloaded.targets["T1"].cross_depends, t1.cross_depends);
    assert_eq!(reloaded.targets["T1"].cross_enables, t1.cross_enables);

    // Also check the serialized form directly — omitted fields
    // (target, capability, note) must not appear when they're None.
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("cross_depends:"));
    assert!(raw.contains("cross_enables:"));
    assert!(raw.contains("Manager API"));
    assert!(raw.contains("T1.4"));

    // And an empty cross_depends/cross_enables vec must not serialize
    // at all (skip_serializing_if = "Vec::is_empty").
    let _unused = CrossEdge {
        repo: "x".into(),
        target: None,
        capability: None,
        note: None,
    };
}

#[test]
fn cross_repo_edge_validation_rejects_empty_ref() {
    use bullseye::schema::CrossEdge;

    // An edge with no `target` and no `capability` is structurally
    // meaningless — there's nothing for the portfolio view to render
    // or for the agent to act on. Validation must flag it.
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().cross_depends = vec![CrossEdge {
        repo: "marcelocantos/other".to_string(),
        target: None,
        capability: None,
        note: None,
    }];

    let errors = graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T1") && e.contains("must set `target` or `capability`")),
        "expected cross-repo edge validation error; got: {errors:?}"
    );
}

#[test]
fn cross_repo_edge_validation_rejects_empty_repo() {
    use bullseye::schema::CrossEdge;

    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().cross_enables = vec![CrossEdge {
        repo: "   ".to_string(),
        target: Some("T1".to_string()),
        capability: None,
        note: None,
    }];

    let errors = graph::validate(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T1") && e.contains("empty repo")),
        "expected empty-repo error; got: {errors:?}"
    );
}

#[test]
fn cross_repo_edges_do_not_block_frontier() {
    use bullseye::schema::CrossEdge;

    // Cross-repo dependencies are advisory only — they must not
    // remove a target from the frontier. Otherwise bullseye would
    // be making authoritative claims about the state of another
    // repo's graph, which it intentionally does not track.
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().cross_depends = vec![CrossEdge {
        repo: "marcelocantos/jevon".to_string(),
        capability: Some("missing capability".to_string()),
        target: None,
        note: None,
    }];

    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    assert!(
        ids.contains(&"T1"),
        "T1 should remain in frontier despite cross_depends; got: {ids:?}"
    );
}

#[test]
fn portfolio_surfaces_cross_repo_edges_from_loaded_yaml() {
    use bullseye::portfolio;
    use std::io::Write;

    // End-to-end: write a repo with cross-repo edges, run
    // discover_repos, check the scan captures the edges and
    // format_portfolio surfaces them.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("org").join("linker");
    std::fs::create_dir_all(&repo).unwrap();
    let path = repo.join("bullseye.yaml");

    let yaml = r#"
schema_version: 1
targets:
  T1:
    name: Local work with cross-repo enabler
    status: identified
    value: 3
    cost: 2
    acceptance:
      - done
    cross_enables:
      - repo: marcelocantos/targets
        target: T1.4
        note: unblocks target-aware compaction
    discovered: 2026-04-07
  T2:
    name: Plain higher-value work
    status: identified
    value: 8
    cost: 3
    acceptance:
      - done
    discovered: 2026-04-07
"#;
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();

    let scan = portfolio::discover_repos(tmp.path(), 5, &[]);
    assert_eq!(scan.repos.len(), 1);
    let r = &scan.repos[0];

    // Edges were captured on the summary.
    assert!(r.cross_depends.is_empty());
    assert_eq!(r.cross_enables.len(), 1);
    assert_eq!(r.cross_enables[0].source_target, "T1");
    assert_eq!(r.cross_enables[0].edge.repo, "marcelocantos/targets");
    assert_eq!(r.cross_enables[0].edge.target.as_deref(), Some("T1.4"));

    // Priority boost: T1 (cross-enabler, v=3) ranks above T2 (plain, v=8).
    let ids: Vec<&str> = r.frontier_targets.iter().map(|ft| ft.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["T1", "T2"],
        "cross-enabler T1 should sort above plain T2 despite lower value"
    );

    // And format_portfolio surfaces both the edge and the boost.
    let out = portfolio::format_portfolio(&scan);
    assert!(out.contains("## Cross-repo edges"));
    assert!(out.contains("🎯T1 enables 🎯T1.4 @ marcelocantos/targets"));
    assert!(out.contains("unblocks target-aware compaction"));
    assert!(out.contains("★ 🎯T1"));
}

// --- Startup context tests ---
