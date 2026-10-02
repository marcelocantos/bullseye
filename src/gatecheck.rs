// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Achieve-time gate enforcement (jevons 🎯T765.1, filed here as 🎯T94).
//!
//! `check_obligations` requires an `attestation` before a target may
//! transition to `achieved` (🎯T58), but until this module the check was
//! purely textual: any non-trivial string satisfied it, including one
//! that names a gate id which does not exist, does not say GREEN, or
//! ran on a commit that does not contain the fix. jevons' `internal/
//! gate/achieve.go` (🎯T765) audits exactly this shape of ledger row
//! *after* it was written; this module is the same verdict logic run
//! *before* the write, so a bad achieve is refused rather than merely
//! detected.
//!
//! Parsing is a direct port of jevons' `CheckAchieve`: an attestation
//! that cites no `GATE … id=…` line is `Ungated` — outside this
//! check's scope, same as upstream — and a cited gate is resolved
//! through the pluggable [`GateStore`] so this module stays testable
//! without a real `~/.jevons/gates` store or a git checkout.

use std::sync::LazyLock;

use regex::Regex;

/// One resolved gate record, as read from `~/.jevons/gates/<id>.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRecord {
    pub verdict: String, // GREEN | RED | DIRTY | SUSPECT | ...
    pub exit_status_zero: bool,
    pub tree_commit: Option<String>,
    pub tree_clean: Option<bool>,
    pub dirty_files: u64,
}

impl GateRecord {
    pub fn is_green(&self) -> bool {
        self.verdict.eq_ignore_ascii_case("GREEN")
    }
}

/// Resolves gate ids and commit ancestry. Implemented over the real
/// `~/.jevons/gates` store + `git merge-base` by [`FsGateStore`]; tests
/// use an in-memory stub.
pub trait GateStore {
    /// Look up a gate record by id. `None` when no such record exists.
    fn lookup(&self, id: &str) -> Option<GateRecord>;
    /// True when `fix` is `code` or an ancestor of it (`fix` was
    /// already in the tree the gate measured). Abbreviated shas are
    /// accepted; a fix newer than the gate's commit is `false`.
    fn contains(&self, code: &str, fix: &str) -> bool;
}

/// No-op store: every id is unknown, no commit contains any other.
/// Used where a real store cannot be constructed (isolated unit tests
/// of unrelated `apply` behaviour) so those tests are unaffected by
/// this module — an attestation with no `GATE … id=` citation never
/// consults the store at all (see [`AchieveVerdict::Ungated`]).
pub struct NullGateStore;

impl GateStore for NullGateStore {
    fn lookup(&self, _id: &str) -> Option<GateRecord> {
        None
    }
    fn contains(&self, _code: &str, _fix: &str) -> bool {
        false
    }
}

/// What [`check_attestation`] concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AchieveVerdict {
    /// Every cited gate resolves GREEN, clean-tree, and contains every
    /// named fix commit.
    Verified,
    /// No gate is cited or mentioned — outside this check's scope.
    Ungated,
    /// The gate evidence does not verify, but the attestation states an
    /// accepted risk about the gate itself. Visibly distinct from a
    /// verified green (🎯T94 acceptance 4).
    Marked { risk: String },
    /// The gate evidence does not support the achieve. The write must
    /// be refused.
    Refused { reasons: Vec<String> },
}

/// Visible prefix a marked achieve is stored with, matching jevons'
/// `gate.AchieveMarker`. A ledger reader who never re-runs the check
/// still sees that this row is not a verified green (🎯T94).
pub const ACCEPTED_RISK_MARKER: &str = "UNVERIFIED — ACCEPTED RISK";

/// Prefix `attestation` with [`ACCEPTED_RISK_MARKER`] unless it already
/// carries it, so re-stamp is idempotent. The original text is kept so
/// the cited `GATE … id=…` line remains on the row.
pub fn with_accepted_risk_marker(attestation: &str) -> String {
    let trimmed = attestation.trim();
    if trimmed.starts_with(ACCEPTED_RISK_MARKER) {
        trimmed.to_string()
    } else {
        format!("{ACCEPTED_RISK_MARKER}: {trimmed}")
    }
}

static LEDGER_GATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"GATE\s+(\S+)\s+(?:exit=(\S+)\s+)?(?:(GREEN|DIRTY|EMPTY|RED|SUSPECT|UNKNOWN|VOID|KILLED)\s+)?id=([0-9a-zA-Z]+)").unwrap()
});
static GATE_RISK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\baccepted[- ]risk\b[^\n.;]{0,120}?\b(dirty|-clean|clean|gate|gates)\b[^\n.;]*",
    )
    .unwrap()
});
static KEY_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\w+=\S+").unwrap());
static NAMED_COMMIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[0-9a-f]{7,40}\b").unwrap());

/// The commits an attestation names as evidence, with gate handles
/// (`id=…`, `out=…`) stripped so they are never mistaken for a fix sha.
pub fn named_commits(attestation: &str) -> Vec<String> {
    let text = LEDGER_GATE_RE.replace_all(attestation, " ");
    let text = KEY_VALUE_RE.replace_all(&text, " ");
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in NAMED_COMMIT_RE.find_iter(&text.to_ascii_lowercase()) {
        let sha = m.as_str();
        if seen.contains(sha) {
            continue;
        }
        let has_digit = sha.chars().any(|c| c.is_ascii_digit());
        let has_alpha = sha.chars().any(|c| c.is_ascii_alphabetic());
        if !has_digit || !has_alpha {
            continue;
        }
        seen.insert(sha.to_string());
        out.push(sha.to_string());
    }
    out
}

/// Check an achieve attestation's cited gate evidence against `store`.
/// Mirrors jevons' `internal/gate.CheckAchieve` (🎯T765) so a bad
/// achieve is refused before the write rather than merely flagged by a
/// later audit walk.
pub fn check_attestation(attestation: &str, store: &dyn GateStore) -> AchieveVerdict {
    let text = attestation.trim();
    let cited: Vec<_> = LEDGER_GATE_RE.captures_iter(text).collect();

    if cited.is_empty() {
        // 🎯T94 scope: this check is about a *cited* `GATE … id=…` line
        // that does not hold. Prose that merely contains the word
        // "gate"/"gated" ("T7 turned out not to gate this") is not a
        // citation and must not be treated as one — that word appears
        // in ordinary English having nothing to do with `bin/gate`.
        return AchieveVerdict::Ungated;
    }

    let fixes = named_commits(text);
    let mut reasons: Vec<String> = Vec::new();
    let mut hard_refuse = false;

    for m in &cited {
        let name = m.get(1).map(|g| g.as_str()).unwrap_or("");
        let cited_verdict = m.get(3).map(|g| g.as_str()).unwrap_or("");
        let id = m.get(4).map(|g| g.as_str()).unwrap_or("");
        let raw = m.get(0).map(|g| g.as_str()).unwrap_or("");

        let rec = match store.lookup(id) {
            Some(r) => r,
            None => {
                reasons.push(format!(
                    "gate id unknown — no gate record {id} exists, so the cited run for \
                     {name:?} (\"{raw}\") was not produced here"
                ));
                hard_refuse = true;
                continue;
            }
        };

        if !cited_verdict.is_empty() && !cited_verdict.eq_ignore_ascii_case(&rec.verdict) {
            reasons.push(format!(
                "verdict misquoted — attestation says {cited_verdict}, gate record {id} says \
                 {} (\"{raw}\")",
                rec.verdict
            ));
            hard_refuse = true;
        }

        if !rec.is_green() || !rec.exit_status_zero {
            reasons.push(format!(
                "verdict {} not GREEN — gate record {id} is exit_status_zero={} verdict={} \
                 (\"{raw}\")",
                if rec.verdict.is_empty() {
                    "UNKNOWN"
                } else {
                    &rec.verdict
                },
                rec.exit_status_zero,
                rec.verdict
            ));
            continue;
        }

        let commit = match &rec.tree_commit {
            Some(c) if !c.is_empty() => c.clone(),
            _ => {
                reasons.push(format!(
                    "gate tree commit unknown — record {id} carries no tree provenance, so the \
                     commit it measured is unknown (\"{raw}\")"
                ));
                continue;
            }
        };

        match rec.tree_clean {
            Some(true) => {}
            _ => {
                reasons.push(format!(
                    "tree.clean:false — gate record {id} ran in a working tree with {} \
                     uncommitted change(s) on {}; re-run as `bin/gate -clean -- <command>` \
                     (\"{raw}\")",
                    rec.dirty_files, commit
                ));
                continue;
            }
        }

        for fix in &fixes {
            if store.contains(&commit, fix) {
                continue;
            }
            reasons.push(format!(
                "commit {fix} is not an ancestor of the gate's measured commit {commit} \
                 (record {id}) — the gate did not measure the code being attested"
            ));
        }
    }

    if reasons.is_empty() {
        return AchieveVerdict::Verified;
    }
    if hard_refuse {
        return AchieveVerdict::Refused { reasons };
    }
    with_risk(text, reasons)
}

/// A failing check is a visible `Marked` accepted-risk only when the
/// attestation itself states an accepted risk about the gate evidence
/// (never for an unknown id or a misquoted verdict — those are never a
/// risk someone accepted, they are a citation of a run that did not
/// happen as described; callers set `hard_refuse` for those and never
/// reach `with_risk` at all).
fn with_risk(text: &str, reasons: Vec<String>) -> AchieveVerdict {
    if let Some(m) = GATE_RISK_RE.find(text) {
        return AchieveVerdict::Marked {
            risk: m.as_str().trim().to_string(),
        };
    }
    AchieveVerdict::Refused { reasons }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct StubStore {
        records: HashMap<String, GateRecord>,
        ancestors: std::collections::HashSet<(String, String)>,
    }

    impl GateStore for StubStore {
        fn lookup(&self, id: &str) -> Option<GateRecord> {
            self.records.get(id).cloned()
        }
        fn contains(&self, code: &str, fix: &str) -> bool {
            code == fix
                || self
                    .ancestors
                    .contains(&(code.to_string(), fix.to_string()))
        }
    }

    fn green(commit: &str) -> GateRecord {
        GateRecord {
            verdict: "GREEN".to_string(),
            exit_status_zero: true,
            tree_commit: Some(commit.to_string()),
            tree_clean: Some(true),
            dirty_files: 0,
        }
    }

    #[test]
    fn ungated_attestation_is_not_refused() {
        let store = StubStore {
            records: HashMap::new(),
            ancestors: Default::default(),
        };
        let v = check_attestation("Manually verified in the running cockpit.", &store);
        assert_eq!(v, AchieveVerdict::Ungated);
    }

    #[test]
    fn unknown_gate_id_is_refused() {
        let store = StubStore {
            records: HashMap::new(),
            ancestors: Default::default(),
        };
        let v = check_attestation("GATE make-test GREEN id=deadbeef", &store);
        match v {
            AchieveVerdict::Refused { reasons } => {
                assert!(reasons.iter().any(|r| r.contains("gate id unknown")));
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn misquoted_verdict_is_refused() {
        let mut records = HashMap::new();
        records.insert(
            "abc12345".to_string(),
            GateRecord {
                verdict: "RED".to_string(),
                exit_status_zero: false,
                tree_commit: Some("cafebabe".to_string()),
                tree_clean: Some(true),
                dirty_files: 0,
            },
        );
        let store = StubStore {
            records,
            ancestors: Default::default(),
        };
        let v = check_attestation("GATE make-test GREEN id=abc12345", &store);
        match v {
            AchieveVerdict::Refused { reasons } => {
                assert!(reasons.iter().any(|r| r.contains("verdict misquoted")));
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn non_green_is_refused() {
        let mut records = HashMap::new();
        records.insert(
            "abc12345".to_string(),
            GateRecord {
                verdict: "RED".to_string(),
                exit_status_zero: false,
                tree_commit: Some("cafebabe".to_string()),
                tree_clean: Some(true),
                dirty_files: 0,
            },
        );
        let store = StubStore {
            records,
            ancestors: Default::default(),
        };
        let v = check_attestation("GATE make-test id=abc12345", &store);
        match v {
            AchieveVerdict::Refused { reasons } => {
                assert!(reasons.iter().any(|r| r.contains("not GREEN")));
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn dirty_tree_is_refused() {
        let mut records = HashMap::new();
        records.insert(
            "abc12345".to_string(),
            GateRecord {
                verdict: "GREEN".to_string(),
                exit_status_zero: true,
                tree_commit: Some("cafebabe".to_string()),
                tree_clean: Some(false),
                dirty_files: 3,
            },
        );
        let store = StubStore {
            records,
            ancestors: Default::default(),
        };
        let v = check_attestation("GATE make-test GREEN id=abc12345", &store);
        match v {
            AchieveVerdict::Refused { reasons } => {
                assert!(reasons.iter().any(|r| r.contains("tree.clean:false")));
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn commit_not_ancestor_is_refused() {
        let mut records = HashMap::new();
        records.insert("abc12345".to_string(), green("cafebabe"));
        let store = StubStore {
            records,
            ancestors: Default::default(),
        };
        let v = check_attestation("GATE make-test GREEN id=abc12345 fixes deadbee1", &store);
        match v {
            AchieveVerdict::Refused { reasons } => {
                assert!(reasons.iter().any(|r| r.contains("is not an ancestor")));
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn descendant_commit_is_verified_not_just_exact_match() {
        let mut records = HashMap::new();
        records.insert("abc12345".to_string(), green("feedface"));
        let mut ancestors = std::collections::HashSet::new();
        ancestors.insert(("feedface".to_string(), "cafebabe".to_string()));
        let store = StubStore { records, ancestors };
        let v = check_attestation("GATE make-test GREEN id=abc12345 fixes cafebabe", &store);
        assert_eq!(v, AchieveVerdict::Verified);
    }

    #[test]
    fn exact_match_commit_is_verified() {
        let mut records = HashMap::new();
        records.insert("abc12345".to_string(), green("cafebabe"));
        let store = StubStore {
            records,
            ancestors: Default::default(),
        };
        let v = check_attestation("GATE make-test GREEN id=abc12345 fixes cafebabe", &store);
        assert_eq!(v, AchieveVerdict::Verified);
    }

    #[test]
    fn accepted_risk_marks_instead_of_refusing_a_dirty_gate() {
        let mut records = HashMap::new();
        records.insert(
            "abc12345".to_string(),
            GateRecord {
                verdict: "GREEN".to_string(),
                exit_status_zero: true,
                tree_commit: Some("cafebabe".to_string()),
                tree_clean: Some(false),
                dirty_files: 1,
            },
        );
        let store = StubStore {
            records,
            ancestors: Default::default(),
        };
        let v = check_attestation(
            "GATE make-test GREEN id=abc12345 — accepted-risk: gate ran dirty, tree \
             structurally cannot go clean here",
            &store,
        );
        match v {
            AchieveVerdict::Marked { risk } => {
                assert!(risk.to_lowercase().contains("accepted-risk"))
            }
            other => panic!("expected Marked, got {other:?}"),
        }
    }

    #[test]
    fn accepted_risk_does_not_launder_an_unknown_id() {
        let store = StubStore {
            records: HashMap::new(),
            ancestors: Default::default(),
        };
        let v = check_attestation(
            "GATE make-test GREEN id=deadbeef — accepted-risk: gate trust",
            &store,
        );
        match v {
            AchieveVerdict::Refused { .. } => {}
            other => panic!("expected Refused (unknown id must never be marked), got {other:?}"),
        }
    }

    #[test]
    fn named_commits_strips_gate_handles() {
        let commits = named_commits("GATE make-test GREEN id=abc12345 out=deadbeef fixes cafe1234");
        assert_eq!(commits, vec!["cafe1234".to_string()]);
    }

    #[test]
    fn accepted_risk_marker_prefixes_once() {
        let raw = "GATE make-test GREEN id=abc12345 — accepted-risk: gate ran dirty";
        let once = with_accepted_risk_marker(raw);
        assert!(once.starts_with(ACCEPTED_RISK_MARKER));
        assert!(once.contains(raw));
        assert_eq!(with_accepted_risk_marker(&once), once);
    }
}

/// Real [`GateStore`] over `~/.jevons/gates/<id>.json` (the store
/// `bin/gate` in the jevons harness writes) and `git merge-base
/// --is-ancestor` run in `repo_dir`.
///
/// `~/.jevons` is a fleet-wide convention (see jevons'
/// `internal/gate`), not a bullseye-specific path; a checkout with no
/// such directory, or a gate id bullseye cannot find there, resolves
/// to [`GateStore::lookup`] returning `None` — refused, not panicked.
pub struct FsGateStore {
    pub gates_dir: std::path::PathBuf,
    pub repo_dir: std::path::PathBuf,
}

impl FsGateStore {
    /// `repo_dir` is the git checkout whose ancestry `contains` walks.
    /// `~/.jevons/gates` is resolved from `$HOME` (or `$JEVONS_HOME` if
    /// set, for tests / alternate installs); no such directory simply
    /// means every lookup misses.
    pub fn new(repo_dir: impl Into<std::path::PathBuf>) -> Self {
        let home = std::env::var("JEVONS_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| std::path::PathBuf::from(h).join(".jevons")))
            .unwrap_or_else(|_| std::path::PathBuf::from(".jevons"));
        Self {
            gates_dir: home.join("gates"),
            repo_dir: repo_dir.into(),
        }
    }
}

#[derive(serde::Deserialize)]
struct GateJsonTree {
    commit: Option<String>,
    clean: Option<bool>,
    #[serde(default)]
    dirty_files: u64,
}

#[derive(serde::Deserialize)]
struct GateJson {
    #[serde(default)]
    verdict: String,
    #[serde(default)]
    exit_status: Option<i64>,
    tree: Option<GateJsonTree>,
}

impl GateStore for FsGateStore {
    fn lookup(&self, id: &str) -> Option<GateRecord> {
        // Reject anything that is not a bare filename component before it
        // ever touches the filesystem — a gate id embeds into a path.
        if id.is_empty() || id.contains('/') || id.contains('\\') || id == "." || id == ".." {
            return None;
        }
        let path = self.gates_dir.join(format!("{id}.json"));
        let bytes = std::fs::read(&path).ok()?;
        let parsed: GateJson = serde_json::from_slice(&bytes).ok()?;
        let (tree_commit, tree_clean, dirty_files) = match parsed.tree {
            Some(t) => (t.commit, t.clean, t.dirty_files),
            None => (None, None, 0),
        };
        Some(GateRecord {
            verdict: parsed.verdict,
            exit_status_zero: parsed.exit_status == Some(0),
            tree_commit,
            tree_clean,
            dirty_files,
        })
    }

    fn contains(&self, code: &str, fix: &str) -> bool {
        if code.eq_ignore_ascii_case(fix) {
            return true;
        }
        // git merge-base --is-ancestor: exit 0 means `fix` is an
        // ancestor of (or equal to) `code` — the real ancestry check,
        // not string-prefix guessing, so a rebased/renamed history is
        // still answered correctly.
        std::process::Command::new("git")
            .arg("-C")
            .arg(&self.repo_dir)
            .arg("merge-base")
            .arg("--is-ancestor")
            .arg(fix)
            .arg(code)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}
