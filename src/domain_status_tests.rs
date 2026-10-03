// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Domain-only transition oracle: no tools, filesystem or MCP envelopes.
use std::collections::{BTreeMap, HashSet};

use crate::apply::{self, ApplyRequest, Fragment};
use crate::ops::{self, SetAsideError, SetAsideOutcome};
use crate::schema::{STATUS_SCOPED_FIELDS, Status, Target, TargetsFile};

fn file() -> TargetsFile {
    serde_yaml_ng::from_str(
        "schema_version: 5\ntargets:\n  T1:\n    name: Test target\n    status: identified\n    value: 0\n    cost: 0\n    acceptance: [works]\n    discovered: 2026-01-01\n",
    )
    .unwrap()
}

/// Every persisted Target field must be classified when introduced. An
/// exhaustive destructure intentionally fails to compile for a new field;
/// the reviewer must add it to the invariant list and exercise its lifecycle.
/// `gates` is the one read-only migration field (never serialized).
fn classified_fields(target: &Target) -> ([&'static str; 6], Vec<&'static str>) {
    let Target {
        name: _,
        status: _,
        value: _,
        cost: _,
        actual_cost: _,
        set_aside_reason: _,
        attestation: _,
        acceptance: _,
        checks: _,
        context: _,
        gates: _,
        depends_on: _,
        cross_depends: _,
        cross_enables: _,
        tags: _,
        strategy: _,
        origin: _,
        discovered: _,
        achieved: _,
        owned_by: _,
        postponed_until: _,
        postpone_predicate: _,
    } = target;
    let scoped = [
        "set_aside_reason",
        "attestation",
        "achieved",
        "owned_by",
        "postponed_until",
        "postpone_predicate",
    ];
    let stable = vec![
        "name",
        "status",
        "value",
        "cost",
        "actual_cost",
        "acceptance",
        "checks",
        "context",
        "depends_on",
        "cross_depends",
        "cross_enables",
        "tags",
        "strategy",
        "origin",
        "discovered",
    ];
    (scoped, stable)
}

#[test]
fn all_persisted_fields_classified_and_status_fields_share_the_schema_table() {
    let mut ledger = file();
    let target = ledger.targets.get_mut("T1").unwrap();
    target.actual_cost = Some(1.0);
    target.set_aside_reason = Some("parked".into());
    target.attestation = Some("green test".into());
    target.context = "context".into();
    target.depends_on.push("T2".into());
    target.cross_depends.push(crate::schema::CrossEdge {
        repo: "repo".into(),
        target: Some("T3".into()),
        capability: None,
        note: None,
    });
    target.cross_enables = target.cross_depends.clone();
    target.tags.push("tag".into());
    target.strategy = Some(crate::schema::Strategy {
        command: "true".into(),
        trigger: "manual".into(),
        timeout: None,
        retry: None,
    });
    target.achieved = Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
    target.owned_by = Some(crate::schema::OwnedBy {
        owner: "agent".into(),
        reason: "reason".into(),
    });
    target.postponed_until = target.achieved;
    target.postpone_predicate = Some("wake".into());
    target.checks = vec![]; // Empty is an intentional default; the field is still classified.
    let (scoped, stable) = classified_fields(target);
    let table: HashSet<_> = STATUS_SCOPED_FIELDS.iter().map(|f| f.name).collect();
    assert_eq!(scoped.into_iter().collect::<HashSet<_>>(), table);
    let yaml = serde_yaml_ng::to_string(target).unwrap();
    let saved: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let keys: HashSet<_> = saved
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap())
        .collect();
    let classified: HashSet<_> = stable.into_iter().chain(scoped).collect();
    assert!(
        keys.is_subset(&classified),
        "unclassified serialized keys: {:?}",
        keys.difference(&classified).collect::<Vec<_>>()
    );
}

#[test]
fn legacy_set_aside_is_pure_and_preserves_noop_and_refusal() {
    for from in [Status::Identified, Status::Converging] {
        let mut ledger = file();
        let target = ledger.targets.get_mut("T1").unwrap();
        target.status = from;
        target.postpone_predicate = Some("hold".into());
        target.postponed_until = Some(chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
        target.owned_by = Some(crate::schema::OwnedBy {
            owner: "other".into(),
            reason: "why".into(),
        });
        assert_eq!(
            ops::set_aside(&mut ledger, "T1", "parked").unwrap(),
            SetAsideOutcome::SetAside {
                name: "Test target".into(),
                prior: from
            }
        );
        let target = &ledger.targets["T1"];
        assert_eq!(target.status, Status::SetAside);
        assert_eq!(target.set_aside_reason.as_deref(), Some("parked"));
        assert!(target.illegal_status_scoped_fields().is_empty());
        let before = ledger.clone();
        assert_eq!(
            ops::set_aside(&mut ledger, "T1", "ignored").unwrap(),
            SetAsideOutcome::AlreadySetAside {
                existing_reason: "parked".into()
            }
        );
        assert_eq!(ledger.targets, before.targets);
    }
    let mut ledger = file();
    assert_eq!(
        ops::set_aside(&mut ledger, "absent", "why"),
        Err(SetAsideError::TargetNotFound("absent".into()))
    );
    ledger.targets.get_mut("T1").unwrap().status = Status::Achieved;
    let before = ledger.clone();
    assert_eq!(
        ops::set_aside(&mut ledger, "T1", "why").unwrap(),
        SetAsideOutcome::AlreadyAchieved
    );
    assert_eq!(ledger.targets, before.targets);
}

#[test]
fn status_transition_matrix_clears_every_incompatible_persisted_field() {
    let statuses = [
        Status::Identified,
        Status::Converging,
        Status::Achieved,
        Status::SetAside,
    ];
    for from in statuses {
        for to in statuses {
            let mut ledger = file();
            let target = ledger.targets.get_mut("T1").unwrap();
            target.status = from;
            if from == Status::Achieved {
                target.achieved = Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
                target.attestation = Some("prior green".into());
            }
            if from == Status::SetAside {
                target.set_aside_reason = Some("prior reason".into());
            }
            if !from.is_terminal() {
                target.postpone_predicate = Some("pending".into());
                target.owned_by = Some(crate::schema::OwnedBy {
                    owner: "peer".into(),
                    reason: "why".into(),
                });
            }
            let mut targets = BTreeMap::new();
            targets.insert(
                "T1".to_string(),
                Fragment {
                    status: Some(
                        format!("{to:?}")
                            .to_lowercase()
                            .replace("setaside", "set_aside"),
                    ),
                    attestation: (to == Status::Achieved && from != to)
                        .then(|| "new green oracle".into()),
                    reason: Some("status changed for matrix".into()),
                    ..Default::default()
                },
            );
            let req = ApplyRequest {
                targets,
                ..Default::default()
            };
            apply::apply(&mut ledger, &req, &HashSet::new())
                .unwrap_or_else(|e| panic!("{from:?} → {to:?}: {e}"));
            let target = &ledger.targets["T1"];
            assert_eq!(target.status, to, "{from:?} → {to:?}");
            assert!(
                target.illegal_status_scoped_fields().is_empty(),
                "{from:?} → {to:?}"
            );
            assert_eq!(target.set_aside_reason.is_some(), to == Status::SetAside);
            assert_eq!(target.achieved.is_some(), to == Status::Achieved);
            assert_eq!(target.attestation.is_some(), to == Status::Achieved);
            if to.is_terminal() {
                assert!(target.owned_by.is_none() && target.postpone_predicate.is_none());
            }
        }
    }
}
