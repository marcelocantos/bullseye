// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use super::support::*;

#[test]
fn verify_plan_builds_for_all_variants() {
    use bullseye::ops::{CheckKind, CheckOutcome, CheckSpec, CheckTool, verify_plan};
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
    assert_eq!(plan.checks[0].tool, CheckTool::CheckConventions);
    assert_eq!(plan.checks[1].tool, CheckTool::Query);
    assert_eq!(plan.checks[2].tool, CheckTool::CheckInvariants);

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

// --- `command:` check kind (🎯T83) --------------------------------------

#[test]
fn command_check_plans_a_shell_run_with_expected_exit() {
    // The fleet V-coverage survey (2026-09-06) found 0 of 995 active
    // targets carrying an executable check, and named the cause: the
    // schema could express sawmill structural queries and nothing else,
    // while 218 acceptance clauses assert "this command passes". This
    // kind closes that gap.
    use bullseye::ops::{CheckKind, CheckSpec, CheckTool, verify_plan};
    use bullseye::schema::{Check, CommandCheck};

    let mut file = load_fixture();
    let t3 = file.targets.get_mut("T3").unwrap();
    t3.checks = vec![
        Check::Command {
            command: CommandCheck {
                run: "cargo test --workspace".to_string(),
                cwd: None,
                expect_exit: None,
            },
        },
        Check::Command {
            command: CommandCheck {
                run: "./scripts/reject-bad-input.sh".to_string(),
                cwd: Some("tools".to_string()),
                expect_exit: Some(1),
            },
        },
    ];

    let plan = verify_plan(&file, "T3").unwrap();
    assert_eq!(plan.checks.len(), 2);

    assert_eq!(plan.checks[0].tool, CheckTool::Shell);
    assert_eq!(plan.report_template.checks[0].kind, CheckKind::Command);
    match &plan.checks[0].spec {
        CheckSpec::Command { command } => {
            assert_eq!(command.run, "cargo test --workspace");
            assert_eq!(command.cwd, None);
            // Absent `expect_exit` means 0 — what almost every check wants.
            assert_eq!(command.required_exit(), 0);
        }
        other => panic!("expected Command, got {other:?}"),
    }

    // Asserting a *failure* is expressible: a guard script that must
    // reject bad input exits non-zero on success of its own job.
    match &plan.checks[1].spec {
        CheckSpec::Command { command } => {
            assert_eq!(command.cwd.as_deref(), Some("tools"));
            assert_eq!(command.required_exit(), 1);
        }
        other => panic!("expected Command, got {other:?}"),
    }

    // The description a human reads must carry the command and the bar.
    assert!(
        plan.checks[0].description.contains("cargo test --workspace")
            && plan.checks[0].description.contains("expect_exit=0"),
        "description should name the command and the required exit: {:?}",
        plan.checks[0].description,
    );
}

#[test]
fn command_check_round_trips_through_yaml() {
    // The ledger is the interchange format, so the shape declared in a
    // `bullseye.yaml` must survive a load/save round trip unchanged.
    use bullseye::schema::{Check, TargetsFile};

    let yaml = r#"
schema_version: 5
targets:
  T1:
    name: The suite passes
    status: identified
    value: 5
    cost: 3
    acceptance:
      - cargo test --workspace exits 0
    discovered: 2026-09-06
    checks:
      - command:
          run: cargo test --workspace
      - command:
          run: make lint
          cwd: build
          expect_exit: 0
"#;
    let file: TargetsFile = serde_yaml_ng::from_str(yaml).expect("parses");
    let checks = &file.targets["T1"].checks;
    assert_eq!(checks.len(), 2, "both command checks parsed");
    match &checks[0] {
        Check::Command { command } => {
            assert_eq!(command.run, "cargo test --workspace");
            assert_eq!(command.required_exit(), 0);
        }
        other => panic!("expected Command, got {other:?}"),
    }

    let out = serde_yaml_ng::to_string(&file).expect("serializes");
    let reparsed: TargetsFile = serde_yaml_ng::from_str(&out).expect("re-parses");
    assert_eq!(
        reparsed.targets["T1"].checks, file.targets["T1"].checks,
        "command checks must survive a round trip:\n{out}",
    );
    // The default must not be written back out as noise.
    assert!(
        !out.contains("expect_exit: null"),
        "absent expect_exit should stay absent:\n{out}",
    );
}

#[test]
fn bullseye_never_executes_a_command_check() {
    // The cross-server constraint, asserted rather than assumed.
    // `bullseye.yaml` is a checked-in file that agents write, so a
    // server that shelled out to strings from it would be an
    // arbitrary-code-execution vector reachable by anyone who can land
    // a commit. Planning keeps the command in plain sight first.
    //
    // The oracle: plan a command that would leave an unmistakable trace
    // if anything ran it, then assert the trace is absent.
    use bullseye::ops::verify_plan;
    use bullseye::schema::{Check, CommandCheck};

    let tmp = tempfile::tempdir().unwrap();
    let canary = tmp.path().join("canary.txt");
    assert!(!canary.exists());

    let mut file = load_fixture();
    let t3 = file.targets.get_mut("T3").unwrap();
    t3.checks = vec![Check::Command {
        command: CommandCheck {
            run: format!("touch {}", canary.display()),
            cwd: None,
            expect_exit: None,
        },
    }];

    let plan = verify_plan(&file, "T3").expect("planning succeeds");
    assert_eq!(plan.checks.len(), 1, "the check was planned");
    assert!(
        !canary.exists(),
        "planning must not run the command — bullseye plans, the caller executes",
    );
}
