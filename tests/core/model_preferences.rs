// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;

use bullseye::apply::{self, ApplyRequest};
use bullseye::handler::handle_query;
use bullseye::models::{self, ModelClass, ModelPreferences};
use bullseye::schema::{Status, TargetsFile};
use serde_json::{Value, json};

use super::support::text_from_call_result;

fn ledger() -> TargetsFile {
    serde_yaml_ng::from_str("schema_version: 5\ntargets:\n  T1:\n    name: Sample\n    status: identified\n    value: 0\n    cost: 0\n    acceptance: [works]\n    context: original prose\n    discovered: 2026-01-01\n").unwrap()
}

fn apply(file: &mut TargetsFile, request: Value) -> Result<apply::ApplyReport, apply::ApplyError> {
    apply::apply(
        file,
        &serde_json::from_value::<ApplyRequest>(request).unwrap(),
        &HashSet::new(),
    )
}

#[test]
fn t107_absence_is_omitted_and_existing_canonical_bytes_are_unchanged() {
    let file = ledger();
    let before = bullseye::store::canonical_body(&file).unwrap();
    assert!(!before.contains("models:"));
    assert!(before.contains("schema_version: 5"));
    let roundtrip: TargetsFile = serde_yaml_ng::from_str(&before).unwrap();
    assert_eq!(before, bullseye::store::canonical_body(&roundtrip).unwrap());
}

#[test]
fn t107_adoption_roundtrip_replace_clear_and_version_ratchet() {
    let mut file = ledger();
    let before = file.targets.clone();
    let request = json!({"targets":{"T1":{"models":{"execute":{"class":"standard","effort":"high","pins":{"codex":"selected-id"}},"review":{"class":"frontier"}}}}});
    let err = apply(&mut file, request.clone()).unwrap_err();
    assert!(err.message.contains("adopt_model_preferences"));
    assert_eq!(file.targets, before);
    let mut adopted = request;
    adopted["adopt_model_preferences"] = json!(true);
    apply(&mut file, adopted).unwrap();
    assert_eq!(bullseye::schema::required_schema_version(&file), 7);
    let yaml = bullseye::store::canonical_body(&file).unwrap();
    let roundtrip: TargetsFile = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(roundtrip.targets, file.targets);
    assert!(yaml.contains("schema_version: 7"));
    // Omission leaves it alone; replacement drops the omitted execute role.
    apply(&mut file, json!({"targets":{"T1":{"value":3}}})).unwrap();
    assert!(
        file.targets["T1"]
            .models
            .as_ref()
            .unwrap()
            .execute
            .is_some()
    );
    apply(
        &mut file,
        json!({"targets":{"T1":{"models":{"review":{"class":"economy"}}}}}),
    )
    .unwrap();
    assert!(
        file.targets["T1"]
            .models
            .as_ref()
            .unwrap()
            .execute
            .is_none()
    );
    apply(&mut file, json!({"targets":{"T1":{"clear":["models"]}}})).unwrap();
    assert!(file.targets["T1"].models.is_none());
    assert_eq!(bullseye::schema::required_schema_version(&file), 5);
}

#[test]
fn t107_class_effort_and_field_typos_never_silently_disappear() {
    for bad in [
        json!({"execute":{"class":"fronter"}}),
        json!({"execute":{"class":"frontier","effort":"extreme"}}),
        json!({"executor":{"class":"frontier"}}),
        json!({"execute":{"class":"frontier","pin":{"claude":"opus"}}}),
    ] {
        assert!(serde_json::from_value::<ModelPreferences>(bad).is_err());
    }
    for bad in [
        json!({}),
        json!({"review":{"class":"standard","pins":{"":"opus"}}}),
        json!({"execute":{"class":"standard","pins":{"claude":" "}}}),
    ] {
        let mut file = ledger();
        let before = file.targets.clone();
        assert!(
            apply(
                &mut file,
                json!({"adopt_model_preferences":true,"targets":{"T1":{"models":bad}}})
            )
            .is_err()
        );
        assert_eq!(file.targets, before);
    }
}

#[test]
fn t107_achieved_preferences_are_immutable_except_representation_migration() {
    let mut file = ledger();
    apply(&mut file, json!({"adopt_model_preferences":true,"targets":{"T1":{"models":{"execute":{"class":"frontier"}}}}})).unwrap();
    apply(
        &mut file,
        json!({"targets":{"T1":{"status":"achieved","attestation":"verified"}}}),
    )
    .unwrap();
    let before = file.targets.clone();
    for patch in [
        json!({"models":{"execute":{"class":"economy"}}}),
        json!({"clear":["models"]}),
    ] {
        assert_eq!(
            apply(&mut file, json!({"targets":{"T1":patch}}))
                .unwrap_err()
                .code,
            bullseye::api::ErrorCode::ImmutableAchieved
        );
        assert_eq!(file.targets, before);
    }
}

#[test]
fn t107_frozen_go_inventory_migrates_every_role_without_losing_any_other_field() {
    let snapshot: Value =
        serde_json::from_str(include_str!("../fixtures/model-tags-golang-go.json")).unwrap();
    assert_eq!(snapshot["source_sha256"].as_str().unwrap().len(), 64);
    let inventory = snapshot["targets"].as_object().unwrap();
    let prototype = ledger().targets["T1"].clone();
    let mut file = TargetsFile::default();
    for (id, tags) in inventory {
        let mut t = prototype.clone();
        t.tags = serde_json::from_value(tags.clone()).unwrap();
        t.status = Status::Achieved;
        t.achieved = Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap());
        t.attestation = Some(format!("historical evidence for {id}"));
        t.depends_on = vec!["preserved-dependency".into()];
        file.targets.insert(id.clone(), t);
    }
    let original = file.targets.clone();
    assert!(models::migrate_tags(&mut file, false).is_err());
    assert_eq!(file.targets, original);
    let changed = models::migrate_tags(&mut file, true).unwrap();
    assert_eq!(changed.len(), inventory.len());
    for (id, target) in &file.targets {
        let mut restored = target.clone();
        let preferences = restored.models.take().unwrap();
        let expected_tags = &original[id].tags;
        for (prefix, role) in [
            ("model:", preferences.execute),
            ("review:", preferences.review),
        ] {
            let expected = expected_tags.iter().find_map(|t| t.strip_prefix(prefix));
            assert_eq!(
                role.as_ref()
                    .and_then(|p| p.pins.get("claude"))
                    .map(String::as_str),
                expected,
                "{id}/{prefix}"
            );
            if let Some(role) = role {
                let expected_class = match expected.unwrap() {
                    "haiku" => ModelClass::Economy,
                    "sonnet" => ModelClass::Standard,
                    "fable" | "opus" => ModelClass::Frontier,
                    other => panic!("unmapped source family {other}"),
                };
                assert_eq!(role.class, expected_class);
            }
        }
        assert_eq!(
            target.tags,
            expected_tags
                .iter()
                .filter(|t| !t.starts_with("model:") && !t.starts_with("review:"))
                .cloned()
                .collect::<Vec<_>>()
        );
        restored.tags = expected_tags.clone();
        assert_eq!(
            restored, original[id],
            "all other fields must survive for {id}"
        );
    }
    let migrated = file.targets.clone();
    assert!(models::migrate_tags(&mut file, true).unwrap().is_empty());
    assert_eq!(file.targets, migrated);
}

#[test]
fn t107_migration_refuses_unknown_and_conflicting_tags_atomically() {
    for tags in [vec!["model:opus", "model:fable"], vec!["model:future"]] {
        let mut file = ledger();
        file.targets.get_mut("T1").unwrap().tags = vec!["model:haiku".into()];
        let mut second = file.targets["T1"].clone();
        second.tags = tags.into_iter().map(str::to_owned).collect();
        file.targets.insert("T2".into(), second);
        let before = file.targets.clone();
        assert!(models::migrate_tags(&mut file, true).is_err());
        assert_eq!(file.targets, before);
    }
}

#[test]
fn t107_json_frontier_filters_capability_without_bypassing_graph_rules() {
    let tmp = tempfile::tempdir().unwrap();
    bullseye::config::set_external_root_override(Some(tmp.path().join("data")));
    let mut file = ledger();
    let prototype = file.targets["T1"].clone();
    for (id, preference) in [
        ("T2", json!({"execute":{"class":"economy"}})),
        ("T3", json!({"execute":{"class":"standard"}})),
        ("T4", json!({"execute":{"class":"frontier"}})),
        ("T5", json!({"review":{"class":"frontier"}})),
        ("T6", json!({"execute":{"class":"economy"}})),
    ] {
        let mut target = prototype.clone();
        target.models = Some(serde_json::from_value(preference).unwrap());
        if id == "T6" {
            target.depends_on = vec!["T4".into()];
        }
        file.targets.insert(id.into(), target);
    }
    bullseye::store::save(&tmp.path().join("bullseye.yaml"), &file).unwrap();
    let query = |class: &str| {
        let t = serde_json::from_value(
            json!({"cwd":tmp.path(),"view":"frontier","model_class":class,"format":"json"}),
        )
        .unwrap();
        let value: Value =
            serde_json::from_str(&text_from_call_result(handle_query(t).unwrap())).unwrap();
        value["targets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect::<HashSet<_>>()
    };
    assert_eq!(
        query("economy"),
        HashSet::from(["T1".into(), "T2".into(), "T5".into()])
    );
    assert_eq!(
        query("standard"),
        HashSet::from(["T1".into(), "T2".into(), "T3".into(), "T5".into()])
    );
    assert_eq!(
        query("frontier"),
        HashSet::from([
            "T1".into(),
            "T2".into(),
            "T3".into(),
            "T4".into(),
            "T5".into()
        ])
    );
    assert!(query("economy").is_subset(&query("standard")));
    assert!(query("standard").is_subset(&query("frontier")));
    for args in [
        json!({"view":"target","id":"T1","model_class":"standard"}),
        json!({"view":"frontier","model_class":"typo"}),
        json!({"view":"list","format":"json"}),
    ] {
        let mut args = args;
        args["cwd"] = json!(tmp.path());
        assert!(handle_query(serde_json::from_value(args).unwrap()).is_err());
    }
}
