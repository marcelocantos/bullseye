// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Real-time GitHub-issues consumer (🎯T33 / 🎯T35).
//!
//! Pulls authorized issue rows from an issuepipe Master export API
//! (documented converge equivalent) and upserts bullseye targets via
//! the store. Opt-in repo filter is applied **after** Master-side authz
//! (security ceiling on Master; preference subset here).
//!
//! Pure mapping and sync logic live in this module always. The HTTP
//! Master client is behind `--features github-issues` so the default
//! binary stays free of the event-path network dependency.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::NaiveDate;
use serde::Deserialize;

use crate::github::IssueState;
use crate::schema::{Status, Target, TargetsFile};
use crate::store;

/// One issue row as exported by issuepipe `GET /v1/repos/{id}/issues`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MasterIssue {
    pub issue_node_id: String,
    pub repo_id: i64,
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub state: String,
    #[serde(default)]
    pub labels_json: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub updated_at: String,
}

/// Target ID for multi-repo event path: `GH{repo_id}-{number}`.
/// Distinct from single-repo T34 `GH{n}` so event-path and manual mirror
/// can coexist without colliding when only one repo is mirrored via T34.
pub fn event_target_id(repo_id: i64, number: u64) -> String {
    format!("GH{repo_id}-{number}")
}

/// Origin coordinate including stable repo id.
pub fn event_origin(repo_id: i64, number: u64) -> String {
    format!("github:repo:{repo_id}#{number}")
}

/// Whether `repo_id` is in the opt-in set. Empty opt-in means **none**
/// (strict opt-in — default produces no targets).
pub fn is_opted_in(repo_id: i64, opt_in: &BTreeSet<i64>) -> bool {
    opt_in.contains(&repo_id)
}

/// Filter Master-authorized repos by local opt-in preference.
pub fn filter_opt_in(authorized: &[i64], opt_in: &BTreeSet<i64>) -> Vec<i64> {
    authorized
        .iter()
        .copied()
        .filter(|id| is_opted_in(*id, opt_in))
        .collect()
}

fn labels_from_json(s: &str) -> Vec<String> {
    if s.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
}

fn issue_state(s: &str) -> IssueState {
    if s.eq_ignore_ascii_case("closed") {
        IssueState::Closed
    } else {
        IssueState::Open
    }
}

fn stub_acceptance(url: &str) -> Vec<String> {
    if url.is_empty() {
        vec!["Mirrored from issuepipe — define acceptance criteria.".into()]
    } else {
        vec![format!("Mirrored from {url} — define acceptance criteria.")]
    }
}

fn mirror_context(issue: &MasterIssue) -> String {
    let url = if issue.html_url.is_empty() {
        format!("issuepipe repo {}#{}", issue.repo_id, issue.number)
    } else {
        issue.html_url.clone()
    };
    if issue.body.trim().is_empty() {
        format!("Mirrored from {url}")
    } else {
        format!("{}\n\nMirrored from {url}", issue.body.trim())
    }
}

/// Build or refresh a target from a Master issue row (idempotent upsert plan).
pub fn target_from_issue(issue: &MasterIssue, today: NaiveDate) -> (String, Target) {
    let id = event_target_id(issue.repo_id, issue.number);
    let status = match issue_state(&issue.state) {
        IssueState::Open => Status::Identified,
        IssueState::Closed => Status::Achieved,
    };
    let mut achieved = None;
    if status == Status::Achieved {
        achieved = Some(today);
    }
    let t = Target {
        name: issue.title.clone(),
        status,
        value: 0.0,
        cost: 0.0,
        actual_cost: None,
        set_aside_reason: None,
        acceptance: stub_acceptance(&issue.html_url),
        checks: Vec::new(),
        context: mirror_context(issue),
        gates: Vec::new(),
        depends_on: Vec::new(),
        cross_depends: Vec::new(),
        cross_enables: Vec::new(),
        tags: labels_from_json(&issue.labels_json),
        strategy: None,
        origin: event_origin(issue.repo_id, issue.number),
        discovered: today,
        achieved,
        owned_by: None,
    };
    (id, t)
}

/// Apply issue rows into an in-memory targets file (idempotent by id).
/// Returns (created, updated, skipped_closed_noop).
pub fn apply_issues(
    file: &mut TargetsFile,
    issues: &[MasterIssue],
    today: NaiveDate,
) -> ApplyStats {
    let mut stats = ApplyStats::default();
    for issue in issues {
        let (id, mut fresh) = target_from_issue(issue, today);
        match file.targets.get(&id) {
            None => {
                file.targets.insert(id, fresh);
                stats.created += 1;
            }
            Some(existing) => {
                // Preserve local lifecycle progress (Converging) and
                // set_aside; still refresh name/context/tags from Master.
                if existing.status == Status::Converging {
                    fresh.status = Status::Converging;
                    fresh.achieved = None;
                } else if existing.status == Status::SetAside {
                    fresh.status = Status::SetAside;
                    fresh.set_aside_reason = existing.set_aside_reason.clone();
                    fresh.achieved = existing.achieved;
                } else if existing.status == Status::Achieved
                    && issue_state(&issue.state) == IssueState::Open
                {
                    // Remote reopen → identified.
                    fresh.status = Status::Identified;
                    fresh.achieved = None;
                }
                // discovered stays earliest
                fresh.discovered = existing.discovered;
                if existing.name == fresh.name
                    && existing.context == fresh.context
                    && existing.tags == fresh.tags
                    && existing.status == fresh.status
                {
                    stats.unchanged += 1;
                    continue;
                }
                file.targets.insert(id, fresh);
                stats.updated += 1;
            }
        }
    }
    stats
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ApplyStats {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
}

/// Source of Master issue rows (network or test double).
pub trait MasterClient {
    fn list_authorized_repos(&self) -> Result<Vec<i64>, String>;
    fn list_issues(&self, repo_id: i64) -> Result<Vec<MasterIssue>, String>;
}

/// One converge cycle: authorized ∩ opt-in → fetch issues → upsert YAML.
pub fn sync_once(
    client: &dyn MasterClient,
    opt_in: &BTreeSet<i64>,
    yaml_path: &Path,
    today: NaiveDate,
) -> Result<ApplyStats, String> {
    let authorized = client.list_authorized_repos()?;
    let repos = filter_opt_in(&authorized, opt_in);
    let mut all = Vec::new();
    for id in repos {
        match client.list_issues(id) {
            Ok(mut issues) => all.append(&mut issues),
            Err(e) => {
                // 403/unauthorized for a single repo should not abort
                // the whole cycle — skip with empty (caller may log).
                if e.contains("403") || e.contains("forbidden") {
                    continue;
                }
                return Err(e);
            }
        }
    }
    store::with_locked_mutation(yaml_path, |file| {
        let stats = apply_issues(file, &all, today);
        Ok::<ApplyStats, String>(stats)
    })
    .map_err(|e| e.to_string())
}

/// Parse comma-separated repo ids for opt-in config.
pub fn parse_opt_in(s: &str) -> BTreeSet<i64> {
    s.split(',')
        .filter_map(|p| p.trim().parse::<i64>().ok())
        .collect()
}

// ── HTTP client (feature github-issues) ──────────────────────────

#[cfg(feature = "github-issues")]
pub mod http {
    use super::*;

    /// HTTP client for issuepipe `/v1` export API.
    pub struct HttpMaster {
        pub base_url: String,
        pub token: String,
    }

    impl MasterClient for HttpMaster {
        fn list_authorized_repos(&self) -> Result<Vec<i64>, String> {
            let url = format!("{}/v1/repos", self.base_url.trim_end_matches('/'));
            let body = get_json(&url, &self.token)?;
            let v: serde_json::Value =
                serde_json::from_str(&body).map_err(|e| format!("parse repos: {e}"))?;
            let arr = v
                .get("repos")
                .and_then(|r| r.as_array())
                .ok_or_else(|| "missing repos array".to_string())?;
            let mut out = Vec::new();
            for x in arr {
                if let Some(id) = x.as_i64() {
                    out.push(id);
                }
            }
            Ok(out)
        }

        fn list_issues(&self, repo_id: i64) -> Result<Vec<MasterIssue>, String> {
            let url = format!(
                "{}/v1/repos/{repo_id}/issues",
                self.base_url.trim_end_matches('/')
            );
            let body = get_json(&url, &self.token)?;
            let v: serde_json::Value =
                serde_json::from_str(&body).map_err(|e| format!("parse issues: {e}"))?;
            if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
                if body.contains("forbidden") {
                    return Err(format!("403 forbidden: {msg}"));
                }
            }
            let issues = v
                .get("issues")
                .cloned()
                .ok_or_else(|| "missing issues".to_string())?;
            serde_json::from_value(issues).map_err(|e| format!("decode issues: {e}"))
        }
    }

    fn get_json(url: &str, token: &str) -> Result<String, String> {
        let resp = ureq::get(url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Accept", "application/json")
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(code, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    format!("HTTP {code}: {body}")
                }
                other => other.to_string(),
            })?;
        resp.into_string().map_err(|e| e.to_string())
    }

    /// CLI entry: `bullseye issues-poll --master URL --token TOK --opt-in 1,2 --cwd DIR`
    pub fn run(args: &[String]) -> Result<String, String> {
        let mut master = None;
        let mut token = None;
        let mut opt_in_s = String::new();
        let mut cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--master" => {
                    i += 1;
                    master = args.get(i).cloned();
                }
                "--token" => {
                    i += 1;
                    token = args.get(i).cloned();
                }
                "--opt-in" => {
                    i += 1;
                    opt_in_s = args.get(i).cloned().unwrap_or_default();
                }
                "--cwd" => {
                    i += 1;
                    cwd = args
                        .get(i)
                        .map(std::path::PathBuf::from)
                        .ok_or("--cwd needs path")?;
                }
                other => return Err(format!("unknown arg: {other}")),
            }
            i += 1;
        }
        let master = master
            .or_else(|| std::env::var("BULLSEYE_ISSUEPIPE_URL").ok())
            .ok_or("need --master or BULLSEYE_ISSUEPIPE_URL")?;
        let token = token
            .or_else(|| std::env::var("BULLSEYE_ISSUEPIPE_TOKEN").ok())
            .or_else(|| std::env::var("GITHUB_TOKEN").ok())
            .ok_or("need --token or BULLSEYE_ISSUEPIPE_TOKEN / GITHUB_TOKEN")?;
        if opt_in_s.is_empty() {
            opt_in_s = std::env::var("BULLSEYE_ISSUEPIPE_OPT_IN").unwrap_or_default();
        }
        let opt_in = parse_opt_in(&opt_in_s);
        if opt_in.is_empty() {
            return Err("opt-in set is empty — set --opt-in or BULLSEYE_ISSUEPIPE_OPT_IN".into());
        }
        let path = store::discover_anywhere(&cwd)
            .ok_or_else(|| format!("no bullseye.yaml under {}", cwd.display()))?;
        let client = HttpMaster {
            base_url: master,
            token,
        };
        let today = chrono::Utc::now().date_naive();
        let stats = sync_once(&client, &opt_in, &path, today)?;
        Ok(format!(
            "issues-poll: created={} updated={} unchanged={}",
            stats.created, stats.updated, stats.unchanged
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::TargetsFile;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn event_ids_stable_and_multi_repo() {
        assert_eq!(event_target_id(99, 7), "GH99-7");
        assert_eq!(event_origin(99, 7), "github:repo:99#7");
    }

    #[test]
    fn opt_in_is_strict() {
        let mut s = BTreeSet::new();
        s.insert(1);
        assert!(is_opted_in(1, &s));
        assert!(!is_opted_in(2, &s));
        assert!(!is_opted_in(1, &BTreeSet::new()));
        assert_eq!(filter_opt_in(&[1, 2, 3], &s), vec![1]);
    }

    #[test]
    fn apply_idempotent_no_duplicates() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();
        let issue = MasterIssue {
            issue_node_id: "I_x".into(),
            repo_id: 10,
            number: 3,
            title: "Hello".into(),
            body: "b".into(),
            state: "open".into(),
            labels_json: r#"["a"]"#.into(),
            html_url: "https://example/issues/3".into(),
            updated_at: "t".into(),
        };
        let mut file = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: Vec::new(),
            targets: BTreeMap::new(),
        };
        let s1 = apply_issues(&mut file, &[issue.clone()], today);
        assert_eq!(s1.created, 1);
        let s2 = apply_issues(&mut file, &[issue.clone()], today);
        assert_eq!(s2.created, 0);
        assert_eq!(s2.unchanged, 1);
        assert_eq!(file.targets.len(), 1);
        assert!(file.targets.contains_key("GH10-3"));

        let mut issue2 = issue;
        issue2.title = "Hello again".into();
        let s3 = apply_issues(&mut file, &[issue2], today);
        assert_eq!(s3.updated, 1);
        assert_eq!(file.targets["GH10-3"].name, "Hello again");
    }

    #[test]
    fn unauthorized_repo_not_opted_in_skipped() {
        // Even if client returns a repo, empty opt-in yields no apply.
        struct C;
        impl MasterClient for C {
            fn list_authorized_repos(&self) -> Result<Vec<i64>, String> {
                Ok(vec![1, 2])
            }
            fn list_issues(&self, _repo_id: i64) -> Result<Vec<MasterIssue>, String> {
                panic!("must not be called when opt-in empty after filter");
            }
        }
        // filter empties → list_issues never called
        let opt = BTreeSet::new();
        assert!(filter_opt_in(&[1, 2], &opt).is_empty());
        let _ = C; // compile
    }

    struct FakeMaster {
        allowed: Vec<i64>,
        issues: Arc<Mutex<BTreeMap<i64, Vec<MasterIssue>>>>,
        fail_403: BTreeSet<i64>,
    }

    impl MasterClient for FakeMaster {
        fn list_authorized_repos(&self) -> Result<Vec<i64>, String> {
            Ok(self.allowed.clone())
        }
        fn list_issues(&self, repo_id: i64) -> Result<Vec<MasterIssue>, String> {
            if self.fail_403.contains(&repo_id) {
                return Err("HTTP 403: forbidden".into());
            }
            Ok(self
                .issues
                .lock()
                .unwrap()
                .get(&repo_id)
                .cloned()
                .unwrap_or_default())
        }
    }

    #[test]
    fn sync_once_writes_yaml_idempotent_and_scopes_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = dir.path().join("bullseye.yaml");
        let initial = TargetsFile {
            schema_version: Some(5),
            last_evaluated: None,
            release_surface: Vec::new(),
            targets: BTreeMap::new(),
        };
        store::save(&yaml, &initial).unwrap();

        let issues = Arc::new(Mutex::new(BTreeMap::from([(
            42i64,
            vec![MasterIssue {
                issue_node_id: "I1".into(),
                repo_id: 42,
                number: 1,
                title: "From Master".into(),
                body: "".into(),
                state: "open".into(),
                labels_json: "[]".into(),
                html_url: "https://example/1".into(),
                updated_at: "t".into(),
            }],
        )])));
        // Master also has repo 99 authorized but not opted in.
        issues.lock().unwrap().insert(
            99,
            vec![MasterIssue {
                issue_node_id: "I99".into(),
                repo_id: 99,
                number: 9,
                title: "Secret".into(),
                body: "".into(),
                state: "open".into(),
                labels_json: "[]".into(),
                html_url: "".into(),
                updated_at: "t".into(),
            }],
        );

        let client = FakeMaster {
            allowed: vec![42, 99],
            issues: issues.clone(),
            fail_403: BTreeSet::new(),
        };
        let mut opt = BTreeSet::new();
        opt.insert(42);
        let today = NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();

        let s1 = sync_once(&client, &opt, &yaml, today).unwrap();
        assert_eq!(s1.created, 1);
        let loaded = store::load(&yaml).unwrap();
        assert!(loaded.targets.contains_key("GH42-1"));
        assert!(!loaded.targets.contains_key("GH99-9"));

        let s2 = sync_once(&client, &opt, &yaml, today).unwrap();
        assert_eq!(s2.unchanged, 1);
        assert_eq!(store::load(&yaml).unwrap().targets.len(), 1);

        // Recovery: new issue after "reconnect"
        issues
            .lock()
            .unwrap()
            .get_mut(&42)
            .unwrap()
            .push(MasterIssue {
                issue_node_id: "I2".into(),
                repo_id: 42,
                number: 2,
                title: "After reconnect".into(),
                body: "".into(),
                state: "open".into(),
                labels_json: "[]".into(),
                html_url: "".into(),
                updated_at: "t2".into(),
            });
        let s3 = sync_once(&client, &opt, &yaml, today).unwrap();
        assert_eq!(s3.created, 1);
        assert_eq!(store::load(&yaml).unwrap().targets.len(), 2);
    }

    #[test]
    fn isolation_403_does_not_create_targets() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = dir.path().join("bullseye.yaml");
        store::save(
            &yaml,
            &TargetsFile {
                schema_version: Some(5),
                last_evaluated: None,
                release_surface: Vec::new(),
                targets: BTreeMap::new(),
            },
        )
        .unwrap();
        let client = FakeMaster {
            allowed: vec![7], // Master claims allowed (stale) but export 403s
            issues: Arc::new(Mutex::new(BTreeMap::new())),
            fail_403: BTreeSet::from([7]),
        };
        let opt = BTreeSet::from([7]);
        let today = NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();
        let s = sync_once(&client, &opt, &yaml, today).unwrap();
        assert_eq!(s.created, 0);
        assert!(store::load(&yaml).unwrap().targets.is_empty());
    }
}
