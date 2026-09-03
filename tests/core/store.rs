// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

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

/// A file that already carries illegal status-scoped residue heals at
/// load (like `migrate_gates_to_depends_on`), so a ledger bricked by an
/// older binary or a hand edit reads correctly with no hand repair —
/// and `op=rehash` persists the repair to disk.
#[test]
fn t64_bricked_ledger_self_heals_on_load_and_rehash_persists_it() {
    use bullseye::config::{self, Location};
    use bullseye::handler::handle_commit;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    std::fs::write(&path, T64_BRICKED_YAML).unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow.path().to_path_buf()));
    let _ = Location::InRepo;

    // Load heals the residue; the ledger validates green immediately.
    let file = store::load(&path).unwrap();
    assert!(file.targets["T2"].set_aside_reason.is_none());
    assert_eq!(
        file.targets["T2"].attestation.as_deref(),
        Some("shipped in 1.2.3"),
        "attestation is legal on an achieved target and must survive",
    );
    let errors = graph::validate_blocking(&file);
    assert!(errors.is_empty(), "{errors:?}");

    // The supported repair op is a plain load-and-save round trip.
    let mut rehash = t64_commit(&cwd, "rehash");
    rehash.reason = Some("🎯T64 self-heal test".into());
    handle_commit(rehash).expect("rehash should succeed");

    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        !raw.contains("duplicate of T288"),
        "rehash must persist the heal:\n{raw}",
    );
    assert!(raw.contains("shipped in 1.2.3"), "{raw}");

    config::set_external_root_override(None);
}

/// One invalid target must not brick unrelated reads: frontier, list,
/// and target still answer, naming the offender rather than returning
/// only the error. `validate` is the one view that still reports errors
/// and nothing else — that is its contract.
#[test]
fn t64_one_invalid_target_does_not_brick_other_reads() {
    use bullseye::config;
    use bullseye::handler::handle_query;
    use bullseye::tools::QueryTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    std::fs::write(&path, T64_ONE_INVALID_YAML).unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();

    let shadow = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow.path().to_path_buf()));

    let query = |view: &str, id: Option<&str>| {
        text_from_call_result(
            handle_query(QueryTool {
                cwd: cwd.clone(),
                view: Some(view.to_string()),
                id: id.map(str::to_string),
                filter: None,
                momentum: None,
                frontier_details: None,
                recent_days: None,
                scope: None,
                nodes: None,
                seeds: None,
                expand: None,
            })
            .unwrap_or_else(|e| panic!("view={view} must still answer, got {e:?}")),
        )
    };

    // Frontier: healthy T1 is reported; the invalid T2 is named and
    // excluded; the answer is not replaced by the error.
    let front = query("frontier", None);
    assert!(front.contains("🎯T1"), "{front}");
    assert!(
        front.contains("Validation errors (degraded read)"),
        "{front}"
    );
    assert!(front.contains("T99"), "{front}");
    assert!(front.contains("1 target(s) ready for work"), "{front}");

    // List: every target is listed, with the offender annotated.
    let list = query("list", None);
    assert!(list.contains("🎯T1"), "{list}");
    assert!(list.contains("🎯T2"), "{list}");
    assert!(list.contains("INVALID:"), "{list}");

    // Target: both the healthy and the invalid target read back.
    let healthy = query("target", Some("T1"));
    assert!(healthy.contains("Healthy and ready"), "{healthy}");
    assert!(!healthy.contains("INVALID:"), "{healthy}");
    let invalid = query("target", Some("T2"));
    assert!(invalid.contains("Dangling dependency"), "{invalid}");
    assert!(invalid.contains("INVALID:"), "{invalid}");

    // Context and summary degrade the same way.
    let context = query("context", None);
    assert!(context.contains("🎯T1"), "{context}");
    assert!(
        context.contains("Validation errors (degraded read)"),
        "{context}"
    );

    // Validate is the surface whose job is the error report — it stays
    // a hard, error-only report.
    let validate = query("validate", None);
    assert!(validate.contains("## Errors"), "{validate}");
    assert!(validate.contains("T99"), "{validate}");

    config::set_external_root_override(None);
}

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
