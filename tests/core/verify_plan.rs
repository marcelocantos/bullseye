// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

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
