// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Write isolation and reporting honesty (🎯T82).
//!
//! THE INCIDENT (2026-09-06, `squz/yourworld2` ledger). A single
//! `bullseye_put` patching one acceptance string on T57 silently
//! stripped the `achieved:` date from thirteen *other* targets — every
//! one whose `status` disagreed with its date (twelve `identified`, one
//! `set_aside`). Target count was unchanged at 88; `achieved:` lines
//! fell from 48 to 35. The tool reported `changed: T57`.
//!
//! Two independent defects, one test module each below.
//!
//! 1. **Blast radius.** `heal_status_scoped_residue` (🎯T64) runs at
//!    *load*, over every target, and the next save persists it. So any
//!    write persists repairs to targets the caller never named, and the
//!    loss lands in a commit whose message is about something else.
//!
//! 2. **Destructiveness.** For `achieved` specifically, the "repair"
//!    destroyed the only record of when a reopened target had previously
//!    been achieved. That date is a historical fact, not a status
//!    attribute — see `reopen_preserves_the_previous_achievement_date`.
//!
//! The fixture below is the shape of the yourworld2 ledger reduced to
//! its essentials: one target carrying residue, one unrelated target to
//! patch. The full captured repro lives at
//! `~/think/burst-2026-09/bullseye-repro/`.

use super::support::*;
use std::io::Write;

/// A ledger where T1 carries an `achieved:` date while sitting at a
/// non-achieved status — exactly the rows yourworld2 lost — plus an
/// unrelated T2 for the mutation to target.
const RESIDUE_YAML: &str = r#"
schema_version: 5
targets:
  T1:
    name: Previously achieved, since reopened
    status: identified
    value: 5
    cost: 3
    acceptance:
      - The thing holds
    context: Reopened after a regression.
    discovered: 2026-03-01
    achieved: 2026-03-10
  T2:
    name: Unrelated work
    status: identified
    value: 3
    cost: 2
    acceptance:
      - Some other thing holds
    discovered: 2026-03-01
"#;

fn write_ledger(dir: &std::path::Path, yaml: &str) -> PathBuf {
    let path = dir.join("bullseye.yaml");
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();
    path
}

/// Patch T2's name and nothing else, through the same locked-mutation
/// path every real op uses.
fn patch_t2_name(path: &std::path::Path) {
    store::with_locked_mutation(path, |file| {
        let t2 = file.targets.get_mut("T2").expect("T2 present");
        t2.name = "Unrelated work, renamed".to_string();
        Ok::<(), bullseye::api::CodedError>(())
    })
    .expect("mutation should succeed");
}

#[test]
fn writing_one_target_does_not_alter_another_targets_fields() {
    // The core invariant. Patch T2; every field of T1 must be unchanged.
    let tmp = tempfile::tempdir().unwrap();
    let path = write_ledger(tmp.path(), RESIDUE_YAML);

    // Field-level, not byte-level: `save` re-renders the whole file, so
    // `cost: 3` becomes `cost: 3.0` and serde defaults like `origin:
    // manual` materialise. Those are canonical-form changes that touch
    // every target equally and carry no meaning. What must not change is
    // any *value* T1 holds.
    //
    // This comparison is only meaningful because it runs on a fixture
    // whose T1 needs no healing — so a difference can only come from the
    // write. The healing-persistence case has its own raw-text test
    // below, where a struct comparison would pass vacuously.
    let t1_before = store::load(&path).unwrap().targets["T1"].clone();

    patch_t2_name(&path);

    let t1_after = store::load(&path).unwrap().targets["T1"].clone();

    assert_eq!(
        t1_before, t1_after,
        "patching T2 must leave every field of T1 untouched; \
         a write to one target is not a licence to rewrite others",
    );
}

#[test]
fn writing_one_target_does_not_strip_another_targets_achieved_date() {
    // The specific loss, asserted on its own so a regression names the
    // field rather than just "T1 differs".
    let tmp = tempfile::tempdir().unwrap();
    let path = write_ledger(tmp.path(), RESIDUE_YAML);

    patch_t2_name(&path);

    let after = store::load(&path).unwrap();
    assert_eq!(
        after.targets["T1"].achieved,
        Some(chrono::NaiveDate::parse_from_str("2026-03-10", "%Y-%m-%d").unwrap()),
        "T1's achieved date records when it was previously achieved; \
         a patch to T2 must not consume it",
    );

    // And the date must survive on disk, not just in memory.
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        raw.contains("achieved: 2026-03-10"),
        "the date must still be in the file:\n{raw}",
    );
}

#[test]
fn reopen_preserves_the_previous_achievement_date() {
    // The deliberate semantic call behind 🎯T82. `achieved:` is the date
    // the target was most recently achieved — a fact about the past.
    // `status` alone answers "is it achieved now". Reopening changes the
    // present, so it must not erase the past: that date is the whole
    // record of the prior achievement, and a reopened target with no
    // date reads as one that was never delivered.
    //
    // `attestation` is deliberately NOT treated this way. It asserts a
    // target is *currently* verified, so reopening must clear it and
    // force a re-achieve to supply a fresh one.
    let tmp = tempfile::tempdir().unwrap();
    let yaml = r#"
schema_version: 5
targets:
  T1:
    name: Delivered then regressed
    status: achieved
    value: 5
    cost: 3
    acceptance:
      - The thing holds
    discovered: 2026-03-01
    achieved: 2026-03-10
    attestation: verified by the nightly run
"#;
    let path = write_ledger(tmp.path(), yaml);

    let mut file = store::load(&path).unwrap();
    bullseye::ops::revert(&mut file, "T1", "regression found in nightly").unwrap();

    let t1 = &file.targets["T1"];
    assert_eq!(t1.status, Status::Converging, "reopen moves status");
    assert_eq!(
        t1.achieved,
        Some(chrono::NaiveDate::parse_from_str("2026-03-10", "%Y-%m-%d").unwrap()),
        "the previous achievement date is history and must survive reopen",
    );
    assert!(
        t1.attestation.is_none(),
        "the attestation claims present verification and must not survive reopen",
    );
    assert!(
        t1.context.contains("Reverted") && t1.context.contains("regression found"),
        "context records why it was reopened: {:?}",
        t1.context,
    );
}

#[test]
fn a_reopened_target_with_its_date_intact_still_validates() {
    // Widening where `achieved:` is legal is only safe if validation
    // agrees. A ledger carrying a date on a non-achieved target must
    // load and validate clean rather than being "healed" into silence.
    let tmp = tempfile::tempdir().unwrap();
    let path = write_ledger(tmp.path(), RESIDUE_YAML);

    let file = store::load(&path).unwrap();
    let errors = graph::validate(&file);
    assert!(
        errors.is_empty(),
        "an achieved date on a reopened target is legal, not residue: {errors:?}",
    );
}

#[test]
fn achieved_date_does_not_make_a_target_count_as_achieved() {
    // The widening must not leak into any "is it done" judgement.
    // Status is the sole authority; the date is provenance.
    let tmp = tempfile::tempdir().unwrap();
    let path = write_ledger(tmp.path(), RESIDUE_YAML);
    let file = store::load(&path).unwrap();

    assert!(
        !file.achieved().contains_key("T1"),
        "T1 has a date but status identified — it is not achieved",
    );
    assert!(file.active().contains_key("T1"), "T1 is still active work",);
}

#[test]
fn residue_that_is_genuinely_illegal_is_still_healed_in_memory() {
    // 🎯T64 must not regress. An `attestation` on a non-achieved target
    // is a real contradiction (it claims present verification of
    // something not achieved), so the loader still heals it rather than
    // leaving the ledger permanently invalid.
    let tmp = tempfile::tempdir().unwrap();
    let yaml = r#"
schema_version: 5
targets:
  T1:
    name: Carries an impossible attestation
    status: identified
    value: 5
    cost: 3
    acceptance:
      - The thing holds
    discovered: 2026-03-01
    attestation: claims verification of unachieved work
"#;
    let path = write_ledger(tmp.path(), yaml);

    let file = store::load(&path).expect("a bricked ledger must still read");
    assert!(
        file.targets["T1"].attestation.is_none(),
        "genuinely illegal residue still heals at load (🎯T64)",
    );
    let errors = graph::validate(&file);
    assert!(
        errors.is_empty(),
        "and the healed file validates: {errors:?}",
    );
}

#[test]
fn healing_another_target_is_not_persisted_by_an_unrelated_write() {
    // The blast-radius half. Even for residue that IS genuinely illegal
    // and heals in memory, an unrelated write must not persist that
    // repair to disk under a commit about something else. `op=rehash`
    // exists to do the load-and-save round trip deliberately.
    let tmp = tempfile::tempdir().unwrap();
    let yaml = r#"
schema_version: 5
targets:
  T1:
    name: Carries an impossible attestation
    status: identified
    value: 5
    cost: 3
    acceptance:
      - The thing holds
    discovered: 2026-03-01
    attestation: claims verification of unachieved work
  T2:
    name: Unrelated work
    status: identified
    value: 3
    cost: 2
    acceptance:
      - Some other thing holds
    discovered: 2026-03-01
"#;
    let path = write_ledger(tmp.path(), yaml);

    patch_t2_name(&path);

    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        raw.contains("attestation: claims verification of unachieved work"),
        "an unrelated write must not quietly rewrite T1 on disk; \
         the repair belongs to a deliberate rehash:\n{raw}",
    );
}

// --- The `changed:` list is the truth (🎯T82) ---------------------------

/// Every target id whose record differs between two ledgers.
fn differing_ids(before: &TargetsFile, after: &TargetsFile) -> Vec<String> {
    let mut ids: Vec<String> = after
        .targets
        .iter()
        .filter(|(id, t)| before.targets.get(*id) != Some(*t))
        .map(|(id, _)| id.clone())
        .collect();
    // A target that vanished changed too.
    ids.extend(
        before
            .targets
            .keys()
            .filter(|id| !after.targets.contains_key(*id))
            .cloned(),
    );
    ids.sort();
    ids
}

/// The ids named on the envelope's `changed:` line.
fn reported_changed(envelope: &str) -> Vec<String> {
    let line = envelope
        .lines()
        .find(|l| l.starts_with("changed:"))
        .unwrap_or_else(|| panic!("no changed: line in envelope:\n{envelope}"));
    let mut ids: Vec<String> = line
        .trim_start_matches("changed:")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "(none)")
        .collect();
    ids.sort();
    ids
}

#[test]
fn reported_changed_list_matches_what_actually_changed() {
    // The reporting half of 🎯T82. On the yourworld2 ledger the envelope
    // said `changed: T57` while thirteen other targets were rewritten, so
    // the loss landed in a commit whose message was about something else.
    // Run a real op over a ledger seeded with residue on an untouched
    // target, then compare the envelope's claim against a field-level
    // diff of the file. They must agree exactly.
    use bullseye::config;
    use bullseye::handler::handle_commit;
    use bullseye::tools::CommitTool;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bullseye.yaml");
    // T1 carries an attestation it may not have — genuinely illegal
    // residue, the kind the loader heals. T2 is what we patch.
    let yaml = r#"
schema_version: 5
targets:
  T1:
    name: Carries an impossible attestation
    status: identified
    value: 5
    cost: 3
    acceptance:
      - The thing holds
    discovered: 2026-03-01
    attestation: claims verification of unachieved work
  T2:
    name: Unrelated work
    status: identified
    value: 3
    cost: 2
    acceptance:
      - Some other thing holds
    discovered: 2026-03-01
"#;
    write!(std::fs::File::create(&path).unwrap(), "{yaml}").unwrap();

    let cwd = tmp.path().to_string_lossy().to_string();
    let shadow_tmp = tempfile::tempdir().unwrap();
    config::set_external_root_override(Some(shadow_tmp.path().to_path_buf()));

    let before = store::load(&path).unwrap();

    let result = handle_commit(CommitTool {
        cwd,
        op: "track".to_string(),
        id: Some("T2".to_string()),
        child_of: None,
        name: Some("Unrelated work, renamed".to_string()),
        value: None,
        cost: None,
        acceptance: None,
        checks: None,
        adopt_command_checks: false,
        context: None,
        status: None,
        depends_on: None,
        blocks: None,
        origin: None,
        tags: None,
        actual_cost: None,
        attestation: None,
        reason: None,
        postponed_until: None,
        postpone_predicate: None,
        parent: None,
        mode: None,
        children: None,
        retire_reason: None,
        tail: None,
        owner: None,
    })
    .expect("patching T2 should succeed");

    let envelope = text_from_call_result(result);
    let after = store::load(&path).unwrap();

    // Both `before` and `after` are healed views, so this comparison
    // alone cannot see a persisted heal — it would agree while T1 was
    // being rewritten underneath. The raw-text assertion below is what
    // makes this test bite on the healing case; the struct diff covers
    // every other kind of unreported edit.
    assert_eq!(
        reported_changed(&envelope),
        differing_ids(&before, &after),
        "the changed: list must name every target the write altered, and no others.\n\
         envelope:\n{envelope}",
    );
    assert_eq!(
        reported_changed(&envelope),
        vec!["T2".to_string()],
        "only T2 was patched:\n{envelope}",
    );

    // T1 was not named as changed, so T1 must be untouched on disk —
    // including the residue the loader heals in memory. An envelope that
    // says `changed: T2` while T1's bytes moved is the yourworld2 bug.
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        raw.contains("attestation: claims verification of unachieved work"),
        "T1 was not in the changed: list, so its record must be untouched \
         on disk:\n{raw}",
    );

    config::set_external_root_override(None);
}
