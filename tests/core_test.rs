// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use bullseye::graph;
use bullseye::schema::{RetryPolicy, Status, Strategy, TargetsFile};
use bullseye::store;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_fixture() -> TargetsFile {
    let path = fixture_path().join("bullseye.yaml");
    store::load(&path).unwrap()
}

#[test]
fn loads_and_counts_targets() {
    let file = load_fixture();
    assert_eq!(file.targets.len(), 5);
}

#[test]
fn active_filter() {
    let file = load_fixture();
    let active = file.active();
    assert_eq!(active.len(), 4); // T1, T2, T3, T5 (T4 is achieved)
    assert!(!active.contains_key("T4"));
}

#[test]
fn achieved_filter() {
    let file = load_fixture();
    let achieved = file.achieved();
    assert_eq!(achieved.len(), 1);
    assert!(achieved.contains_key("T4"));
}

#[test]
fn validates_ok() {
    let file = load_fixture();
    let errors = graph::validate(&file);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[test]
fn mermaid_generation() {
    let file = load_fixture();
    let diagram = graph::mermaid(&file);
    assert!(diagram.contains("graph TD"));
    // T5 depends on T1 and T3 — should have "needs" edges.
    assert!(diagram.contains("needs"));
}

#[test]
fn discovers_from_subdirectory() {
    let found = store::discover(&fixture_path());
    assert!(found.is_some());
    assert!(found.unwrap().ends_with("bullseye.yaml"));
}

#[test]
fn detects_cycle_in_depends_on() {
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().depends_on = vec!["T2".to_string()];
    file.targets.get_mut("T2").unwrap().depends_on = vec!["T1".to_string()];
    let errors = graph::validate(&file);
    assert!(errors.iter().any(|e| e.contains("cycle")));
}

#[test]
fn yaml_roundtrip() {
    let file = load_fixture();
    let yaml = serde_yaml_ng::to_string(&file).unwrap();
    let reparsed: TargetsFile = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(file.targets.len(), reparsed.targets.len());
    assert_eq!(file.targets["T1"].status, reparsed.targets["T1"].status);
}

#[test]
fn frontier_returns_unblocked_leaves() {
    let file = load_fixture();
    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    // T1 (converging, no deps), T2 (identified, no deps), T3 (identified, no deps)
    // are all active leaves with no unachieved dependencies.
    // T4 is achieved so excluded. T5 depends on T1+T3 so blocked.
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&"T1"));
    assert!(ids.contains(&"T2"));
    assert!(ids.contains(&"T3"));
    assert!(!ids.contains(&"T5"), "T5 should be blocked by T1 and T3");
}

#[test]
fn frontier_excludes_blocked() {
    let mut file = load_fixture();
    // Make T2 depend on T1 (which is converging, not achieved).
    file.targets.get_mut("T2").unwrap().depends_on = vec!["T1".to_string()];
    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    assert!(!ids.contains(&"T2"), "T2 should be blocked by T1");
    assert!(ids.contains(&"T1"));
    assert!(ids.contains(&"T3"));
}

#[test]
fn load_accepts_legacy_file_without_schema_version() {
    use bullseye::schema::CURRENT_SCHEMA_VERSION;
    use std::io::Write;
    // A bullseye.yaml written before schema_version was introduced must
    // still load cleanly. The loader treats the missing field as the
    // current (v1) schema and fills it in so the next save stamps it.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    let legacy_yaml = r#"
targets:
  T1:
    name: Legacy target
    status: identified
    value: 3
    cost: 2
    acceptance:
      - it works
    discovered: 2026-04-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{legacy_yaml}").unwrap();

    let file = store::load(&path).unwrap();
    assert_eq!(file.schema_version, Some(CURRENT_SCHEMA_VERSION));
    assert_eq!(file.targets.len(), 1);
}

#[test]
fn load_rejects_newer_schema_version_with_upgrade_prompt() {
    use std::io::Write;
    // A bullseye.yaml declaring a schema_version higher than this
    // binary supports must fail fast with a clear upgrade message,
    // not silently drop or misinterpret unknown fields.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    let future_yaml = r#"
schema_version: 999
targets:
  T1:
    name: From the future
    status: identified
    value: 3
    cost: 2
    acceptance:
      - it works
    discovered: 2026-04-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{future_yaml}").unwrap();

    let err = store::load(&path).unwrap_err();
    // Must be the structured VersionTooNew variant so callers can
    // discriminate it from Io/Parse errors. Also check the rendered
    // Display form carries enough detail for a human.
    match &err {
        store::LoadError::VersionTooNew {
            found, supported, ..
        } => {
            assert_eq!(*found, 999);
            assert!(*supported < 999);
        }
        other => panic!("expected VersionTooNew, got {other:?}"),
    }
    let rendered = err.to_string();
    assert!(rendered.contains("schema_version 999"), "got: {rendered}");
    assert!(rendered.contains("Upgrade bullseye"), "got: {rendered}");
}

#[test]
fn save_stamps_current_schema_version() {
    use bullseye::schema::CURRENT_SCHEMA_VERSION;
    use std::io::Write;
    // Loading a legacy file and re-saving must produce a file with
    // the current schema_version on disk, so legacy files self-upgrade
    // on first contact with a v0.9.0+ bullseye.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    let legacy_yaml = r#"
targets:
  T1:
    name: Legacy target
    status: identified
    value: 3
    cost: 2
    acceptance:
      - it works
    discovered: 2026-04-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{legacy_yaml}").unwrap();

    let file = store::load(&path).unwrap();
    store::save(&path, &file).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains(&format!("schema_version: {CURRENT_SCHEMA_VERSION}")),
        "expected schema_version stamp; got:\n{after}"
    );
}

#[test]
fn load_migrates_legacy_gates_to_depends_on() {
    use std::io::Write;
    // Write a legacy YAML with the old `gates` field and verify that
    // `T2.gates = [T1]` folds into `T2.depends_on += [T1]` — i.e., the
    // owning target absorbs its gates as blockers ("T2 is gated by T1").
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    let legacy_yaml = r#"
targets:
  T1:
    name: Upstream prerequisite
    status: identified
    value: 3
    cost: 2
    acceptance:
      - prerequisite is ready
    discovered: 2026-04-01
  T2:
    name: Downstream work
    status: identified
    value: 3
    cost: 2
    acceptance:
      - it works
    gates:
      - target: T1
        criticality: 0.8
    discovered: 2026-04-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{legacy_yaml}").unwrap();

    let file = store::load(&path).unwrap();
    // T2 should now depend on T1, because T2 was gated by T1.
    assert_eq!(file.targets["T2"].depends_on, vec!["T1"]);
    assert!(file.targets["T1"].depends_on.is_empty());
    // Both targets should have empty gates after migration.
    assert!(file.targets["T1"].gates.is_empty());
    assert!(file.targets["T2"].gates.is_empty());
    // And the frontier reflects the new blocking edge.
    let front = graph::frontier(&file);
    let ids: Vec<&str> = front.iter().map(|f| f.id.as_str()).collect();
    assert!(ids.contains(&"T1"), "T1 is unblocked");
    assert!(!ids.contains(&"T2"), "T2 is blocked by T1");
}

#[test]
fn create_starter_produces_valid_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(
        tmp.path(),
        bullseye::config::Location::InRepo,
        "test-project",
    )
    .unwrap();

    assert!(path.exists());
    assert_eq!(path, tmp.path().join("bullseye.yaml"));

    let file = store::load(&path).unwrap();
    assert_eq!(file.targets.len(), 1);

    let t1 = &file.targets["T1"];
    assert!(t1.name.contains("test-project"));
    assert_eq!(t1.status, Status::Identified);
    assert_eq!(t1.origin, "bullseye_init");
    assert_eq!(t1.acceptance.len(), 2);

    // Validate the file passes all checks.
    let errors = graph::validate(&file);
    assert!(errors.is_empty(), "validation errors: {errors:?}");
}

#[test]
fn create_starter_does_not_overwrite() {
    let tmp = tempfile::tempdir().unwrap();

    // Create the first time.
    let path = store::create_at(tmp.path(), bullseye::config::Location::InRepo, "project").unwrap();
    assert!(path.exists());

    // discover should now find it, so handler-level guard works.
    let found = store::discover(tmp.path());
    assert!(found.is_some());
}

// --- Portfolio tests ---

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

#[test]
fn startup_context_shows_frontier_and_counts() {
    let file = load_fixture();
    let ctx = graph::startup_context(&file, "test/bullseye.yaml", 14);

    // Header with counts.
    assert!(ctx.contains("Active: 4 target(s)"));
    assert!(ctx.contains("Frontier:"));

    // Frontier section should include T1 and T2 (unblocked active targets).
    assert!(ctx.contains("## Frontier"));
    assert!(ctx.contains("🎯T1"));
    assert!(ctx.contains("🎯T2"));
}

#[test]
fn startup_context_shows_recently_achieved() {
    let mut file = load_fixture();
    // Set T4's achieved date to today so it appears in recent.
    let today = chrono::Local::now().date_naive();
    file.targets.get_mut("T4").unwrap().achieved = Some(today);

    let ctx = graph::startup_context(&file, "test", 14);
    assert!(ctx.contains("## Recently achieved"));
    assert!(ctx.contains("🎯T4"));
    assert!(ctx.contains("Documentation covers all public APIs"));
}

#[test]
fn startup_context_omits_old_achieved() {
    let file = load_fixture();
    // T4 was achieved on 2026-03-10, which is >14 days ago (test runs after that).
    let ctx = graph::startup_context(&file, "test", 14);
    // Should NOT have a recently achieved section (T4 is too old).
    assert!(!ctx.contains("## Recently achieved"));
}

// --- Summary tests ---

#[test]
fn summary_shows_totals_and_sections() {
    let file = load_fixture();
    let out = graph::summary(&file, "test/bullseye.yaml", None, false);

    // Header with counts.
    assert!(out.contains("Total: 5 target(s)"));
    assert!(out.contains("4 active"));
    assert!(out.contains("1 achieved"));

    // Has key sections. No WSJF ranking section or annotations — at
    // repo scope the frontier itself carries the focus ordering
    // (distance-to-observable, fanout). The banner introduced by
    // 🎯T16 names WSJF once to explicitly disavow it, so we check
    // for the absence of actual WSJF *ranking* signals rather than
    // the word itself.
    assert!(out.contains("## Active targets by group"));
    assert!(out.contains("## Frontier"));
    assert!(!out.contains("## WSJF ranking"));
    assert!(!out.contains("wsjf="));
}

#[test]
fn summary_shows_frontier_targets() {
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);

    // Frontier should include T1, T2, T3 (unblocked).
    let frontier_section = out.split("## Frontier").nth(1).unwrap();
    let frontier_end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..frontier_end];
    assert!(frontier_text.contains("🎯T1"));
    assert!(frontier_text.contains("🎯T2"));
    assert!(frontier_text.contains("🎯T3"));
}

#[test]
fn summary_frontier_section_opens_with_repo_scope_banner() {
    // 🎯T16 (v5): every repo-scope frontier rendering must lead with the
    // banner + legend so agents see the correct ordering framing
    // inline and don't default to WSJF/SAFe reasoning from
    // training-data habit. The banner has to sit inside the
    // `## Frontier` section (not before it) so it survives
    // convergence's summary-body splicing.
    //
    // v5 removed the verify/checkpoint/tunnel apparatus; the banner
    // now describes fanout-only ordering.
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);
    let frontier_section = out.split("## Frontier").nth(1).unwrap();
    let frontier_end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..frontier_end];

    assert!(
        frontier_text.contains("Repo-scope ordering"),
        "banner must name the repo-scope ordering function; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("max unblocking fanout"),
        "banner must describe the primary sort key; got:\n{frontier_text}"
    );
    assert!(
        frontier_text.contains("portfolio-scope"),
        "banner must disavow portfolio-scope framing at repo scope; got:\n{frontier_text}"
    );
    // Legend covers the per-entry annotation shapes used in the
    // rendered frontier.
    assert!(
        frontier_text.contains("`fanout=N`"),
        "legend must define the `fanout=N` annotation; got:\n{frontier_text}"
    );
}

#[test]
fn summary_shows_blocked_targets() {
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);

    // T5 depends on T1+T3 (not achieved), so it's blocked.
    assert!(out.contains("## Blocked targets"));
    assert!(out.contains("🎯T5"));
    assert!(out.contains("blocked by"));
}

#[test]
fn summary_frontier_ordered_by_fanout() {
    // Repo-level ordering (v5/🎯T25): descending unblocking fanout,
    // then ascending target ID.
    //
    // The fixture's frontier is T1, T2, T3. T5 depends on T1 and T3
    // (fanout=1 each). T2 has no dependants (fanout=0). Expected order:
    // T1, T3 (both fanout=1, T1 < T3 by ID), then T2 (fanout=0).
    //
    // Value/cost have no effect on repo-level ordering.
    let file = load_fixture();
    let out = graph::summary(&file, "test", None, false);

    let frontier_section = out
        .split("## Frontier")
        .nth(1)
        .expect("frontier section exists");
    let end = frontier_section
        .find("\n## ")
        .unwrap_or(frontier_section.len());
    let frontier_text = &frontier_section[..end];

    let t1_pos = frontier_text.find("🎯T1").expect("T1 in frontier");
    let t3_pos = frontier_text.find("🎯T3").expect("T3 in frontier");
    let t2_pos = frontier_text.find("🎯T2").expect("T2 in frontier");

    assert!(
        t1_pos < t3_pos,
        "T1 (fanout=1, id=T1) should rank above T3 (fanout=1, id=T3); got: {frontier_text}"
    );
    assert!(
        t3_pos < t2_pos,
        "T3 (fanout=1) should rank above T2 (fanout=0); got: {frontier_text}"
    );

    // Annotation format exposes only fanout, not dist/value/focus/momentum.
    assert!(frontier_text.contains("fanout="));
    assert!(!frontier_text.contains("dist="));
    assert!(!frontier_text.contains("v=8"));
    assert!(!frontier_text.contains("focus"));
    assert!(!frontier_text.contains("momentum"));
}

// --- Convergence integration tests ---

fn write_project(tmp: &std::path::Path, makefile: &str, targets_yaml: &str) {
    use std::io::Write;
    write!(
        std::fs::File::create(tmp.join("bullseye.yaml")).unwrap(),
        "{targets_yaml}"
    )
    .unwrap();
    write!(
        std::fs::File::create(tmp.join("Makefile")).unwrap(),
        "{makefile}"
    )
    .unwrap();
}

/// Extract the concatenated text payload from an MCP `CallToolResult`,
/// panicking if any content block is not a `TextContent`. Used by the
/// handler-level end-to-end tests that drive `handle_convergence`
/// directly rather than calling `convergence::convergence`.
fn text_from_call_result(result: rust_mcp_sdk::schema::CallToolResult) -> String {
    use rust_mcp_sdk::schema::ContentBlock;
    result
        .content
        .into_iter()
        .map(|block| match block {
            ContentBlock::TextContent(t) => t.text,
            other => panic!("expected TextContent, got {other:?}"),
        })
        .collect::<Vec<_>>()
        .join("")
}

const SIMPLE_TARGETS_YAML: &str = r#"
schema_version: 5
targets:
  T1:
    name: Primary deliverable
    status: identified
    value: 8
    cost: 3
    acceptance:
      - Produces the primary artifact
      - Tests cover the happy path
    context: The highest-value thing in the project.
    discovered: 2026-04-01
  T1.v:
    name: Verify primary deliverable
    status: identified
    value: 1
    cost: 1
    acceptance:
      - T1 passes
    depends_on:
      - T1
    discovered: 2026-04-01
  T2:
    name: Secondary polish
    status: identified
    value: 3
    cost: 2
    acceptance:
      - Rough edges smoothed
    discovered: 2026-04-01
"#;

#[test]
fn convergence_end_to_end_green_invariants_picks_top_frontier() {
    // Full integration: real temp project, real Makefile that exits 0,
    // real bullseye.yaml, real convergence pipeline. Verifies the whole
    // path from hook invocation to recommendation text.
    let tmp = tempfile::tempdir().unwrap();
    // `true` is a trivial program that exits 0 — standing invariants green.
    let makefile = "bullseye:\n\t@true\n";
    write_project(tmp.path(), makefile, SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    assert!(out.contains("# Convergence"));
    assert!(out.contains("## Invariants"));
    assert!(
        out.contains("Status: ✅ all green"),
        "expected green status; got:\n{out}"
    );
    assert!(out.contains("## Unreleased fixes"));
    // No git history in the temp dir → no tag → no unreleased fixes reported.
    assert!(out.contains("(none"));
    assert!(out.contains("## Frontier"));
    // Frontier should include both targets with full details inline.
    assert!(out.contains("🎯T1 Primary deliverable"));
    assert!(out.contains("🎯T2 Secondary polish"));
    assert!(
        out.contains("Produces the primary artifact"),
        "frontier details should include acceptance criteria; got:\n{out}"
    );
    assert!(
        out.contains("The highest-value thing in the project."),
        "frontier details should include context; got:\n{out}"
    );
    assert!(out.contains("## Next action"));
    assert!(
        out.contains("**Execute now**: Work on 🎯T1 Primary deliverable"),
        "expected top-focus target as next action; got:\n{out}"
    );
    // No WSJF ranking annotations anywhere in the convergence output.
    // (The 🎯T16 banner names WSJF once to explicitly disavow it at
    // repo scope — absence of `wsjf=` covers the actual anti-pattern
    // of scored WSJF entries without colliding with the disavowal.)
    assert!(!out.to_lowercase().contains("wsjf="));
    // 🎯T16: repo-scope banner must survive the summary-body splice
    // into convergence output.
    assert!(
        out.contains("Repo-scope ordering"),
        "convergence output must carry the repo-scope banner; got:\n{out}"
    );
}

#[test]
fn convergence_end_to_end_red_invariants_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    // `false` exits 1 — standing invariants red.
    let makefile = "bullseye:\n\t@echo 'tests failing'; false\n";
    write_project(tmp.path(), makefile, SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    assert!(out.contains("Status: ❌ failed"));
    assert!(out.contains("tests failing"));
    let next = out
        .split("## Next action")
        .nth(1)
        .expect("next action section");
    assert!(
        next.contains("**Blocked**"),
        "blocked recommendation expected; got:\n{next}"
    );
    assert!(next.contains("invariants failing"));
    // Crucially: the Execute now path must NOT fire when invariants fail,
    // even though there are perfectly good frontier targets.
    assert!(!next.contains("**Execute now**"));
}

#[test]
fn convergence_end_to_end_skip_invariants_flag_bypasses_hook() {
    let tmp = tempfile::tempdir().unwrap();
    // Red Makefile — but we're skipping, so it should never run.
    let makefile = "bullseye:\n\t@echo 'would have failed'; false\n";
    write_project(tmp.path(), makefile, SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, true);

    assert!(out.contains("(skipped"));
    assert!(!out.contains("would have failed"));
    assert!(!out.contains("Status: ❌"));
    // With invariants skipped and no unreleased fixes, we should go
    // straight to the frontier-based recommendation.
    let next = out
        .split("## Next action")
        .nth(1)
        .expect("next action section");
    assert!(
        next.contains("**Execute now**: Work on 🎯T1"),
        "expected top-focus target as next action; got:\n{next}"
    );
}

#[test]
fn convergence_missing_makefile_degrades_gracefully() {
    // A repo with bullseye.yaml but no Makefile. Convergence must
    // still run to completion — emit the target snapshot, mark
    // invariants as unknown with setup instructions embedded, and
    // still produce a frontier recommendation.
    let tmp = tempfile::tempdir().unwrap();
    use std::io::Write;
    write!(
        std::fs::File::create(tmp.path().join("bullseye.yaml")).unwrap(),
        "{SIMPLE_TARGETS_YAML}"
    )
    .unwrap();
    // Note: NO Makefile.

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    // Full convergence shape is present.
    assert!(out.contains("# Convergence"));
    assert!(out.contains("## Invariants"));
    assert!(out.contains("## Unreleased fixes"));
    assert!(out.contains("## Frontier"));
    assert!(out.contains("## Next action"));

    // Invariants section includes the setup warning inline.
    let invariants_section = out
        .split("## Invariants")
        .nth(1)
        .expect("invariants section");
    let end = invariants_section
        .find("\n## ")
        .unwrap_or(invariants_section.len());
    let invariants_text = &invariants_section[..end];
    assert!(invariants_text.contains("⚠"));
    assert!(invariants_text.contains("not configured"));
    assert!(invariants_text.contains("Makefile"));
    assert!(invariants_text.contains("bullseye:"));
    assert!(invariants_text.contains("unknown"));

    // Target snapshot still renders — the frontier has details.
    assert!(out.contains("🎯T1 Primary deliverable"));
    assert!(out.contains("Produces the primary artifact"));

    // Next action still fires — frontier recommendation — with a
    // prominent note that invariants are unknown.
    let next = out.split("## Next action").nth(1).expect("next action");
    assert!(
        next.contains("**Execute now**: Work on 🎯T1"),
        "frontier recommendation should still fire when hook is missing; got:\n{next}"
    );
    assert!(
        next.contains("standing invariants are **unknown**"),
        "should warn that invariants are unknown; got:\n{next}"
    );
}

#[test]
fn convergence_makefile_without_bullseye_rule_degrades_gracefully() {
    // Same shape as the no-Makefile case, but with a Makefile that
    // exists but has no `bullseye` target. The setup warning should
    // identify the specific build file so the fix is obvious.
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path(), "all:\n\t@echo hello\n", SIMPLE_TARGETS_YAML);

    let path = tmp.path().join("bullseye.yaml");
    let file = store::load(&path).unwrap();
    let out = bullseye::convergence::convergence(&file, &path, tmp.path(), None, false);

    let invariants_section = out
        .split("## Invariants")
        .nth(1)
        .expect("invariants section");
    let end = invariants_section
        .find("\n## ")
        .unwrap_or(invariants_section.len());
    let invariants_text = &invariants_section[..end];
    assert!(invariants_text.contains("found `Makefile`"));
    assert!(invariants_text.contains("no `bullseye` target"));

    // Frontier recommendation still fires.
    let next = out.split("## Next action").nth(1).expect("next action");
    assert!(next.contains("**Execute now**: Work on 🎯T1"));
}

#[test]
fn handle_convergence_resolves_repo_root() {
    // Regression guard for a user-reported bug: `handle_convergence`
    // used to compute the repo root by stepping up two parent
    // directories unconditionally, landing in the grandparent. No
    // Makefile was found there, so invariant detection fell through to
    // "hook not configured" even though the repo had a perfectly good
    // `bullseye:` rule at the real root.
    //
    // Every other convergence end-to-end test in this file calls
    // `bullseye::convergence::convergence(...)` directly, passing
    // `repo_root` explicitly — which bypasses the path-computation
    // layer that contained the bug. This test drives
    // `handle_convergence` as a full integration so any future
    // inversion of `repo_root_from_targets_path` or
    // `store::discover`'s candidate order is caught at the handler
    // boundary.
    use bullseye::config;
    use bullseye::handler::handle_convergence;
    use bullseye::tools::ConvergenceTool;

    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path(), "bullseye:\n\t@true\n", SIMPLE_TARGETS_YAML);

    // Isolate the external shadow root so discover_anywhere can't pick
    // up state from the developer's real ~/.local/share/bullseye.
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            bullseye::config::set_external_root_override(None);
        }
    }
    let _cleanup = Cleanup;

    let result = handle_convergence(ConvergenceTool {
        cwd: tmp.path().to_string_lossy().into_owned(),
        momentum: None,
        skip_invariants: None,
    })
    .expect("handle_convergence should succeed with a valid project");
    let out = text_from_call_result(result);

    // Headline assertion: the invariants hook must have been found and
    // run. If the repo root was computed incorrectly, this would
    // instead report "not configured" + a setup warning, and the
    // status would be "unknown".
    assert!(
        out.contains("Status: ✅ all green"),
        "expected green invariants status — this is the regression guard for the \
         root-level bullseye.yaml bug; if this fails, handle_convergence is \
         computing repo_root incorrectly. Output:\n{out}"
    );

    // Mirror the canonical `convergence_end_to_end_green_invariants_picks_top_frontier`
    // assertions so this test also covers the rest of the pipeline,
    // not just the repo-root fix.
    assert!(out.contains("# Convergence"));
    assert!(out.contains("## Invariants"));
    assert!(out.contains("## Frontier"));
    assert!(out.contains("🎯T1 Primary deliverable"));
    assert!(
        out.contains("**Execute now**: Work on 🎯T1 Primary deliverable"),
        "expected top-focus target as next action; got:\n{out}"
    );

    // Negative: no stray "not configured" text anywhere — this is the
    // exact phrase the buggy path produced, and it must not appear.
    assert!(
        !out.contains("not configured"),
        "convergence should not report the hook as missing when it is \
         present at the repo root; got:\n{out}"
    );
}

#[test]
fn convergence_unreleased_fixes_detected_in_git_repo() {
    // Initialise a real git repo in a temp dir, tag it, then add a
    // "Fix ..." commit so convergence sees an unreleased fix.
    use std::process::Command;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path();
    write_project(path, "bullseye:\n\t@true\n", SIMPLE_TARGETS_YAML);

    // Minimal git init + config + tag + commit sequence.
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr),
        );
    };
    git(&["init", "-q", "-b", "master"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "test"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "Initial"]);
    git(&["tag", "v0.1.0"]);

    // Touch a file and make a fix commit.
    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "-q", "-m", "Fix missing README for v0.1.0"]);

    let yaml_path = path.join("bullseye.yaml");
    let file = store::load(&yaml_path).unwrap();
    let out = bullseye::convergence::convergence(&file, &yaml_path, path, None, false);

    let unreleased_section = out
        .split("## Unreleased fixes")
        .nth(1)
        .expect("unreleased section exists");
    assert!(
        unreleased_section.contains("Fix missing README"),
        "expected fix commit in unreleased section; got:\n{unreleased_section}"
    );
    // Priority 2: unreleased fixes take precedence — expect /release as next action.
    let next = out
        .split("## Next action")
        .nth(1)
        .expect("next action section");
    assert!(
        next.contains("**Execute now**: Run `/release`"),
        "expected /release next action; got:\n{next}"
    );
}

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

#[test]
fn summary_momentum_does_not_affect_repo_level_ordering() {
    use std::collections::BTreeMap;

    // 🎯T7 removed momentum (and value/cost) from repo-level
    // frontier ordering. The parameter is still accepted on the
    // wire for backward compatibility, but it must not perturb the
    // order — repo scope is driven purely by distance-to-observable
    // and unblocking fanout. Momentum lives at the portfolio layer
    // now (`src/portfolio.rs`), not here.
    let file = load_fixture();
    let mut momentum = BTreeMap::new();
    // Boost T2 (the dirty tunnel) with an absurd multiplier. In the
    // old value × momentum formula this would catapult T2 to the
    // top. Under repo-level ordering it must stay dead last — its
    // distance to an observable is `None`.
    momentum.insert("T2".to_string(), 100.0);
    momentum.insert("T1".to_string(), 0.01);

    let with = graph::summary(&file, "test", Some(&momentum), false);
    let without = graph::summary(&file, "test", None, false);

    let section = |s: &str| -> String {
        let start = s.split("## Frontier").nth(1).unwrap();
        let end = start.find("\n## ").unwrap_or(start.len());
        start[..end].to_string()
    };

    assert_eq!(
        section(&with),
        section(&without),
        "momentum map must not change repo-level frontier ordering"
    );
    // Deliberately absent: any WSJF ranking signal, focus label, or
    // momentum annotation in the repo-scope output. The 🎯T16 banner
    // legitimately names WSJF once in order to disavow it, so we
    // check for the ranking-annotation pattern (`wsjf=`) rather than
    // the word.
    assert!(!with.contains("wsjf="));
    assert!(!with.contains("focus"));
    assert!(!with.contains("× momentum"));
}

#[test]
fn legacy_showcase_demonstration_keys_load_and_strip_on_save() {
    // 🎯T23 removed the `showcase` and `demonstration` fields from
    // the schema entirely (v3 → v4). Pre-v4 yaml files in the wild
    // still carry these keys; the loader must accept them silently
    // (serde drops unknown fields) and the next save must strip them.
    // This is the migration path — a one-shot, no-op rewrite.
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    // schema_version: 3 mimics a file written by v0.20.0..v0.26.0.
    // Both the new `showcase` field and its legacy `observable` alias
    // must be tolerated, plus the `demonstration` companion.
    let yaml = r#"
schema_version: 3
targets:
  T1:
    name: Legacy work A
    status: achieved
    value: 5
    cost: 3
    showcase: true
    demonstration: ran the binary and shared a screenshot
    acceptance:
      - done
    discovered: 2026-03-01
    achieved: 2026-03-15
  T2:
    name: Legacy work B
    status: identified
    value: 3
    cost: 2
    observable: true
    acceptance:
      - done
    discovered: 2026-03-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();

    // Load must succeed.
    let file = store::load(&path).expect("pre-v4 file must still load");
    assert!(file.targets.contains_key("T1"));
    assert!(file.targets.contains_key("T2"));

    // Save + read raw: the retired showcase/demonstration/observable
    // keys must all be stripped by the round-trip. Match the YAML key
    // shape (`<key>:`) so target names that legitimately mention these
    // words don't false-positive.
    store::save(&path, &file).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        !raw.contains("showcase:"),
        "saved file must not retain the retired `showcase` key; got:\n{raw}"
    );
    assert!(
        !raw.contains("demonstration:"),
        "saved file must not retain the retired `demonstration` key; got:\n{raw}"
    );
    assert!(
        !raw.contains("observable:"),
        "saved file must not retain the retired `observable` alias; got:\n{raw}"
    );
}

// --- Phase-boundary tests (🎯T11): value/cost optional at repo scope ---

/// Creating a repo-scope target without value or cost must succeed.
/// value/cost are portfolio-scope metadata (cross-repo WSJF ranking) and
/// must not be required when working within a single repo.
#[test]
fn put_create_without_value_cost_succeeds() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::store;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "phase-boundary-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    // Cleared at end of test.

    let result = handle_put(PutTool {
        cwd: cwd.clone(),
        id: None,
        name: Some("Repo-scope target with no portfolio metadata".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["CI green".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    assert!(
        result.is_ok(),
        "create without value/cost must succeed: {result:?}"
    );

    // The created target must pass validation.
    let file = store::load(&path).unwrap();
    // Find the newly created target (it will be auto-assigned an ID beyond T1).
    let new_target = file
        .targets
        .values()
        .find(|t| t.name.contains("Repo-scope target"))
        .expect("new target should exist after put");
    assert_eq!(new_target.value, 0.0, "value should default to 0.0");
    assert_eq!(new_target.cost, 0.0, "cost should default to 0.0");

    // Validate: 0.0 value/cost must not produce validation errors.
    let errors = graph::validate(&file);
    let value_cost_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.contains("value") || e.contains("cost"))
        .collect();
    assert!(
        value_cost_errors.is_empty(),
        "0.0 value/cost should not produce validation errors: {value_cost_errors:?}"
    );

    config::set_external_root_override(None);
}

#[test]
fn summary_stale_parent_all_children_achieved() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    let mut file = load_fixture();
    let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();

    // Add sub-targets T1.1 and T1.2, both achieved.
    for sub in ["T1.1", "T1.2"] {
        file.targets.insert(
            sub.to_string(),
            Target {
                name: format!("Sub {sub}"),
                status: Status::Achieved,
                value: 2.0,
                cost: 1.0,
                actual_cost: None,
                set_aside_reason: None,
                acceptance: vec!["done".to_string()],
                checks: vec![],
                context: String::new(),
                gates: vec![],
                depends_on: vec![],
                cross_depends: vec![],
                cross_enables: vec![],
                tags: vec![],
                strategy: None,

                origin: "test".to_string(),
                discovered: date,
                achieved: Some(date),
            },
        );
    }

    // T1 is converging but both children are achieved — stale.
    let out = graph::summary(&file, "test", None, false);
    assert!(out.contains("## Stale targets"));
    assert!(out.contains("all sub-targets achieved"));
}

#[test]
fn summary_shows_grouped_children() {
    use bullseye::schema::Target;
    use chrono::NaiveDate;

    let mut file = load_fixture();
    let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();

    // Add sub-target T1.1 (active).
    file.targets.insert(
        "T1.1".to_string(),
        Target {
            name: "Sub-target of T1".to_string(),
            status: Status::Identified,
            value: 2.0,
            cost: 1.0,
            actual_cost: None,
            set_aside_reason: None,
            acceptance: vec!["done".to_string()],
            checks: vec![],
            context: String::new(),
            gates: vec![],
            depends_on: vec![],
            cross_depends: vec![],
            cross_enables: vec![],
            tags: vec![],
            strategy: None,

            origin: "test".to_string(),
            discovered: date,
            achieved: None,
        },
    );

    let out = graph::summary(&file, "test", None, false);
    // T1 should show a rollup count.
    assert!(out.contains("achieved)"));
    // T1.1 should appear indented under T1.
    assert!(out.contains("🎯T1.1"));
}

#[test]
fn summary_with_validation_errors_skips_frontier() {
    let mut file = load_fixture();
    // Create a dangling depends_on reference.
    file.targets
        .get_mut("T1")
        .unwrap()
        .depends_on
        .push("T99".to_string());

    let out = graph::summary(&file, "test", None, false);
    assert!(out.contains("## Validation errors"));
    assert!(out.contains("T99"));
    // Should NOT have frontier or blocked sections.
    assert!(!out.contains("## Frontier"));
    assert!(!out.contains("## Blocked"));
}

#[test]
fn summary_with_only_warnings_still_renders_frontier() {
    // Advisory warnings (e.g. non-conforming target IDs) must not strand
    // the frontier section. graph::summary gates on validate_blocking,
    // not the warning-inclusive validate, so a malformed-ID complaint
    // doesn't suppress the unblocked-targets list. See `validate_warnings`
    // doc comment and convergence.rs's separate "## Validation warnings"
    // rendering.
    let mut file = load_fixture();
    let target = file.targets.get("T1").unwrap().clone();
    file.targets.insert("Bogus".to_string(), target);

    let out = graph::summary(&file, "test", None, false);
    assert!(
        out.contains("## Frontier"),
        "warning-only validation should not suppress ## Frontier:\n{out}"
    );
    assert!(
        !out.contains("## Validation errors"),
        "warning-only validation should not produce ## Validation errors:\n{out}"
    );
}

#[test]
fn startup_context_no_file_is_graceful() {
    // A repo with no bullseye.yaml must not make startup_context fail
    // outright — the session-start hook that typically invokes it runs
    // before the agent knows whether the repo uses bullseye. Return a
    // friendly "not using bullseye yet" message instead.
    let tmp = tempfile::tempdir().unwrap();
    // Sanity check: discover returns None on a fresh empty dir.
    assert!(store::discover(tmp.path()).is_none());

    let out = graph::startup_context_no_file(&tmp.path().display().to_string());
    assert!(out.contains("# Startup context"));
    assert!(out.contains("no bullseye.yaml found"));
    assert!(out.contains("bullseye_init"));
    // Must not look like an error string — agents should be able to
    // keep going.
    assert!(!out.to_lowercase().contains("error"));
}

#[test]
fn load_parse_error_is_structured() {
    use std::io::Write;
    // A bullseye.yaml that exists but is syntactically broken should
    // return LoadError::Parse — the typed variant lets callers like
    // bullseye_startup_context choose to degrade gracefully instead
    // of surfacing a raw tool-call error.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    // Deliberately malformed: unterminated list + stray colon.
    let broken_yaml = "targets:\n  T1:\n    name: [unterminated\n    status:::\n";
    write!(std::fs::File::create(&path).unwrap(), "{broken_yaml}").unwrap();

    let err = store::load(&path).unwrap_err();
    assert!(
        matches!(err, store::LoadError::Parse(_)),
        "expected Parse, got {err:?}"
    );
}

#[test]
fn startup_context_broken_file_is_graceful() {
    // The helper that formats the degraded response for a broken
    // bullseye.yaml must surface the error without looking like a
    // tool-call failure — session start should continue.
    let out = graph::startup_context_broken_file(
        "/tmp/fake/bullseye.yaml",
        "failed to parse /tmp/fake/bullseye.yaml: invalid YAML at line 4",
    );
    assert!(out.contains("# Startup context"));
    assert!(out.contains("/tmp/fake/bullseye.yaml"));
    assert!(out.contains("could not be loaded"));
    assert!(out.contains("invalid YAML at line 4"));
    assert!(out.contains("Session start is continuing"));
}

#[test]
fn startup_context_shows_validation_errors() {
    let mut file = load_fixture();
    // Create a dangling depends_on reference.
    file.targets
        .get_mut("T1")
        .unwrap()
        .depends_on
        .push("T99".to_string());

    let ctx = graph::startup_context(&file, "test", 14);
    assert!(ctx.contains("Validation errors"));
    assert!(ctx.contains("T99"));
}

// --- executable acceptance checks (🎯T1.1) --------------------------------

#[test]
fn checks_field_yaml_roundtrip() {
    use bullseye::schema::{Check, QueryCheck};

    // Parsing a targets file with every check variant and re-serializing
    // it must preserve shape, field names, and variant discriminators.
    // This is the load/save round-trip guarantee called out in the
    // target description.
    let yaml = r#"
schema_version: 1
targets:
  T1:
    name: Platform code is isolated
    status: identified
    value: 5
    cost: 3
    acceptance:
      - No platform #ifdefs outside src/platform/
    checks:
      - convention: no-platform-ifdefs
      - query:
          kind: preprocessor_directive
          pattern: "ifdef|ifndef|if defined"
          exclude_path: src/platform/
          expect: 0
      - invariant: platform-isolation
    discovered: 2026-04-01
"#;

    let file: TargetsFile = serde_yaml_ng::from_str(yaml).unwrap();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.checks.len(), 3);
    assert_eq!(
        t1.checks[0],
        Check::Convention {
            convention: "no-platform-ifdefs".to_string()
        }
    );
    match &t1.checks[1] {
        Check::Query {
            query:
                QueryCheck {
                    kind,
                    pattern,
                    exclude_path,
                    expect,
                },
        } => {
            assert_eq!(kind, "preprocessor_directive");
            assert_eq!(pattern.as_deref(), Some("ifdef|ifndef|if defined"));
            assert_eq!(exclude_path.as_deref(), Some("src/platform/"));
            assert_eq!(*expect, 0);
        }
        other => panic!("expected Query, got {other:?}"),
    }
    assert_eq!(
        t1.checks[2],
        Check::Invariant {
            invariant: "platform-isolation".to_string()
        }
    );

    // Round-trip through YAML and re-parse — must equal the original
    // in-memory shape.
    let reserialized = serde_yaml_ng::to_string(&file).unwrap();
    let reparsed: TargetsFile = serde_yaml_ng::from_str(&reserialized).unwrap();
    assert_eq!(reparsed.targets["T1"].checks, t1.checks);

    // Convention variant should serialize as a single-key map
    // `- convention: ...`, not `- !Convention ...` or `- {tag: ...}`.
    assert!(
        reserialized.contains("convention: no-platform-ifdefs"),
        "got:\n{reserialized}"
    );
    assert!(
        reserialized.contains("invariant: platform-isolation"),
        "got:\n{reserialized}"
    );
    assert!(reserialized.contains("query:"), "got:\n{reserialized}");
}

#[test]
fn checks_field_survives_store_save_load() {
    use bullseye::schema::{Check, QueryCheck};
    // End-to-end round-trip through the store layer (which adds the
    // schema version stamp, migrations, etc.) to prove `checks`
    // survives a real save/load cycle, not just in-memory serde.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");

    let mut file = load_fixture();
    file.targets.get_mut("T3").unwrap().checks = vec![
        Check::Convention {
            convention: "no-platform-ifdefs".to_string(),
        },
        Check::Query {
            query: QueryCheck {
                kind: "preprocessor_directive".to_string(),
                pattern: None,
                exclude_path: Some("src/platform/".to_string()),
                expect: 0,
            },
        },
    ];

    store::save(&path, &file).unwrap();
    let reloaded = store::load(&path).unwrap();
    assert_eq!(reloaded.targets["T3"].checks, file.targets["T3"].checks);
    // Other targets with no checks stay empty.
    assert!(reloaded.targets["T1"].checks.is_empty());
}

#[test]
fn checks_field_skipped_when_empty() {
    // A target with no checks must not emit an empty `checks: []`
    // key — it should be omitted entirely to avoid cluttering the
    // default YAML view.
    let file = load_fixture();
    let yaml = serde_yaml_ng::to_string(&file).unwrap();
    assert!(
        !yaml.contains("checks:"),
        "fixture targets have no checks; field should be omitted.\n{yaml}"
    );
}

#[test]
fn verify_plan_builds_for_all_variants() {
    use bullseye::ops::{CheckKind, CheckOutcome, CheckSpec, SawmillTool, verify_plan};
    use bullseye::schema::{Check, QueryCheck};

    let mut file = load_fixture();
    let t3 = file.targets.get_mut("T3").unwrap();
    t3.checks = vec![
        Check::Convention {
            convention: "no-platform-ifdefs".to_string(),
        },
        Check::Query {
            query: QueryCheck {
                kind: "preprocessor_directive".to_string(),
                pattern: Some("ifdef|ifndef|if defined".to_string()),
                exclude_path: Some("src/platform/".to_string()),
                expect: 0,
            },
        },
        Check::Invariant {
            invariant: "platform-isolation".to_string(),
        },
    ];

    let plan = verify_plan(&file, "T3").unwrap();
    assert_eq!(plan.target_id, "T3");
    assert_eq!(plan.checks.len(), 3);

    // Each planned check is routed to the right sawmill tool.
    assert_eq!(plan.checks[0].tool, SawmillTool::CheckConventions);
    assert_eq!(plan.checks[1].tool, SawmillTool::Query);
    assert_eq!(plan.checks[2].tool, SawmillTool::CheckInvariants);

    // And each carries structured args the agent can feed to sawmill.
    match &plan.checks[0].spec {
        CheckSpec::Convention { convention } => {
            assert_eq!(convention, "no-platform-ifdefs");
        }
        other => panic!("expected Convention, got {other:?}"),
    }
    match &plan.checks[1].spec {
        CheckSpec::Query { query: q } => {
            assert_eq!(q.kind, "preprocessor_directive");
            assert_eq!(q.expect, 0);
        }
        other => panic!("expected Query, got {other:?}"),
    }
    match &plan.checks[2].spec {
        CheckSpec::Invariant { invariant } => {
            assert_eq!(invariant, "platform-isolation");
        }
        other => panic!("expected Invariant, got {other:?}"),
    }

    // Report template starts pending with one entry per planned check.
    assert_eq!(plan.report_template.target, "T3");
    assert_eq!(plan.report_template.overall, CheckOutcome::Pending);
    assert_eq!(plan.report_template.checks.len(), 3);
    assert_eq!(plan.report_template.checks[0].kind, CheckKind::Convention);
    assert_eq!(plan.report_template.checks[1].kind, CheckKind::Query);
    assert_eq!(plan.report_template.checks[2].kind, CheckKind::Invariant);
    for entry in &plan.report_template.checks {
        assert_eq!(entry.outcome, CheckOutcome::Pending);
        assert!(entry.failures.is_empty());
    }
}

#[test]
fn verify_plan_errors_for_missing_target() {
    use bullseye::ops::{VerifyError, verify_plan};

    let file = load_fixture();
    let err = verify_plan(&file, "T99").unwrap_err();
    assert_eq!(err, VerifyError::TargetNotFound("T99".to_string()));
}

#[test]
fn verify_plan_errors_when_no_checks_defined() {
    use bullseye::ops::{VerifyError, verify_plan};

    let file = load_fixture();
    // Fixture T1 has no checks — verify_plan should refuse with a
    // structured error rather than returning an empty plan (callers
    // need to distinguish "no work to plan" from "plan is ready").
    let err = verify_plan(&file, "T1").unwrap_err();
    assert_eq!(err, VerifyError::NoChecks("T1".to_string()));
}

#[test]
fn verify_report_structure_serializes_file_line_detail() {
    use bullseye::ops::{CheckFailure, CheckKind, CheckOutcome, CheckResult, VerifyReport};

    // The report type is what the agent populates after running
    // sawmill. Make sure file/line-level detail round-trips through
    // serde so the agent can feed reports back into tooling.
    let report = VerifyReport {
        target: "T3".to_string(),
        overall: CheckOutcome::Fail,
        checks: vec![CheckResult {
            index: 0,
            kind: CheckKind::Convention,
            outcome: CheckOutcome::Fail,
            failures: vec![CheckFailure {
                file: Some("src/foo.c".to_string()),
                line: Some(42),
                message: "platform #ifdef outside src/platform/".to_string(),
            }],
        }],
    };

    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"overall\":\"fail\""));
    assert!(json.contains("\"file\":\"src/foo.c\""));
    assert!(json.contains("\"line\":42"));

    let reparsed: VerifyReport = serde_json::from_str(&json).unwrap();
    assert_eq!(reparsed, report);
}

// ---------------------------------------------------------------------
// Per-repo discovery integration tests (v0.16.0+).
// ---------------------------------------------------------------------

/// RAII helper: isolate the external shadow root to a tempdir so the
/// tests don't touch the developer's real `~/.local/share/bullseye`,
/// and cleanly restore on drop.
struct ShadowFixture {
    _tmp: tempfile::TempDir,
}

impl ShadowFixture {
    fn with_root(root: &std::path::Path) -> Self {
        bullseye::config::set_external_root_override(Some(root.to_path_buf()));
        // Caller owns the tempdir; this holder just flips the override back.
        ShadowFixture {
            _tmp: tempfile::tempdir().unwrap(),
        }
    }
}

impl Drop for ShadowFixture {
    fn drop(&mut self) {
        bullseye::config::set_external_root_override(None);
    }
}

#[test]
fn missing_targets_file_surfaces_location_prompt() {
    use bullseye::handler::handle_list;
    use bullseye::tools::ListTool;

    // Isolate the shadow root so discover_anywhere's external probe
    // can't accidentally hit an unrelated file.
    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let err = handle_list(ListTool {
        cwd: work.path().to_string_lossy().into_owned(),
        filter: "active".to_string(),
    })
    .expect_err("missing targets file must surface as error");
    let msg = format!("{err:?}");

    // The error names where we looked and carries the init prompt.
    assert!(
        msg.contains("no bullseye.yaml found"),
        "not-found preamble missing: {msg}"
    );
    assert!(
        msg.contains("Create bullseye.yaml for this repo where?"),
        "location prompt missing: {msg}"
    );
    assert!(msg.contains("in_repo"), "in_repo choice missing: {msg}");
    assert!(msg.contains("external"), "external choice missing: {msg}");
    assert!(
        msg.contains("bullseye_init"),
        "call-to-action missing: {msg}"
    );
}

#[test]
fn init_without_location_returns_prompt() {
    use bullseye::handler::handle_init;
    use bullseye::tools::InitTool;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let err = handle_init(InitTool {
        cwd: work.path().to_string_lossy().into_owned(),
        location: String::new(), // empty → unknown → prompt
        project_name: None,
    })
    .expect_err("empty location must surface as error");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("unknown location"),
        "parse error missing: {msg}"
    );
    assert!(
        msg.contains("Create bullseye.yaml for this repo where?"),
        "location prompt missing: {msg}"
    );
}

#[test]
fn init_in_repo_creates_file_in_cwd() {
    use bullseye::handler::{handle_init, handle_list};
    use bullseye::tools::{InitTool, ListTool};

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    handle_init(InitTool {
        cwd: cwd.clone(),
        location: "in_repo".to_string(),
        project_name: Some("demo".to_string()),
    })
    .expect("init should succeed");

    assert!(
        work.path().join("bullseye.yaml").is_file(),
        "in-repo init must write into the cwd"
    );

    // Discovery finds it, list works.
    let text = text_from_call_result(
        handle_list(ListTool {
            cwd,
            filter: "active".to_string(),
        })
        .expect("list after init should succeed"),
    );
    assert!(text.contains("🎯T1"), "listing missing T1: {text}");
}

#[test]
fn init_external_creates_file_in_shadow_tree() {
    use bullseye::handler::{handle_init, handle_list};
    use bullseye::tools::{InitTool, ListTool};

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    handle_init(InitTool {
        cwd: cwd.clone(),
        location: "external".to_string(),
        project_name: Some("demo".to_string()),
    })
    .expect("external init should succeed");

    // cwd stays clean.
    assert!(
        !work.path().join("bullseye.yaml").exists(),
        "external init must not write into the cwd"
    );

    // Shadow path contains the file.
    let mut expected = shadow.path().to_path_buf();
    for c in work.path().components() {
        if let std::path::Component::Normal(part) = c {
            expected.push(part);
        }
    }
    expected.push("bullseye.yaml");
    assert!(
        expected.is_file(),
        "shadow-tree file missing: {}",
        expected.display()
    );

    // Discovery finds it through discover_anywhere's external branch.
    let text = text_from_call_result(
        handle_list(ListTool {
            cwd,
            filter: "active".to_string(),
        })
        .expect("list after external init should succeed"),
    );
    assert!(text.contains("🎯T1"), "listing missing T1: {text}");
}

#[test]
fn init_refuses_when_file_already_exists_in_either_location() {
    use bullseye::handler::handle_init;
    use bullseye::tools::InitTool;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    handle_init(InitTool {
        cwd: cwd.clone(),
        location: "in_repo".to_string(),
        project_name: None,
    })
    .expect("first init should succeed");

    // Second init — even with a different location — is refused.
    let err = handle_init(InitTool {
        cwd,
        location: "external".to_string(),
        project_name: None,
    })
    .expect_err("second init must be refused");
    assert!(
        format!("{err:?}").contains("already exists"),
        "expected already-exists error"
    );
}

#[test]
fn in_repo_wins_when_both_locations_have_files() {
    use bullseye::handler::handle_list;
    use bullseye::store;
    use bullseye::tools::ListTool;

    let shadow = tempfile::tempdir().unwrap();
    let _guard = ShadowFixture::with_root(shadow.path());

    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_string_lossy().into_owned();

    // Pre-seed both locations. Use distinguishable content so the
    // assertion can prove which file was read.
    let in_repo_path = work.path().join("bullseye.yaml");
    std::fs::write(
        &in_repo_path,
        "schema_version: 1\ntargets:\n  T1:\n    name: IN_REPO_WINS\n    status: identified\n    value: 5\n    cost: 3\n    acceptance:\n      - a\n    origin: manual\n    discovered: 2026-01-01\n",
    )
    .unwrap();

    let mut shadow_file = store::shadow_path(shadow.path(), work.path());
    std::fs::create_dir_all(&shadow_file).unwrap();
    shadow_file.push("bullseye.yaml");
    std::fs::write(
        &shadow_file,
        "schema_version: 1\ntargets:\n  T1:\n    name: SHADOW_SHOULD_LOSE\n    status: identified\n    value: 5\n    cost: 3\n    acceptance:\n      - a\n    origin: manual\n    discovered: 2026-01-01\n",
    )
    .unwrap();

    let text = text_from_call_result(
        handle_list(ListTool {
            cwd,
            filter: "active".to_string(),
        })
        .expect("list should succeed"),
    );
    assert!(
        text.contains("IN_REPO_WINS"),
        "in-repo precedence broken: {text}"
    );
    assert!(
        !text.contains("SHADOW_SHOULD_LOSE"),
        "shadow file should not have been consulted: {text}"
    );
}

// --- Parse cache tests (🎯T13) ---

/// Write a minimal valid bullseye.yaml to a path.
fn write_yaml(path: &std::path::Path, target_name: &str) {
    use std::io::Write;
    write!(
        std::fs::File::create(path).unwrap(),
        "schema_version: 1\ntargets:\n  T1:\n    name: {target_name}\n    \
         status: identified\n    value: 3\n    cost: 2\n    acceptance:\n      \
         - done\n    discovered: 2026-04-15\n"
    )
    .unwrap();
}

#[test]
fn cache_hit_on_unchanged_mtime() {
    // Two consecutive loads of the same file with no modification in between
    // must return the same in-memory data without re-reading the disk. We
    // verify this indirectly: both loads succeed and agree on the target name.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "Original name");

    let first = store::load(&path).unwrap();
    let second = store::load(&path).unwrap();
    assert_eq!(first.targets["T1"].name, second.targets["T1"].name);
    assert_eq!(first.targets["T1"].name, "Original name");
}

#[test]
fn cache_miss_after_mtime_change() {
    // Writing new content to the file must cause the next load to return
    // the updated data, not the previously cached parse.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "First version");

    let first = store::load(&path).unwrap();
    assert_eq!(first.targets["T1"].name, "First version");

    // Rewrite the file with a new name. Use save() to ensure the cache is
    // evicted, then write fresh content to simulate an external edit.
    // We sleep briefly to guarantee a distinct mtime on systems with 1-second
    // mtime granularity (most Linux filesystems without noatime).
    std::thread::sleep(std::time::Duration::from_millis(10));
    write_yaml(&path, "Second version");

    let second = store::load(&path).unwrap();
    assert_eq!(
        second.targets["T1"].name, "Second version",
        "cache should have been invalidated after file was modified"
    );
}

#[test]
fn cache_evicted_after_save() {
    // After store::save() the cache entry is evicted so the next load
    // reads back what was actually written, not a stale in-memory snapshot.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "Original");

    let mut file = store::load(&path).unwrap();
    assert_eq!(file.targets["T1"].name, "Original");

    // Mutate and save.
    file.targets.get_mut("T1").unwrap().name = "Updated".to_string();
    store::save(&path, &file).unwrap();

    // Re-load must reflect the saved state (not the stale in-memory copy).
    let reloaded = store::load(&path).unwrap();
    assert_eq!(reloaded.targets["T1"].name, "Updated");
}

#[test]
fn cache_fallback_to_stale_on_reparse_failure() {
    // If the file becomes temporarily unreadable after the first successful
    // parse, the last valid cached copy is served rather than propagating
    // the I/O error (simulating a mid-edit state).
    use std::io::Write;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    write_yaml(&path, "Good state");

    // Prime the cache with a successful parse.
    let good = store::load(&path).unwrap();
    assert_eq!(good.targets["T1"].name, "Good state");

    // Overwrite with invalid YAML to simulate a mid-edit state.
    // Sleep briefly to ensure a new mtime on coarse-grained filesystems.
    std::thread::sleep(std::time::Duration::from_millis(10));
    writeln!(std::fs::File::create(&path).unwrap(), "not: valid: yaml: [").unwrap();

    // The load must succeed by serving the stale cached copy rather than
    // propagating the parse error.
    let fallback = store::load(&path).unwrap();
    assert_eq!(
        fallback.targets["T1"].name, "Good state",
        "expected stale cache fallback on parse failure"
    );
}

#[test]
fn concurrent_mutations_do_not_lose_updates() {
    // 🎯T17 regression test: two concurrent mutators each add a distinct
    // target to the same bullseye.yaml. Without flock, one mutation's
    // serialized-back-to-disk write clobbers the other. With flock, the
    // mutations serialise and both targets must be present at the end.
    //
    // We use threads rather than subprocesses because fs2's advisory
    // locks (flock(2) on POSIX, LockFileEx on Windows) are tied to the
    // open-file-description — each thread's independent
    // `OpenOptions::open(...)` gets a distinct OFD, so same-process
    // threads contend on the lock exactly like cross-process writers
    // would. This catches the same lost-update race with ~0ms overhead
    // per iteration (subprocess spawn would cost ~50ms × 2 × N iters).
    //
    // Loop count: 10 iterations, fresh tempdir per iteration. Each
    // iteration runs N concurrent writers and asserts every write
    // landed.
    use std::sync::{Arc, Barrier};
    use std::thread;

    const ITERATIONS: usize = 10;
    const WRITERS_PER_ITERATION: usize = 4;

    for iter in 0..ITERATIONS {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bullseye.yaml");
        write_yaml(&path, "Baseline");

        // All threads wait on this barrier before starting their
        // mutation — maximises contention on the lock.
        let barrier = Arc::new(Barrier::new(WRITERS_PER_ITERATION));

        let handles: Vec<_> = (0..WRITERS_PER_ITERATION)
            .map(|i| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    let new_id = format!("T{}", 1000 + i);
                    store::with_locked_mutation(&path, |file| -> Result<(), String> {
                        file.targets.insert(
                            new_id.clone(),
                            bullseye::schema::Target {
                                name: format!("Concurrent target {i}"),
                                status: Status::Identified,
                                value: 1.0,
                                cost: 1.0,
                                actual_cost: None,
                                set_aside_reason: None,
                                acceptance: vec!["done".to_string()],
                                checks: Vec::new(),
                                context: String::new(),
                                gates: Vec::new(),
                                depends_on: Vec::new(),
                                cross_depends: Vec::new(),
                                cross_enables: Vec::new(),
                                tags: Vec::new(),
                                strategy: None,

                                origin: "concurrent-test".to_string(),
                                discovered: chrono::Local::now().date_naive(),
                                achieved: None,
                            },
                        );
                        Ok(())
                    })
                    .unwrap_or_else(|e| {
                        panic!("iter {iter} thread {i}: locked mutation failed: {e}")
                    });
                })
            })
            .collect();

        for h in handles {
            h.join().expect("writer thread panicked");
        }

        // Every writer must have landed. Read fresh from disk —
        // bypass any cache by stat'ing directly (load() does this
        // via mtime, but parse_file is private; load() is fine).
        let final_file = store::load(&path).unwrap();
        for i in 0..WRITERS_PER_ITERATION {
            let id = format!("T{}", 1000 + i);
            assert!(
                final_file.targets.contains_key(&id),
                "iter {iter}: target {id} was lost — concurrent write clobbered it"
            );
        }
        // Plus the baseline T1 from write_yaml.
        assert!(
            final_file.targets.contains_key("T1"),
            "iter {iter}: baseline T1 was lost"
        );
    }
}

// --- 🎯T18: set-aside disposition ---

/// `bullseye_set_aside` flips a target's status to `set_aside` and
/// records the rationale; the target is then excluded from `active()`,
/// included in `set_aside()`, and unblocks its dependents the same
/// way an achieved target would.
#[test]
fn set_aside_marks_target_terminal_with_reason() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "set-aside").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let reason = "deferred to v2.0 — UX needs more thought";
    let result = handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: reason.to_string(),
    });
    assert!(result.is_ok(), "set_aside should succeed: {result:?}");

    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];
    assert_eq!(t1.status, Status::SetAside);
    assert_eq!(t1.set_aside_reason.as_deref(), Some(reason));

    // Excluded from active(), included in set_aside().
    assert!(!file.active().contains_key("T1"));
    assert!(file.set_aside().contains_key("T1"));
    // And NOT counted as achieved — that's the whole point.
    assert!(!file.achieved().contains_key("T1"));

    config::set_external_root_override(None);
}

/// Empty / whitespace-only reasons are rejected — the rationale is
/// the load-bearing artefact of the disposition.
#[test]
fn set_aside_rejects_empty_reason() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "set-aside-empty").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    for bad in ["", "   ", "\n\t  "] {
        let result = handle_set_aside(SetAsideTool {
            cwd: cwd.clone(),
            id: "T1".to_string(),
            reason: bad.to_string(),
        });
        assert!(
            result.is_err(),
            "empty/whitespace reason must be rejected: input={bad:?}"
        );
    }

    // Target must remain untouched after rejected calls.
    let file = store::load(&path).unwrap();
    assert_eq!(file.targets["T1"].status, Status::Identified);
    assert!(file.targets["T1"].set_aside_reason.is_none());

    config::set_external_root_override(None);
}

/// Already-achieved targets cannot be set aside — that would be
/// rewriting the achievement record. Already-set-aside targets are a
/// no-op (idempotent reporting, original reason preserved).
#[test]
fn set_aside_refuses_achieved_and_is_idempotent() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "set-aside-achieved").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Mark T1 achieved out-of-band, then try to set it aside.
    {
        let mut file = store::load(&path).unwrap();
        file.targets.get_mut("T1").unwrap().status = Status::Achieved;
        store::save(&path, &file).unwrap();
    }

    let after_achieved = handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: "doesn't matter — should be refused".to_string(),
    });
    assert!(
        after_achieved.is_err(),
        "set_aside on an achieved target must be refused"
    );
    let still_achieved = store::load(&path).unwrap();
    assert_eq!(still_achieved.targets["T1"].status, Status::Achieved);

    // Seed a T2 we can exercise idempotency on.
    {
        let mut file = store::load(&path).unwrap();
        let mut t2 = file.targets["T1"].clone();
        t2.name = "Idempotency probe".to_string();
        t2.status = Status::Identified;
        t2.set_aside_reason = None;
        t2.depends_on = vec![];
        file.targets.insert("T2".to_string(), t2);
        store::save(&path, &file).unwrap();
    }

    // Idempotency: set T2 aside with reason A, then try to set it
    // aside again with reason B — original reason wins, no error.
    let original = "parked pending design discussion";
    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T2".to_string(),
        reason: original.to_string(),
    })
    .unwrap();
    let second = handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T2".to_string(),
        reason: "different reason".to_string(),
    });
    assert!(
        second.is_ok(),
        "second set_aside on already-set-aside target should not error: {second:?}"
    );
    let file = store::load(&path).unwrap();
    assert_eq!(
        file.targets["T2"].set_aside_reason.as_deref(),
        Some(original)
    );

    config::set_external_root_override(None);
}

/// Set-aside targets unblock their dependents the same way achieved
/// targets do — the frontier surfaces the dependent once the upstream
/// is in a terminal disposition, regardless of which kind.
#[test]
fn set_aside_dependency_unblocks_frontier() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "set-aside-frontier").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Seed T2 depending on T1, then set T1 aside. T2 should appear
    // in the frontier afterwards.
    {
        let mut file = store::load(&path).unwrap();
        let mut t2 = file.targets["T1"].clone();
        t2.name = "Dependent of T1".to_string();
        t2.status = Status::Identified;
        t2.set_aside_reason = None;
        t2.depends_on = vec!["T1".to_string()];
        file.targets.insert("T2".to_string(), t2);
        store::save(&path, &file).unwrap();
    }

    // Pre-condition: T2 is blocked.
    let pre = bullseye::graph::frontier(&store::load(&path).unwrap());
    let pre_ids: Vec<_> = pre.iter().map(|f| f.id.as_str()).collect();
    assert!(
        !pre_ids.contains(&"T2"),
        "T2 should be blocked while T1 is identified; frontier was {pre_ids:?}"
    );

    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: "won't fix — superseded by 🎯T57".to_string(),
    })
    .unwrap();

    let post = bullseye::graph::frontier(&store::load(&path).unwrap());
    let post_ids: Vec<_> = post.iter().map(|f| f.id.as_str()).collect();
    assert!(
        post_ids.contains(&"T2"),
        "T2 should unblock once T1 is set aside; frontier was {post_ids:?}"
    );
    assert!(
        !post_ids.contains(&"T1"),
        "T1 should not appear in the frontier once it's set aside"
    );

    config::set_external_root_override(None);
}

/// `bullseye_validate` flags `status: set_aside` without a reason as
/// a structural error, and a `set_aside_reason` set on a non-set-aside
/// status as a stale leftover.
#[test]
fn validate_flags_set_aside_reason_mismatch() {
    use bullseye::graph::validate;
    use bullseye::schema::{Status, TargetsFile};

    // Start from a real file so the surrounding fields are valid; then
    // mutate just the status / reason to exercise validation.
    let tmp = tempfile::tempdir().unwrap();
    let path = bullseye::store::create_at(
        tmp.path(),
        bullseye::config::Location::InRepo,
        "validate-set-aside",
    )
    .unwrap();
    let mut file: TargetsFile = bullseye::store::load(&path).unwrap();

    // Case 1: set_aside without reason → error.
    file.targets.get_mut("T1").unwrap().status = Status::SetAside;
    file.targets.get_mut("T1").unwrap().set_aside_reason = None;
    let errs = validate(&file);
    assert!(
        errs.iter().any(|e| e.contains("set_aside_reason")),
        "missing reason must be flagged; errors: {errs:?}"
    );

    // Case 2: set_aside with whitespace-only reason → still error.
    file.targets.get_mut("T1").unwrap().set_aside_reason = Some("   ".to_string());
    let errs = validate(&file);
    assert!(
        errs.iter().any(|e| e.contains("set_aside_reason")),
        "whitespace-only reason must be flagged; errors: {errs:?}"
    );

    // Case 3: reason set on a non-set-aside status → error.
    file.targets.get_mut("T1").unwrap().status = Status::Identified;
    file.targets.get_mut("T1").unwrap().set_aside_reason = Some("stale".to_string());
    let errs = validate(&file);
    assert!(
        errs.iter()
            .any(|e| e.contains("set_aside_reason") && e.contains("only valid")),
        "stale reason on non-set-aside status must be flagged; errors: {errs:?}"
    );

    // Case 4: clean — set_aside with a real reason. No error from us.
    file.targets.get_mut("T1").unwrap().status = Status::SetAside;
    file.targets.get_mut("T1").unwrap().set_aside_reason =
        Some("parked pending review".to_string());
    let errs = validate(&file);
    assert!(
        !errs.iter().any(|e| e.contains("set_aside")),
        "valid set-aside should not produce set-aside-related errors; errors: {errs:?}"
    );
}

/// The summary header reports set-aside targets as a distinct count
/// (not lumped into achieved), and the `## Set aside` section lists
/// each target with its reason. See 🎯T18.
#[test]
fn summary_shows_set_aside_group_and_count() {
    use bullseye::config::{self, Location};
    use bullseye::graph;
    use bullseye::handler::handle_set_aside;
    use bullseye::schema::Status;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "summary-set-aside").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Set T1 aside.
    let reason = "out of scope for this cycle";
    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: reason.to_string(),
    })
    .unwrap();

    let file = store::load(&path).unwrap();
    let out = graph::summary(&file, "test/bullseye.yaml", None, false);

    // Header must name the set-aside count explicitly.
    assert!(
        out.contains("1 set aside"),
        "summary header must include set-aside count; output:\n{out}"
    );
    // Set-aside must NOT inflate the achieved count.
    assert!(
        !out.contains("1 achieved") || file.achieved().is_empty(),
        "set-aside must not inflate achieved count; output:\n{out}"
    );
    // A dedicated ## Set aside section must appear with the reason.
    assert!(
        out.contains("## Set aside"),
        "summary must have ## Set aside section; output:\n{out}"
    );
    assert!(
        out.contains(reason),
        "summary must include the set-aside reason; output:\n{out}"
    );
    // T1 must not appear in active targets.
    let file2 = store::load(&path).unwrap();
    assert_eq!(file2.targets["T1"].status, Status::SetAside);
    assert!(
        !file2.active().contains_key("T1"),
        "set-aside target must not appear in active()"
    );

    config::set_external_root_override(None);
}

/// `bullseye_list` with filter `"set_aside"` returns only set-aside
/// targets and shows the reason inline. See 🎯T18.
#[test]
fn list_set_aside_filter_returns_set_aside_targets() {
    use bullseye::config::{self, Location};
    use bullseye::handler::{handle_list, handle_set_aside};
    use bullseye::tools::{ListTool, SetAsideTool};

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "list-set-aside").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let reason = "won't fix — design changed";
    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: reason.to_string(),
    })
    .unwrap();

    let result = handle_list(ListTool {
        cwd: cwd.clone(),
        filter: "set_aside".to_string(),
    });
    assert!(result.is_ok(), "set_aside filter must succeed: {result:?}");

    let content = text_from_call_result(result.unwrap());

    assert!(
        content.contains("T1"),
        "set_aside list must include T1; content:\n{content}"
    );
    assert!(
        content.contains(reason),
        "set_aside list must show the reason; content:\n{content}"
    );
    // Active targets should not appear in set_aside filter.
    let file = store::load(&path).unwrap();
    for (id, t) in &file.targets {
        if t.status != bullseye::schema::Status::SetAside {
            assert!(
                !content.contains(&format!("🎯{id} ")),
                "active target {id} must not appear in set_aside filter; content:\n{content}"
            );
        }
    }

    config::set_external_root_override(None);
}

// --- 🎯T20: envelope-leak guard ---
//
// Tests for the check_no_envelope_leak validator wired into every
// mutating handler. The four markers are:
//   "<invoke "   "</invoke>"   "<parameter "   "</parameter>"
// Generic tags like <context> or <tags> are NOT rejected.

/// Each of the four envelope markers must be rejected on handle_put.name.
/// The error message must name both the field and the marker.
#[test]
fn put_rejects_envelope_markers_in_name() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let _path = store::create_at(tmp.path(), Location::InRepo, "envelope-name-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let markers = ["<invoke ", "</invoke>", "<parameter ", "</parameter>"];
    for marker in markers {
        let result = handle_put(PutTool {
            cwd: cwd.clone(),
            id: None,
            name: Some(format!("some {marker} name")),
            value: None,
            cost: None,
            acceptance: Some(vec!["CI green".to_string()]),
            context: None,
            status: None,
            depends_on: None,
            blocks: None,
            origin: None,
            tags: None,
        });
        let err = result.expect_err(&format!("marker `{marker}` in name must be rejected"));
        let msg = format!("{err:?}");
        assert!(
            msg.contains("name"),
            "error must name the field `name`; marker={marker:?}; got: {msg}"
        );
        assert!(
            msg.contains(marker.trim()),
            "error must name the marker; marker={marker:?}; got: {msg}"
        );
    }

    config::set_external_root_override(None);
}

/// Markers in context, acceptance items, tags, and origin are all rejected.
/// Tests the field names appear in the error messages.
#[test]
fn put_rejects_envelope_markers_in_other_fields() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let _path = store::create_at(tmp.path(), Location::InRepo, "envelope-fields-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let marker = "<invoke ";

    // context
    let r = handle_put(PutTool {
        cwd: cwd.clone(),
        id: None,
        name: Some("Legit name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["CI green".to_string()]),
        context: Some(format!("context with {marker} leaked")),
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    let msg = format!("{:?}", r.expect_err("marker in context must be rejected"));
    assert!(
        msg.contains("context"),
        "error must name field `context`; got: {msg}"
    );

    // acceptance[0]
    let r = handle_put(PutTool {
        cwd: cwd.clone(),
        id: None,
        name: Some("Legit name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec![format!("criterion {marker} bad")]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    let msg = format!(
        "{:?}",
        r.expect_err("marker in acceptance[0] must be rejected")
    );
    assert!(
        msg.contains("acceptance[0]"),
        "error must name field `acceptance[0]`; got: {msg}"
    );

    // tags[0]
    let r = handle_put(PutTool {
        cwd: cwd.clone(),
        id: None,
        name: Some("Legit name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["CI green".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: Some(vec![format!("bad{marker}tag")]),
    });
    let msg = format!("{:?}", r.expect_err("marker in tags[0] must be rejected"));
    assert!(
        msg.contains("tags[0]"),
        "error must name field `tags[0]`; got: {msg}"
    );

    // origin
    let r = handle_put(PutTool {
        cwd: cwd.clone(),
        id: None,
        name: Some("Legit name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["CI green".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,

        origin: Some(format!("{marker}bad-origin")),
        tags: None,
    });
    let msg = format!("{:?}", r.expect_err("marker in origin must be rejected"));
    assert!(
        msg.contains("origin"),
        "error must name field `origin`; got: {msg}"
    );

    config::set_external_root_override(None);
}

/// Legitimate angle-bracket content that is NOT an envelope marker must
/// pass validation — e.g. `<context>` or `<tags>` in prose.
#[test]
fn put_allows_legitimate_angle_bracket_prose() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let _path = store::create_at(tmp.path(), Location::InRepo, "envelope-prose-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // These contain angle brackets but are NOT envelope markers.
    let result = handle_put(PutTool {
        cwd: cwd.clone(),
        id: None,
        name: Some("Valid name with <context> reference".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec![
            "Output matches <expected>".to_string(),
            "No <tags> leakage".to_string(),
        ]),
        context: Some("<context> See design doc </context> for details".to_string()),
        status: None,
        depends_on: None,
        blocks: None,

        origin: Some("<manual> 2026-04-26".to_string()),
        tags: Some(vec!["<visual>".to_string()]),
    });
    assert!(
        result.is_ok(),
        "angle-bracket prose that isn't an envelope marker must pass; got: {result:?}"
    );

    config::set_external_root_override(None);
}

/// When handle_put rejects a call due to an envelope-leak, the file on
/// disk must be unchanged (no partial write).
#[test]
fn put_file_unchanged_on_envelope_rejection() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "envelope-unchanged-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Record the file content before the rejected call.
    let before = std::fs::read_to_string(&path).unwrap();

    let result = handle_put(PutTool {
        cwd: cwd.clone(),
        id: None,
        name: Some("Good name".to_string()),
        value: None,
        cost: None,
        acceptance: Some(vec!["<invoke bad".to_string()]),
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });
    assert!(
        result.is_err(),
        "envelope marker in acceptance must be rejected"
    );

    // File must be byte-for-byte identical after the rejected call.
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        before, after,
        "file must be unchanged when handle_put rejects an envelope marker"
    );

    config::set_external_root_override(None);
}

/// handle_import rejects markdown whose parsed targets carry envelope
/// markers in any free-text field, AND no YAML file is written.
#[test]
fn import_rejects_envelope_markers_in_parsed_markdown() {
    use bullseye::config;
    use bullseye::handler::handle_import;
    use bullseye::tools::ImportTool;

    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    // Markdown with a leaked closing-tag in the free-text description —
    // exactly the failure mode 🎯T20 was raised against.
    let md_path = tmp.path().join("targets.md");
    std::fs::write(
        &md_path,
        "# Targets\n\n## Active\n\n\
         ### 🎯T1 Some target\n\n\
         Description with </invoke> leaked from a malformed tool call.\n\n\
         - **Value**: 1\n\
         - **Cost**: 1\n\
         - **Acceptance**: ok\n\
         - **Status**: Identified\n\
         - **Discovered**: 2026-04-26\n",
    )
    .unwrap();

    let result = handle_import(ImportTool {
        cwd: cwd.clone(),
        path: Some(md_path.to_string_lossy().to_string()),
        location: "in_repo".to_string(),
        force: false,
    });
    let err = result.expect_err("import must reject envelope-marker leakage");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("</invoke>"),
        "error must name the marker; got: {msg}"
    );

    // No bullseye.yaml should have been written into the cwd.
    assert!(
        store::discover_anywhere(tmp.path()).is_none(),
        "import must not write a YAML file when validation rejects the input"
    );

    config::set_external_root_override(None);
}

/// `store::load` must continue to load pre-existing YAML files that
/// contain envelope markers — the validator is write-side only, so an
/// operator can repair a corrupted file in place.
#[test]
fn store_load_still_loads_corrupted_files() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    std::fs::write(
        &path,
        "schema_version: 1\n\
         targets:\n  \
           T1:\n    \
             name: Pre-corrupted target\n    \
             status: identified\n    \
             value: 1\n    \
             cost: 1\n    \
             acceptance: [ok]\n    \
             context: \"leaked </invoke> in context\"\n    \
             origin: manual\n    \
             discovered: 2026-04-26\n",
    )
    .unwrap();

    let file = store::load(&path).expect("load must succeed on corrupted file");
    assert!(file.targets["T1"].context.contains("</invoke>"));
}

/// handle_set_aside rejects an envelope marker in the reason field.
#[test]
fn set_aside_rejects_envelope_marker_in_reason() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_set_aside;
    use bullseye::tools::SetAsideTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "envelope-set-aside-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let result = handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T1".to_string(),
        reason: "deferred </invoke> because".to_string(),
    });
    let err = result.expect_err("envelope marker in reason must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("reason"),
        "error must name field `reason`; got: {msg}"
    );
    assert!(
        msg.contains("</invoke>"),
        "error must name the marker; got: {msg}"
    );

    // Target must remain un-set-aside.
    let file = store::load(&path).unwrap();
    assert_ne!(file.targets["T1"].status, Status::SetAside);

    config::set_external_root_override(None);
}

/// A non-conforming target ID — e.g. "T36.v1" that snuck in via a bad
/// tool call or hand edit — is a stylistic warning, not a structural
/// error. The graph operates fine on it (depends_on, verifies, frontier
/// resolution all key on the string itself), so frontier and convergence
/// must not block on it; otherwise the user has no way to retire or
/// set-aside the offending target without an out-of-band YAML edit.
#[test]
fn non_conforming_id_is_warning_not_blocking_error() {
    let mut file = load_fixture();
    // Inject a target with a non-conforming ID.
    let mut t = file.targets["T1"].clone();
    t.name = "Stand-in for an arbitrary check".to_string();
    file.targets.insert("T1.v1".to_string(), t);

    let warnings = graph::validate_warnings(&file);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("T1.v1") && w.contains("invalid target ID format")),
        "warnings must flag the non-conforming ID; got: {warnings:?}"
    );

    let errors = graph::validate_blocking(&file);
    assert!(
        !errors
            .iter()
            .any(|e| e.contains("invalid target ID format")),
        "ID format must NOT appear in blocking errors; got: {errors:?}"
    );

    // The legacy `validate()` (used by bullseye_validate's combined
    // report) still surfaces the warning — callers that want the union
    // get it.
    let combined = graph::validate(&file);
    assert!(
        combined
            .iter()
            .any(|e| e.contains("invalid target ID format")),
        "combined validate() must include the warning; got: {combined:?}"
    );
}

// ── 🎯T15.2: strategy schema tests ──────────────────────────────────────────

/// Parse a YAML target with a full strategy block.
#[test]
fn strategy_parsed_from_yaml() {
    let yaml = r#"
targets:
  T1:
    name: Sync dotfiles
    status: identified
    value: 3
    cost: 2
    acceptance:
      - yadm status is clean
    discovered: 2026-01-01
    strategy:
      command: "yadm add -u && yadm commit -m sync && yadm push"
      trigger: "cron:0 * * * *"
      timeout: "60s"
      retry:
        max_attempts: 5
        backoff: "exponential"
"#;
    let file: TargetsFile = serde_yaml_ng::from_str(yaml).unwrap();
    let strat = file.targets["T1"]
        .strategy
        .as_ref()
        .expect("strategy present");
    assert_eq!(
        strat.command,
        "yadm add -u && yadm commit -m sync && yadm push"
    );
    assert_eq!(strat.trigger, "cron:0 * * * *");
    assert_eq!(strat.timeout.as_deref(), Some("60s"));
    let retry = strat.retry.as_ref().expect("retry present");
    assert_eq!(retry.max_attempts, Some(5));
    assert_eq!(retry.backoff.as_deref(), Some("exponential"));
}

/// Round-trip a strategy through YAML serialise → parse without data loss.
#[test]
fn strategy_yaml_roundtrip() {
    let yaml = r#"
targets:
  T1:
    name: Sync dotfiles
    status: identified
    value: 3
    cost: 2
    acceptance:
      - yadm status is clean
    discovered: 2026-01-01
    strategy:
      command: "yadm push"
      trigger: "on_wake"
      timeout: "30m"
      retry:
        max_attempts: 3
        backoff: "linear:30m"
"#;
    let file: TargetsFile = serde_yaml_ng::from_str(yaml).unwrap();
    let serialised = serde_yaml_ng::to_string(&file).unwrap();
    let reparsed: TargetsFile = serde_yaml_ng::from_str(&serialised).unwrap();
    let orig = file.targets["T1"].strategy.as_ref().unwrap();
    let back = reparsed.targets["T1"].strategy.as_ref().unwrap();
    assert_eq!(orig, back);
}

/// Targets without a strategy field are unaffected — field is None and
/// existing YAML is not mutated by the new field.
#[test]
fn strategy_absent_is_none() {
    let file = load_fixture();
    for (id, target) in &file.targets {
        assert!(
            target.strategy.is_none(),
            "fixture target {id} should have no strategy, got: {:?}",
            target.strategy,
        );
    }
}

/// validate_blocking rejects a strategy with an empty command.
#[test]
fn validate_strategy_empty_command_is_error() {
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().strategy = Some(Strategy {
        command: "   ".to_string(), // whitespace only
        trigger: "on_wake".to_string(),
        timeout: None,
        retry: None,
    });
    let errors = graph::validate_blocking(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T1") && e.contains("strategy.command") && e.contains("empty")),
        "expected empty-command error; got: {errors:?}"
    );
}

/// validate_blocking rejects a strategy with an empty trigger.
#[test]
fn validate_strategy_empty_trigger_is_error() {
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().strategy = Some(Strategy {
        command: "make converge".to_string(),
        trigger: "".to_string(), // empty
        timeout: None,
        retry: None,
    });
    let errors = graph::validate_blocking(&file);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("T1") && e.contains("strategy.trigger") && e.contains("empty")),
        "expected empty-trigger error; got: {errors:?}"
    );
}

/// A well-formed strategy passes validate_blocking.
#[test]
fn validate_strategy_valid_passes() {
    let mut file = load_fixture();
    file.targets.get_mut("T1").unwrap().strategy = Some(Strategy {
        command: "make converge".to_string(),
        trigger: "cron:*/15 * * * *".to_string(),
        timeout: Some("5m".to_string()),
        retry: Some(RetryPolicy {
            max_attempts: Some(3),
            backoff: Some("exponential".to_string()),
        }),
    });
    let errors = graph::validate_blocking(&file);
    assert!(
        !errors.iter().any(|e| e.contains("strategy")),
        "valid strategy should not produce errors; got: {errors:?}"
    );
}

// ── 🎯T24: refuse mutation in submodule replicas / detached HEAD ────────────

/// Run `git -C <dir> <args>` and panic on failure with captured stderr.
/// Used by the 🎯T24 integration tests to set up parent + submodule
/// repos and to flip HEAD into a detached state. Identity / hooks
/// config matches the helpers in `git_commit::tests::git_init` so
/// commits work in CI without a global gitconfig.
fn t24_run_git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git invocation failed");
    assert!(
        out.status.success(),
        "git {args:?} failed in {dir:?}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Initialise a git repo at `dir` with stable identity + an empty
/// hooks dir so the developer's global pre-commit hooks don't fire.
fn t24_git_init(dir: &std::path::Path) {
    t24_run_git(dir, &["init", "-q", "-b", "master"]);
    t24_run_git(dir, &["config", "user.email", "test@example.com"]);
    t24_run_git(dir, &["config", "user.name", "Test"]);
    t24_run_git(dir, &["config", "commit.gpgsign", "false"]);
    let empty = dir.join(".git/empty-hooks");
    std::fs::create_dir_all(&empty).unwrap();
    t24_run_git(dir, &["config", "core.hooksPath", empty.to_str().unwrap()]);
}

/// Set the standard env vars `git commit` requires when no system
/// gitconfig is available (CI). The tests' per-repo `user.name` /
/// `user.email` config is enough on most platforms, but
/// `git submodule add` runs a sub-command in the child working tree
/// before our config takes effect — these env vars carry through.
fn t24_set_git_env() {
    // Safety: tests run sequentially within one process here, but
    // `cargo test` parallelises across tests by default. We set these
    // env vars defensively even though per-repo `user.*` config is
    // also populated; they're idempotent and process-local.
    unsafe {
        std::env::set_var("GIT_AUTHOR_NAME", "Test");
        std::env::set_var("GIT_AUTHOR_EMAIL", "test@example.com");
        std::env::set_var("GIT_COMMITTER_NAME", "Test");
        std::env::set_var("GIT_COMMITTER_EMAIL", "test@example.com");
    }
}

/// Commit count on `HEAD` for `repo`, or 0 if `HEAD` is unborn.
fn t24_commit_count(repo: &std::path::Path) -> usize {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap_or(0)
}

const T24_FIXTURE_YAML: &str = r#"schema_version: 1
targets:
  T1:
    name: Example target
    status: identified
    value: 1
    cost: 1
    acceptance:
      - it works
    origin: manual
    discovered: 2026-01-01
"#;

/// Mutating `bullseye_put` from inside a submodule replica must be
/// refused with a clear error naming the superproject path. The
/// submodule worktree must remain at its original commit count — no
/// auto-commit must land on a dangling local branch.
#[test]
fn t24_mutation_refused_in_submodule_replica() {
    use bullseye::config;
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    t24_set_git_env();

    let scratch = tempfile::tempdir().unwrap();
    let parent = scratch.path().join("parent");
    let inner = scratch.path().join("inner-source");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::create_dir_all(&inner).unwrap();

    // Build a self-contained "inner" repo to serve as the submodule
    // source. Carries a bullseye.yaml so `discover_anywhere` finds it
    // inside the submodule path.
    t24_git_init(&inner);
    std::fs::write(inner.join("bullseye.yaml"), T24_FIXTURE_YAML).unwrap();
    t24_run_git(&inner, &["add", "bullseye.yaml"]);
    t24_run_git(&inner, &["commit", "-q", "-m", "init inner"]);

    // Parent repo with the inner repo nested as a submodule. Modern
    // git refuses `submodule add` on file:// URLs by default; allow it.
    t24_git_init(&parent);
    std::fs::write(parent.join("README.md"), "parent\n").unwrap();
    t24_run_git(&parent, &["add", "README.md"]);
    t24_run_git(&parent, &["commit", "-q", "-m", "init parent"]);
    let inner_url = format!("file://{}", inner.display());
    t24_run_git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &inner_url,
            "sub",
        ],
    );
    t24_run_git(&parent, &["commit", "-q", "-m", "add submodule"]);

    let submodule = parent.join("sub");
    assert!(
        submodule.join("bullseye.yaml").exists(),
        "submodule should carry bullseye.yaml after add",
    );

    // Isolate the external shadow root so discover_anywhere can't pick
    // up a file in the developer's real ~/.local/share/bullseye.
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            bullseye::config::set_external_root_override(None);
        }
    }
    let _cleanup = Cleanup;

    let commits_before = t24_commit_count(&submodule);

    let result = handle_put(PutTool {
        cwd: submodule.to_string_lossy().to_string(),
        id: Some("T1".to_string()),
        name: Some("Renamed via submodule".to_string()),
        value: None,
        cost: None,
        acceptance: None,
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });

    let err = result.expect_err("mutation in submodule must be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("submodule"),
        "error should mention submodule: {msg}",
    );
    assert!(
        msg.contains(&parent.display().to_string()),
        "error should name the superproject path {parent:?}; got: {msg}",
    );
    assert!(
        msg.contains(&submodule.display().to_string()),
        "error should name the submodule cwd {submodule:?}; got: {msg}",
    );

    // Commit count in the submodule worktree is unchanged — no
    // auto-commit landed on a dangling branch.
    assert_eq!(
        t24_commit_count(&submodule),
        commits_before,
        "no commit should land in the submodule when the mutation is refused",
    );

    // The bullseye.yaml inside the submodule is byte-identical to the
    // fixture: the refusal happened before any read-modify-write.
    let after = std::fs::read_to_string(submodule.join("bullseye.yaml")).unwrap();
    assert_eq!(
        after, T24_FIXTURE_YAML,
        "submodule's bullseye.yaml must be untouched after a refused mutation",
    );
}

/// Mutating `bullseye_put` against a repo with detached HEAD must be
/// refused with a clear error naming the detached state. Auto-commit
/// onto a dangling local branch would otherwise lose the work.
#[test]
fn t24_mutation_refused_in_detached_head() {
    use bullseye::config;
    use bullseye::handler::handle_put;
    use bullseye::tools::PutTool;

    t24_set_git_env();

    let scratch = tempfile::tempdir().unwrap();
    let repo = scratch.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    t24_git_init(&repo);
    std::fs::write(repo.join("bullseye.yaml"), T24_FIXTURE_YAML).unwrap();
    t24_run_git(&repo, &["add", "bullseye.yaml"]);
    t24_run_git(&repo, &["commit", "-q", "-m", "init"]);
    // Detach HEAD by checking out the SHA directly.
    t24_run_git(&repo, &["checkout", "-q", "--detach", "HEAD"]);

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            bullseye::config::set_external_root_override(None);
        }
    }
    let _cleanup = Cleanup;

    let commits_before = t24_commit_count(&repo);

    let result = handle_put(PutTool {
        cwd: repo.to_string_lossy().to_string(),
        id: Some("T1".to_string()),
        name: Some("Renamed via detached HEAD".to_string()),
        value: None,
        cost: None,
        acceptance: None,
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
    });

    let err = result.expect_err("mutation under detached HEAD must be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("detached"),
        "error should mention detached HEAD: {msg}",
    );
    assert!(
        msg.contains(&repo.display().to_string()),
        "error should name the repo path {repo:?}; got: {msg}",
    );
    assert!(
        msg.contains("checkout") || msg.contains("switch"),
        "error should suggest a branch checkout: {msg}",
    );

    assert_eq!(
        t24_commit_count(&repo),
        commits_before,
        "no commit should land when detached HEAD blocks the mutation",
    );
    let after = std::fs::read_to_string(repo.join("bullseye.yaml")).unwrap();
    assert_eq!(after, T24_FIXTURE_YAML, "yaml must be untouched");
}

/// Read-only operations (here: `bullseye_list`) must succeed even
/// inside a submodule replica — the 🎯T24 guard fires only on
/// mutating handlers, so research from inside a parent project's
/// submodule still works.
#[test]
fn t24_read_only_ops_unaffected_in_submodule() {
    use bullseye::config;
    use bullseye::handler::handle_list;
    use bullseye::tools::ListTool;

    t24_set_git_env();

    let scratch = tempfile::tempdir().unwrap();
    let parent = scratch.path().join("parent");
    let inner = scratch.path().join("inner-source");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::create_dir_all(&inner).unwrap();

    t24_git_init(&inner);
    std::fs::write(inner.join("bullseye.yaml"), T24_FIXTURE_YAML).unwrap();
    t24_run_git(&inner, &["add", "bullseye.yaml"]);
    t24_run_git(&inner, &["commit", "-q", "-m", "init inner"]);

    t24_git_init(&parent);
    std::fs::write(parent.join("README.md"), "parent\n").unwrap();
    t24_run_git(&parent, &["add", "README.md"]);
    t24_run_git(&parent, &["commit", "-q", "-m", "init parent"]);
    let inner_url = format!("file://{}", inner.display());
    t24_run_git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &inner_url,
            "sub",
        ],
    );
    t24_run_git(&parent, &["commit", "-q", "-m", "add submodule"]);

    let submodule = parent.join("sub");
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            bullseye::config::set_external_root_override(None);
        }
    }
    let _cleanup = Cleanup;

    // bullseye_list is read-only; it must succeed despite the cwd
    // sitting inside a submodule worktree.
    handle_list(ListTool {
        cwd: submodule.to_string_lossy().to_string(),
        filter: "active".to_string(),
    })
    .expect("read-only list should succeed inside a submodule");
}

// --- bullseye_subdivide (🎯T27.1) -----------------------------------------

/// Stand up a fresh tempdir with three targets in a chain so each
/// subdivide test starts from the same shape:
///
///   T1 (identified, no deps)
///   T2 (identified, depends_on: [T1])
///   T3 (identified, depends_on: [T1])
///
/// T1 has two dependents. Subdivision against T1 in any mode is
/// observable as a change in how T2/T3 wire to whatever replaces T1.
fn subdivide_fixture() -> (tempfile::TempDir, tempfile::TempDir, String) {
    use bullseye::config::{self, Location};

    let tmp = tempfile::tempdir().unwrap();
    let path = store::create_at(tmp.path(), Location::InRepo, "subdivide-test").unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    // The starter file already seeded T1; add T2 and T3 depending on T1.
    let mut file = store::load(&path).unwrap();
    let today = chrono::Local::now().date_naive();
    for id in ["T2", "T3"] {
        file.targets.insert(
            id.to_string(),
            bullseye::schema::Target {
                name: format!("Dependent {id}"),
                status: bullseye::schema::Status::Identified,
                value: 0.0,
                cost: 0.0,
                actual_cost: None,
                set_aside_reason: None,
                acceptance: vec!["done".to_string()],
                checks: vec![],
                context: String::new(),
                gates: vec![],
                depends_on: vec!["T1".to_string()],
                cross_depends: vec![],
                cross_enables: vec![],
                tags: vec![],
                strategy: None,
                origin: "test".to_string(),
                discovered: today,
                achieved: None,
            },
        );
    }
    store::save(&path, &file).unwrap();

    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    (tmp, shadow_tmp, cwd)
}

fn child_spec(name: &str, acceptance: &[&str]) -> bullseye::tools::SubdivisionChild {
    bullseye::tools::SubdivisionChild {
        id: None,
        name: name.to_string(),
        acceptance: acceptance.iter().map(|s| s.to_string()).collect(),
        context: None,
        tags: None,
        depends_on: None,
    }
}

#[test]
fn subdivide_add_mode_leaves_parent_and_extends_dependents() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    handle_subdivide(SubdivideTool {
        cwd: cwd.clone(),
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![
            child_spec("Spillover A", &["does A"]),
            child_spec("Spillover B", &["does B"]),
        ],
        retire_reason: None,
    })
    .expect("add-mode subdivide should succeed");

    let file = store::load(&path).unwrap();

    // Two children created as sub-targets of T1.
    assert!(file.targets.contains_key("T1.1"));
    assert!(file.targets.contains_key("T1.2"));
    assert_eq!(file.targets["T1.1"].name, "Spillover A");
    assert_eq!(file.targets["T1.2"].name, "Spillover B");
    assert_eq!(file.targets["T1.1"].origin, "subdivide(🎯T1)");

    // Parent untouched.
    let t1 = &file.targets["T1"];
    assert_eq!(t1.status, Status::Identified);
    assert!(
        t1.depends_on.is_empty(),
        "add mode must not touch parent depends_on; got {:?}",
        t1.depends_on
    );

    // Dependents gain the new children alongside T1.
    let t2 = &file.targets["T2"];
    assert_eq!(t2.depends_on, vec!["T1", "T1.1", "T1.2"]);
    let t3 = &file.targets["T3"];
    assert_eq!(t3.depends_on, vec!["T1", "T1.1", "T1.2"]);

    config::set_external_root_override(None);
}

#[test]
fn subdivide_aggregate_mode_makes_parent_umbrella() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "aggregate".to_string(),
        children: vec![
            child_spec("Sub A", &["does A"]),
            child_spec("Sub B", &["does B"]),
        ],
        retire_reason: None,
    })
    .expect("aggregate-mode subdivide should succeed");

    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];

    // Parent now depends on the new children and moves to converging.
    assert_eq!(t1.depends_on, vec!["T1.1", "T1.2"]);
    assert_eq!(t1.status, Status::Converging);

    // Dependents untouched (still pointing at T1 only).
    let t2 = &file.targets["T2"];
    assert_eq!(t2.depends_on, vec!["T1"]);
    let t3 = &file.targets["T3"];
    assert_eq!(t3.depends_on, vec!["T1"]);

    config::set_external_root_override(None);
}

#[test]
fn subdivide_retire_mode_retires_parent_and_rewires_dependents() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "retire".to_string(),
        children: vec![
            child_spec("Spillover A", &["does A"]),
            child_spec("Spillover B", &["does B"]),
        ],
        retire_reason: Some("original scope met; A and B emerged during work".to_string()),
    })
    .expect("retire-mode subdivide should succeed");

    let file = store::load(&path).unwrap();
    let t1 = &file.targets["T1"];

    // Parent retired with today's date and audit line in context.
    assert_eq!(t1.status, Status::Achieved);
    assert_eq!(t1.achieved, Some(chrono::Local::now().date_naive()));
    assert!(
        t1.context.contains("Subdivided ") && t1.context.contains("A and B emerged"),
        "retire-mode audit line missing or malformed; context: {:?}",
        t1.context,
    );

    // Dependents had T1 replaced by the children — original order
    // preserved, just spliced.
    let t2 = &file.targets["T2"];
    assert_eq!(t2.depends_on, vec!["T1.1", "T1.2"]);
    let t3 = &file.targets["T3"];
    assert_eq!(t3.depends_on, vec!["T1.1", "T1.2"]);

    config::set_external_root_override(None);
}

#[test]
fn subdivide_supports_explicit_child_ids() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::{SubdivideTool, SubdivisionChild};

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![SubdivisionChild {
            id: Some("T42".to_string()),
            name: "Top-level spillover".to_string(),
            acceptance: vec!["explicit ID".to_string()],
            context: None,
            tags: None,
            depends_on: None,
        }],
        retire_reason: None,
    })
    .expect("explicit-id subdivide should succeed");

    let file = store::load(&path).unwrap();
    assert!(
        file.targets.contains_key("T42"),
        "explicit child id T42 should be created"
    );
    // Dependents pick up T42 alongside T1.
    assert_eq!(file.targets["T2"].depends_on, vec!["T1", "T42"]);

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_id_collision() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::{SubdivideTool, SubdivisionChild};

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![SubdivisionChild {
            id: Some("T2".to_string()), // already exists
            name: "Collider".to_string(),
            acceptance: vec!["nope".to_string()],
            context: None,
            tags: None,
            depends_on: None,
        }],
        retire_reason: None,
    })
    .expect_err("id collision must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("T2"),
        "error should name the colliding id: {msg}"
    );

    // File untouched.
    let file = store::load(&path).unwrap();
    assert_eq!(file.targets["T2"].name, "Dependent T2");

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_terminal_parent() {
    use bullseye::config;
    use bullseye::handler::{handle_set_aside, handle_subdivide};
    use bullseye::tools::{SetAsideTool, SubdivideTool};

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    // Retire T2 directly (achieved).
    {
        let mut file = store::load(&path).unwrap();
        file.targets.get_mut("T2").unwrap().status = Status::Achieved;
        file.targets.get_mut("T2").unwrap().achieved = Some(chrono::Local::now().date_naive());
        store::save(&path, &file).unwrap();
    }

    // Achieved parent: rejected.
    let err = handle_subdivide(SubdivideTool {
        cwd: cwd.clone(),
        parent: "T2".to_string(),
        mode: "add".to_string(),
        children: vec![child_spec("nope", &["nope"])],
        retire_reason: None,
    })
    .expect_err("subdivide on achieved parent must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Achieved") && msg.contains("bullseye_revert"),
        "error should mention Achieved + bullseye_revert hint: {msg}"
    );

    // Set aside T3 and check the same rejection path.
    handle_set_aside(SetAsideTool {
        cwd: cwd.clone(),
        id: "T3".to_string(),
        reason: "deferred for the test".to_string(),
    })
    .unwrap();
    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T3".to_string(),
        mode: "add".to_string(),
        children: vec![child_spec("nope", &["nope"])],
        retire_reason: None,
    })
    .expect_err("subdivide on set-aside parent must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("SetAside") && msg.contains("bullseye_put"),
        "error should mention SetAside + bullseye_put hint: {msg}"
    );

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_empty_children() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");

    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![],
        retire_reason: None,
    })
    .expect_err("empty children must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("at least one child"),
        "error should explain the empty-children rule: {msg}"
    );

    // File untouched — T1 still has no sub-targets.
    let file = store::load(&path).unwrap();
    assert!(!file.targets.contains_key("T1.1"));

    config::set_external_root_override(None);
}

#[test]
fn subdivide_rejects_invalid_mode() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    let (_tmp, _shadow, cwd) = subdivide_fixture();

    let err = handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "replace".to_string(),
        children: vec![child_spec("nope", &["nope"])],
        retire_reason: None,
    })
    .expect_err("invalid mode must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("replace") && msg.contains("add"),
        "error should name the offending mode and list valid choices: {msg}"
    );

    config::set_external_root_override(None);
}

#[test]
fn subdivide_auto_assigns_unique_ids_for_multiple_children() {
    use bullseye::config;
    use bullseye::handler::handle_subdivide;
    use bullseye::tools::SubdivideTool;

    // If T1.1 already exists (e.g. earlier sub-target), auto-assignment
    // must skip past it instead of colliding.
    let (tmp, _shadow, cwd) = subdivide_fixture();
    let path = tmp.path().join("bullseye.yaml");
    {
        let mut file = store::load(&path).unwrap();
        let mut t11 = file.targets["T1"].clone();
        t11.name = "Pre-existing T1.1".to_string();
        t11.status = Status::Identified;
        t11.depends_on = vec![];
        file.targets.insert("T1.1".to_string(), t11);
        store::save(&path, &file).unwrap();
    }

    handle_subdivide(SubdivideTool {
        cwd,
        parent: "T1".to_string(),
        mode: "add".to_string(),
        children: vec![child_spec("First", &["a"]), child_spec("Second", &["b"])],
        retire_reason: None,
    })
    .expect("auto-assign past T1.1 should succeed");

    let file = store::load(&path).unwrap();
    // Existing T1.1 untouched.
    assert_eq!(file.targets["T1.1"].name, "Pre-existing T1.1");
    // New children start at T1.2 and continue from there.
    assert_eq!(file.targets["T1.2"].name, "First");
    assert_eq!(file.targets["T1.3"].name, "Second");

    config::set_external_root_override(None);
}
