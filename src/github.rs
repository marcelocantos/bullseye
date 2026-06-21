// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! GitHub issue mirror (🎯T34) — a user-driven, `gh`-based two-way sync
//! between GitHub issues and bullseye targets. No server, no daemon:
//! `bullseye github sync` shells out to `gh`, which already carries the
//! caller's GitHub identity, so authentication is transitive and nothing
//! is invented or stored.
//!
//! ## Model
//!
//! - **Identity.** A mirrored issue becomes a target whose ID *is* the
//!   issue number in the `GH<n>` namespace (reserved `GH<repo>-<n>`
//!   for multi-repo). The ID is content-derived, so there is no
//!   local↔remote number mapping and no allocator slot; `origin =
//!   github:<owner>/<repo>#<n>` records the authoritative coordinate.
//! - **Field ownership** (loop prevention). Title / body / labels flow
//!   **GitHub → bullseye only**; lifecycle (open/closed) is the sole
//!   two-way field. Each direction has one writer-of-record, so prose
//!   edits never ping-pong.
//! - **Reconcile.** The lone two-way bit (issue open/closed ↔ target
//!   active/terminal) is merged three-way against a `.bullseye/
//!   github-sync.json` base: remote-only change → mirror in; local-only
//!   change → push out via `gh`. With a single binary bit a "both
//!   changed" case can only resolve to agreement, so the lifecycle
//!   reconcile is conflict-free by construction; the one event worth
//!   surfacing is a remote close that overrides locally-in-progress work.
//!
//! The reconcile is computed by the pure [`plan`] function so it is
//! exhaustively unit-testable without touching the network; [`run_with`]
//! applies the plan via a [`GhClient`] and the locked target store.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::schema::{Status, Target, TargetsFile};
use crate::store;

/// Stub acceptance written onto a freshly-mirrored target. A GitHub
/// issue is a task, not a testable desired state, so we cannot honestly
/// synthesise acceptance criteria — we leave a pointer for the user.
fn stub_acceptance(url: &str) -> Vec<String> {
    vec![format!("Mirrored from {url} — define acceptance criteria.")]
}

/// The `GH<n>` target ID for issue `number`.
pub fn target_id(number: u64) -> String {
    format!("GH{number}")
}

/// The authoritative `origin` coordinate for an issue.
pub fn origin_for(repo: &str, number: u64) -> String {
    format!("github:{repo}#{number}")
}

/// Open/closed state of a GitHub issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueState {
    Open,
    Closed,
}

/// A GitHub issue as far as the mirror cares about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub state: IssueState,
    pub url: String,
    pub updated_at: String,
}

/// Filters applied when listing issues.
#[derive(Debug, Clone, Default)]
pub struct ListOpts {
    /// Include closed issues (`--state all`) rather than just open ones.
    pub include_closed: bool,
    /// Only issues updated at or after this RFC3339 instant.
    pub since: Option<String>,
    /// Restrict to a label.
    pub label: Option<String>,
    /// Restrict to an assignee (`@me` for the authenticated user).
    pub assignee: Option<String>,
}

/// Reason a target's terminal status maps to when closing an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// Target achieved — `gh issue close --reason completed`.
    Completed,
    /// Target set aside — `gh issue close --reason "not planned"`.
    NotPlanned,
}

impl CloseReason {
    fn gh_value(self) -> &'static str {
        match self {
            CloseReason::Completed => "completed",
            CloseReason::NotPlanned => "not planned",
        }
    }
}

/// A lifecycle action to push to GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhAction {
    Close {
        number: u64,
        reason: CloseReason,
        comment: String,
    },
    Reopen {
        number: u64,
        comment: String,
    },
}

impl GhAction {
    fn number(&self) -> u64 {
        match self {
            GhAction::Close { number, .. } | GhAction::Reopen { number, .. } => *number,
        }
    }
}

/// The comment bullseye leaves when it drives an issue's lifecycle.
fn action_comment(status: Status) -> String {
    match status {
        Status::Achieved => "Closed by bullseye: target achieved.".to_string(),
        Status::SetAside => "Closed by bullseye: target set aside (not planned).".to_string(),
        Status::Identified | Status::Converging => {
            "Reopened by bullseye: target reactivated.".to_string()
        }
    }
}

/// Abstraction over the `gh` CLI so the reconcile logic can be tested
/// without a network or a real GitHub repo.
pub trait GhClient {
    /// Fail if the caller is not authenticated (`gh auth status`).
    fn auth_status(&self) -> Result<(), String>;
    /// The `owner/repo` slug for the current context.
    fn current_repo(&self) -> Result<String, String>;
    /// List issues matching `opts`.
    fn list_issues(&self, repo: &str, opts: &ListOpts) -> Result<Vec<Issue>, String>;
    /// Close an issue with a reason and a comment.
    fn close_issue(
        &self,
        repo: &str,
        number: u64,
        reason: CloseReason,
        comment: &str,
    ) -> Result<(), String>;
    /// Reopen an issue with a comment.
    fn reopen_issue(&self, repo: &str, number: u64, comment: &str) -> Result<(), String>;
}

/// Real [`GhClient`] that shells out to `gh`, run in `cwd` so an omitted
/// repo resolves to the current checkout.
pub struct RealGh {
    cwd: PathBuf,
}

impl RealGh {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }

    fn gh(&self) -> Command {
        let mut c = Command::new("gh");
        c.current_dir(&self.cwd);
        c
    }
}

/// Shape of one issue in `gh issue list --json …` output.
#[derive(Debug, Deserialize)]
struct GhIssueJson {
    number: u64,
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    labels: Vec<GhLabelJson>,
    /// `"OPEN"` or `"CLOSED"`.
    state: String,
    url: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct GhLabelJson {
    name: String,
}

impl From<GhIssueJson> for Issue {
    fn from(j: GhIssueJson) -> Self {
        Issue {
            number: j.number,
            title: j.title,
            body: j.body,
            labels: j.labels.into_iter().map(|l| l.name).collect(),
            state: if j.state.eq_ignore_ascii_case("closed") {
                IssueState::Closed
            } else {
                IssueState::Open
            },
            url: j.url,
            updated_at: j.updated_at,
        }
    }
}

/// Run a `gh` command, returning stdout on success or a structured
/// error (including stderr) on failure.
fn run_capture(mut cmd: Command) -> Result<Vec<u8>, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("failed to run gh (is the GitHub CLI installed?): {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("gh exited {}: {}", out.status, stderr.trim()));
    }
    Ok(out.stdout)
}

impl GhClient for RealGh {
    fn auth_status(&self) -> Result<(), String> {
        let mut c = self.gh();
        c.args(["auth", "status"]);
        run_capture(c)
            .map(|_| ())
            .map_err(|e| format!("not authenticated — run `gh auth login` first ({e})"))
    }

    fn current_repo(&self) -> Result<String, String> {
        let mut c = self.gh();
        c.args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ]);
        let out = run_capture(c)?;
        let slug = String::from_utf8_lossy(&out).trim().to_string();
        if slug.is_empty() {
            return Err(
                "could not determine the current repo (pass --repo owner/name)".to_string(),
            );
        }
        Ok(slug)
    }

    fn list_issues(&self, repo: &str, opts: &ListOpts) -> Result<Vec<Issue>, String> {
        let mut c = self.gh();
        c.args([
            "issue",
            "list",
            "-R",
            repo,
            "--limit",
            "1000",
            "--json",
            "number,title,body,labels,state,url,updatedAt",
            "--state",
            if opts.include_closed { "all" } else { "open" },
        ]);
        if let Some(l) = &opts.label {
            c.args(["--label", l]);
        }
        if let Some(a) = &opts.assignee {
            c.args(["--assignee", a]);
        }
        if let Some(s) = &opts.since {
            c.args(["--search", &format!("updated:>={s} sort:updated-asc")]);
        }
        let out = run_capture(c)?;
        let issues: Vec<GhIssueJson> =
            serde_json::from_slice(&out).map_err(|e| format!("parsing gh issue list JSON: {e}"))?;
        Ok(issues.into_iter().map(Issue::from).collect())
    }

    fn close_issue(
        &self,
        repo: &str,
        number: u64,
        reason: CloseReason,
        comment: &str,
    ) -> Result<(), String> {
        let mut c = self.gh();
        c.args([
            "issue",
            "close",
            &number.to_string(),
            "-R",
            repo,
            "--reason",
            reason.gh_value(),
            "--comment",
            comment,
        ]);
        run_capture(c).map(|_| ())
    }

    fn reopen_issue(&self, repo: &str, number: u64, comment: &str) -> Result<(), String> {
        let mut c = self.gh();
        c.args([
            "issue",
            "reopen",
            &number.to_string(),
            "-R",
            repo,
            "--comment",
            comment,
        ]);
        run_capture(c).map(|_| ())
    }
}

/// Per-issue base recorded in the sidecar — the three-way merge base
/// that distinguishes a local-only change from a remote-only change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRecord {
    /// Target status at last sync.
    pub last_status: Status,
    /// Issue open/closed at last sync.
    pub last_state: IssueState,
}

/// Sidecar sync state, persisted to `.bullseye/github-sync.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    pub repo: String,
    /// Newest `updatedAt` observed, used to bound the next pull.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<String>,
    #[serde(default)]
    pub issues: BTreeMap<u64, IssueRecord>,
}

impl SyncState {
    /// Path to the sidecar for the repo rooted at `repo_root` (the
    /// directory containing `bullseye.yaml`).
    pub fn path_for(repo_root: &Path) -> PathBuf {
        repo_root.join(".bullseye").join("github-sync.json")
    }

    /// Load the sidecar, or a fresh empty state if it doesn't exist.
    pub fn load(path: &Path) -> Result<SyncState, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("parsing {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SyncState::default()),
            Err(e) => Err(format!("reading {}: {e}", path.display())),
        }
    }

    /// Persist the sidecar, creating `.bullseye/` as needed.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        let body =
            serde_json::to_vec_pretty(self).map_err(|e| format!("serialising sync state: {e}"))?;
        std::fs::write(path, body).map_err(|e| format!("writing {}: {e}", path.display()))
    }
}

/// A planned mutation to the target store.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetOp {
    /// Create a brand-new mirrored target.
    Create { id: String, target: Box<Target> },
    /// Overwrite GitHub-owned content fields on an existing target.
    UpdateContent {
        id: String,
        name: String,
        context: String,
        tags: Vec<String>,
    },
    /// Apply a reconciled lifecycle status (mirror-in direction).
    SetStatus { id: String, status: Status },
}

/// The full plan produced by [`plan`]: store mutations, GitHub actions,
/// the next sidecar state, and human-facing notes.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub target_ops: Vec<TargetOp>,
    pub gh_actions: Vec<GhAction>,
    pub next_state: SyncState,
    /// Surfaced events (e.g. a remote close overriding in-progress work).
    pub notes: Vec<String>,
    pub created: usize,
    pub updated: usize,
    pub mirrored_in: usize,
    pub pushed: usize,
}

/// Inputs governing what the sync is allowed to do.
#[derive(Debug, Clone, Copy)]
pub struct PlanOpts {
    /// Apply remote → local (create/update/mirror-in).
    pub pull: bool,
    /// Apply local → remote (push lifecycle).
    pub push: bool,
    /// A label/assignee filter is active, so an issue's *absence* from
    /// the listing is ambiguous (could be filtered, not closed) — do
    /// not infer a remote close from absence.
    pub filtered: bool,
}

impl Default for PlanOpts {
    fn default() -> Self {
        Self {
            pull: true,
            push: true,
            filtered: false,
        }
    }
}

/// GitHub-owned context blob: the issue body followed by its URL.
fn mirror_context(issue: &Issue) -> String {
    if issue.body.trim().is_empty() {
        format!("Mirrored from {}", issue.url)
    } else {
        format!("{}\n\nMirrored from {}", issue.body.trim(), issue.url)
    }
}

/// Construct a fresh mirrored target from an open issue.
fn new_target(repo: &str, issue: &Issue, today: NaiveDate) -> Target {
    Target {
        name: issue.title.clone(),
        status: Status::Identified,
        value: 0.0,
        cost: 0.0,
        actual_cost: None,
        set_aside_reason: None,
        acceptance: stub_acceptance(&issue.url),
        checks: Vec::new(),
        context: mirror_context(issue),
        gates: Vec::new(),
        depends_on: Vec::new(),
        cross_depends: Vec::new(),
        cross_enables: Vec::new(),
        tags: issue.labels.clone(),
        strategy: None,
        origin: origin_for(repo, issue.number),
        discovered: today,
        achieved: None,
    }
}

/// The lifecycle bit: true = "closed/terminal", false = "open/active".
fn closed_bit(s: IssueState) -> bool {
    s == IssueState::Closed
}
fn terminal_bit(s: Status) -> bool {
    s.is_terminal()
}

/// Which way to drive an issue's lifecycle on push.
#[derive(Clone, Copy)]
enum PushKind {
    Close(CloseReason),
    Reopen,
}

/// Outcome of reconciling one issue's lifecycle bit against its base.
struct Reconciled {
    /// Mirror-in: set the target to this status.
    set_status: Option<Status>,
    /// Push: drive GitHub this way.
    push: Option<PushKind>,
    /// Lifecycle state to record as the new base (post-intended-sync).
    final_state: IssueState,
    /// Target status to record as the new base.
    final_status: Status,
    /// Note worth surfacing to the user.
    note: Option<String>,
}

/// Reconcile the lone two-way lifecycle bit (issue open/closed ↔ target
/// active/terminal) three-way against `base`.
///
/// With one binary bit, `remote_changed && local_changed` implies both
/// flipped away from the same base value and therefore now agree — so
/// there is no diverging-conflict branch. The reachable, surfaced event
/// is a remote close adopted over an in-progress (Converging) target.
fn reconcile(
    remote_state: IssueState,
    local_status: Status,
    base: &IssueRecord,
    opts: PlanOpts,
) -> Reconciled {
    let remote_bit = closed_bit(remote_state);
    let local_bit = terminal_bit(local_status);
    let base_bit = closed_bit(base.last_state);

    let remote_changed = remote_bit != base_bit;
    let local_changed = local_bit != base_bit;

    let mut r = Reconciled {
        set_status: None,
        push: None,
        final_state: remote_state,
        final_status: local_status,
        note: None,
    };

    if remote_changed && !local_changed {
        // Remote-only change → mirror in.
        if !opts.pull {
            r.final_state = base.last_state;
            return r;
        }
        match remote_state {
            IssueState::Closed => {
                if !local_status.is_terminal() {
                    r.set_status = Some(Status::Achieved);
                    r.final_status = Status::Achieved;
                    if local_status == Status::Converging {
                        r.note = Some(
                            "GitHub closed the issue while the target was in progress; \
                                  marked Achieved"
                                .to_string(),
                        );
                    }
                }
                r.final_state = IssueState::Closed;
            }
            IssueState::Open => {
                if local_status.is_terminal() {
                    r.set_status = Some(Status::Identified);
                    r.final_status = Status::Identified;
                }
                r.final_state = IssueState::Open;
            }
        }
    } else if local_changed && !remote_changed {
        // Local-only change → push out.
        if !opts.push {
            r.final_state = base.last_state;
            return r;
        }
        match local_status {
            Status::Achieved => {
                r.push = Some(PushKind::Close(CloseReason::Completed));
                r.final_state = IssueState::Closed;
            }
            Status::SetAside => {
                r.push = Some(PushKind::Close(CloseReason::NotPlanned));
                r.final_state = IssueState::Closed;
            }
            Status::Identified | Status::Converging => {
                r.push = Some(PushKind::Reopen);
                r.final_state = IssueState::Open;
            }
        }
    }
    // (true, true) ⇒ same bit ⇒ already converged; (false, false) ⇒ idle.
    r
}

/// Apply a [`Reconciled`] outcome for `number`/`id` onto the plan.
fn apply_reconciled(p: &mut Plan, id: &str, number: u64, local_status: Status, r: Reconciled) {
    if let Some(status) = r.set_status {
        p.target_ops.push(TargetOp::SetStatus {
            id: id.to_string(),
            status,
        });
        p.mirrored_in += 1;
    }
    if let Some(kind) = r.push {
        let comment = action_comment(local_status);
        let action = match kind {
            PushKind::Close(reason) => GhAction::Close {
                number,
                reason,
                comment,
            },
            PushKind::Reopen => GhAction::Reopen { number, comment },
        };
        p.gh_actions.push(action);
        p.pushed += 1;
    }
    if let Some(note) = r.note {
        p.notes.push(format!("{id} (issue #{number}): {note}"));
    }
    p.next_state.issues.insert(
        number,
        IssueRecord {
            last_status: r.final_status,
            last_state: r.final_state,
        },
    );
}

/// Compute the full sync plan. Pure: no I/O, no `gh`, no clock — every
/// branch of the reconcile matrix is exercisable from tests.
pub fn plan(
    repo: &str,
    issues: &[Issue],
    targets: &BTreeMap<String, Target>,
    prior: &SyncState,
    today: NaiveDate,
    opts: PlanOpts,
) -> Plan {
    let mut p = Plan {
        next_state: SyncState {
            repo: repo.to_string(),
            watermark: prior.watermark.clone(),
            issues: prior.issues.clone(),
        },
        ..Default::default()
    };

    let seen: BTreeSet<u64> = issues.iter().map(|i| i.number).collect();

    // Advance the watermark to the newest updatedAt observed.
    for issue in issues {
        if p.next_state
            .watermark
            .as_deref()
            .is_none_or(|w| issue.updated_at.as_str() > w)
        {
            p.next_state.watermark = Some(issue.updated_at.clone());
        }
    }

    // --- Issues present in the listing ---
    for issue in issues {
        let id = target_id(issue.number);
        let base = prior.issues.get(&issue.number);
        let existing = targets.get(&id);

        match existing {
            None => {
                // Untracked: mirror in only if open. Closed-and-untracked
                // issues we never adopted are ignored.
                if opts.pull && issue.state == IssueState::Open {
                    p.target_ops.push(TargetOp::Create {
                        id: id.clone(),
                        target: Box::new(new_target(repo, issue, today)),
                    });
                    p.created += 1;
                    p.next_state.issues.insert(
                        issue.number,
                        IssueRecord {
                            last_status: Status::Identified,
                            last_state: IssueState::Open,
                        },
                    );
                }
            }
            Some(t) => {
                // Field ownership: refresh GitHub-owned content (open only).
                if opts.pull && issue.state == IssueState::Open {
                    let new_ctx = mirror_context(issue);
                    if t.name != issue.title || t.context != new_ctx || t.tags != issue.labels {
                        p.target_ops.push(TargetOp::UpdateContent {
                            id: id.clone(),
                            name: issue.title.clone(),
                            context: new_ctx,
                            tags: issue.labels.clone(),
                        });
                        p.updated += 1;
                    }
                }
                // Reconcile lifecycle. Absent base ⇒ treat current as the
                // base so we don't spuriously push.
                let base_rec = base.cloned().unwrap_or(IssueRecord {
                    last_status: t.status,
                    last_state: issue.state,
                });
                let r = reconcile(issue.state, t.status, &base_rec, opts);
                apply_reconciled(&mut p, &id, issue.number, t.status, r);
            }
        }
    }

    // --- Tracked issues absent from the listing ---
    // With no active filter, absence means the issue was closed remotely.
    if opts.pull && !opts.filtered {
        for (&number, base) in &prior.issues {
            if seen.contains(&number) || base.last_state == IssueState::Closed {
                continue;
            }
            let id = target_id(number);
            let Some(t) = targets.get(&id) else {
                // Target was deleted locally; drop the stale record.
                p.next_state.issues.remove(&number);
                continue;
            };
            let r = reconcile(IssueState::Closed, t.status, base, opts);
            apply_reconciled(&mut p, &id, number, t.status, r);
        }
    }

    p
}

/// Apply `target_ops` to the locked target file. Returns the count
/// applied.
fn apply_target_ops(file: &mut TargetsFile, ops: &[TargetOp]) -> usize {
    let mut n = 0;
    for op in ops {
        match op {
            TargetOp::Create { id, target } => {
                file.targets.insert(id.clone(), (**target).clone());
                n += 1;
            }
            TargetOp::UpdateContent {
                id,
                name,
                context,
                tags,
            } => {
                if let Some(t) = file.targets.get_mut(id) {
                    t.name = name.clone();
                    t.context = context.clone();
                    t.tags = tags.clone();
                    n += 1;
                }
            }
            TargetOp::SetStatus { id, status } => {
                if let Some(t) = file.targets.get_mut(id) {
                    t.status = *status;
                    n += 1;
                }
            }
        }
    }
    n
}

/// Outcome summary printed to the user.
#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub repo: String,
    pub created: usize,
    pub updated: usize,
    pub mirrored_in: usize,
    pub pushed: usize,
    pub notes: Vec<String>,
    pub errors: Vec<String>,
    pub dry_run: bool,
}

impl SyncReport {
    /// One-line-plus-detail summary.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "github sync {}{}: {} created, {} updated, {} mirrored-in, {} pushed",
            self.repo,
            if self.dry_run { " (dry run)" } else { "" },
            self.created,
            self.updated,
            self.mirrored_in,
            self.pushed,
        );
        for n in &self.notes {
            s.push_str(&format!("\n  note: {n}"));
        }
        for e in &self.errors {
            s.push_str(&format!("\n  error: {e}"));
        }
        s
    }
}

/// Parsed arguments to `bullseye github sync`.
#[derive(Debug, Clone)]
pub struct GithubArgs {
    pub cwd: PathBuf,
    pub repo: Option<String>,
    pub label: Option<String>,
    pub assignee: Option<String>,
    pub pull: bool,
    pub push: bool,
    pub dry_run: bool,
}

/// Parse flags for the `github sync` subcommand.
pub fn parse_args(args: &[String]) -> Result<GithubArgs, String> {
    let mut rest = args;
    match rest.first().map(String::as_str) {
        Some("sync") => rest = &rest[1..],
        Some("--help") | Some("-h") | None => return Err("HELP".to_string()),
        Some(other) => return Err(format!("unknown github subcommand: {other}")),
    }

    let cwd = std::env::current_dir().map_err(|e| format!("cannot determine cwd: {e}"))?;
    let mut a = GithubArgs {
        cwd,
        repo: None,
        label: None,
        assignee: None,
        pull: true,
        push: true,
        dry_run: false,
    };

    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--repo" => a.repo = Some(take_value(rest, &mut i, "--repo")?),
            "--label" => a.label = Some(take_value(rest, &mut i, "--label")?),
            "--assignee" => a.assignee = Some(take_value(rest, &mut i, "--assignee")?),
            "--cwd" => a.cwd = PathBuf::from(take_value(rest, &mut i, "--cwd")?),
            "--pull-only" => {
                a.push = false;
                i += 1;
            }
            "--push-only" => {
                a.pull = false;
                i += 1;
            }
            "--dry-run" => {
                a.dry_run = true;
                i += 1;
            }
            "--help" | "-h" => return Err("HELP".to_string()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(a)
}

fn take_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    let v = args
        .get(*i + 1)
        .ok_or_else(|| format!("{flag} requires a value"))?
        .clone();
    *i += 2;
    Ok(v)
}

/// Run the sync against a concrete [`GhClient`]. Split from [`run`] so
/// tests can drive it with a fake client and an explicit `today`.
pub fn run_with(
    client: &dyn GhClient,
    args: &GithubArgs,
    today: NaiveDate,
) -> Result<SyncReport, String> {
    client.auth_status()?;

    let repo = match &args.repo {
        Some(r) => r.clone(),
        None => client.current_repo()?,
    };

    let yaml = store::discover_anywhere(&args.cwd).ok_or_else(|| {
        format!(
            "no bullseye.yaml found from {} — run bullseye_init first",
            args.cwd.display()
        )
    })?;
    let repo_root = yaml
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| args.cwd.clone());
    let state_path = SyncState::path_for(&repo_root);
    let prior = SyncState::load(&state_path)?;

    let filtered = args.label.is_some() || args.assignee.is_some();
    // The first run lists all open issues; later runs add recently-closed
    // ones (bounded by the watermark) so remote closes are detected.
    let list_opts = ListOpts {
        include_closed: prior.watermark.is_some(),
        since: prior.watermark.clone(),
        label: args.label.clone(),
        assignee: args.assignee.clone(),
    };
    let issues = client.list_issues(&repo, &list_opts)?;

    let targets = store::load(&yaml).map_err(|e| e.to_string())?.targets;
    let plan_opts = PlanOpts {
        pull: args.pull,
        push: args.push,
        filtered,
    };
    let p = plan(&repo, &issues, &targets, &prior, today, plan_opts);

    let mut report = SyncReport {
        repo: repo.clone(),
        created: p.created,
        updated: p.updated,
        mirrored_in: p.mirrored_in,
        pushed: p.pushed,
        notes: p.notes.clone(),
        dry_run: args.dry_run,
        ..Default::default()
    };

    if args.dry_run {
        return Ok(report);
    }

    // Push lifecycle actions first. A failure leaves that issue's base
    // un-advanced so the next run retries.
    let mut next_state = p.next_state.clone();
    for action in &p.gh_actions {
        let number = action.number();
        let res = match action {
            GhAction::Close {
                number,
                reason,
                comment,
            } => client.close_issue(&repo, *number, *reason, comment),
            GhAction::Reopen { number, comment } => client.reopen_issue(&repo, *number, comment),
        };
        if let Err(e) = res {
            report.errors.push(format!("issue #{number}: {e}"));
            match prior.issues.get(&number) {
                Some(rec) => {
                    next_state.issues.insert(number, rec.clone());
                }
                None => {
                    next_state.issues.remove(&number);
                }
            }
        }
    }

    // Apply target mutations under the store lock.
    if !p.target_ops.is_empty() {
        store::with_locked_mutation(&yaml, |file| {
            apply_target_ops(file, &p.target_ops);
            Ok::<(), String>(())
        })
        .map_err(|e| e.to_string())?;
    }

    next_state.save(&state_path)?;
    Ok(report)
}

/// Top-level entry point for the `bullseye github` subcommand.
pub fn run(args: &[String]) -> Result<String, String> {
    let parsed = match parse_args(args) {
        Ok(p) => p,
        Err(e) if e == "HELP" => return Ok(help_text()),
        Err(e) => return Err(e),
    };
    let today = chrono::Local::now().date_naive();
    let client = RealGh::new(parsed.cwd.clone());
    let report = run_with(&client, &parsed, today)?;
    Ok(report.summary())
}

/// Help text for `bullseye github`.
pub fn help_text() -> String {
    "USAGE:\n    \
     bullseye github sync [--repo owner/name] [--label L] [--assignee A]\n                         \
     [--pull-only | --push-only] [--dry-run] [--cwd DIR]\n\n\
     Mirror GitHub issues into bullseye targets and reflect target\n\
     lifecycle back to issues, using the `gh` CLI (which carries your\n\
     GitHub identity — no token is stored). Mirrored targets use\n\
     GH<issue-number> IDs.\n\n\
     FLAGS:\n    \
     --repo owner/name   Repo to sync (default: the current checkout)\n    \
     --label L           Only mirror issues with label L\n    \
     --assignee A        Only mirror issues assigned to A (@me for you)\n    \
     --pull-only         Mirror in only; do not push lifecycle to GitHub\n    \
     --push-only         Push lifecycle only; do not mirror in\n    \
     --dry-run           Show what would change without writing\n    \
     --cwd DIR           Discover bullseye.yaml from DIR (default: cwd)\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 21).unwrap()
    }

    fn issue(number: u64, title: &str, state: IssueState) -> Issue {
        Issue {
            number,
            title: title.to_string(),
            body: format!("body of #{number}"),
            labels: vec!["bug".to_string()],
            state,
            url: format!("https://github.com/o/r/issues/{number}"),
            updated_at: format!("2026-06-21T00:00:0{number}Z"),
        }
    }

    /// A mirrored target matching `issue`, at `status`.
    fn mirrored(issue: &Issue, status: Status) -> Target {
        let mut t = new_target("o/r", issue, date());
        t.status = status;
        t
    }

    fn rec(status: Status, state: IssueState) -> IssueRecord {
        IssueRecord {
            last_status: status,
            last_state: state,
        }
    }

    fn prior_with(number: u64, status: Status, state: IssueState) -> SyncState {
        let mut s = SyncState::default();
        s.issues.insert(number, rec(status, state));
        s
    }

    #[test]
    fn gh_issue_json_maps_real_shape() {
        // Pins the field mapping against `gh issue list --json …` output:
        // uppercase state, label objects, and the `updatedAt` rename.
        let json = r#"[{"number":12,"title":"T","body":"B",
            "labels":[{"name":"bug"},{"name":"p1"}],"state":"CLOSED",
            "url":"https://github.com/o/r/issues/12","updatedAt":"2026-06-01T00:00:00Z"}]"#;
        let parsed: Vec<GhIssueJson> = serde_json::from_str(json).unwrap();
        let issue: Issue = parsed.into_iter().next().unwrap().into();
        assert_eq!(issue.number, 12);
        assert_eq!(issue.labels, vec!["bug".to_string(), "p1".to_string()]);
        assert_eq!(issue.state, IssueState::Closed);
        assert_eq!(issue.updated_at, "2026-06-01T00:00:00Z");
    }

    #[test]
    fn first_pull_creates_gh_targets() {
        let issues = vec![issue(7, "Fix the thing", IssueState::Open)];
        let p = plan(
            "o/r",
            &issues,
            &BTreeMap::new(),
            &SyncState::default(),
            date(),
            PlanOpts::default(),
        );
        assert_eq!(p.created, 1);
        assert_eq!(p.target_ops.len(), 1);
        match &p.target_ops[0] {
            TargetOp::Create { id, target } => {
                assert_eq!(id, "GH7");
                assert_eq!(target.name, "Fix the thing");
                assert_eq!(target.origin, "github:o/r#7");
                assert_eq!(target.tags, vec!["bug".to_string()]);
                assert_eq!(target.status, Status::Identified);
                assert!(target.context.contains("issues/7"));
            }
            other => panic!("expected Create, got {other:?}"),
        }
        assert_eq!(
            p.next_state.issues[&7],
            rec(Status::Identified, IssueState::Open)
        );
        assert_eq!(
            p.next_state.watermark.as_deref(),
            Some("2026-06-21T00:00:07Z")
        );
    }

    #[test]
    fn re_pull_is_idempotent_no_duplicate() {
        let issues = vec![issue(7, "Fix the thing", IssueState::Open)];
        let mut targets = BTreeMap::new();
        targets.insert("GH7".to_string(), mirrored(&issues[0], Status::Identified));
        let prior = prior_with(7, Status::Identified, IssueState::Open);

        let p = plan(
            "o/r",
            &issues,
            &targets,
            &prior,
            date(),
            PlanOpts::default(),
        );
        assert_eq!(p.created, 0);
        assert_eq!(p.updated, 0, "no content change ⇒ no update op");
        assert!(p.target_ops.is_empty());
        assert!(p.gh_actions.is_empty());
    }

    #[test]
    fn content_change_updates_but_keeps_status() {
        let mut iss = issue(7, "Renamed title", IssueState::Open);
        iss.labels = vec!["bug".into(), "p1".into()];
        // Existing target reflects the OLD title.
        let old = issue(7, "Old title", IssueState::Open);
        let mut targets = BTreeMap::new();
        targets.insert("GH7".to_string(), mirrored(&old, Status::Converging));
        let prior = prior_with(7, Status::Converging, IssueState::Open);

        let p = plan("o/r", &[iss], &targets, &prior, date(), PlanOpts::default());
        assert_eq!(p.updated, 1);
        match &p.target_ops[0] {
            TargetOp::UpdateContent { id, name, tags, .. } => {
                assert_eq!(id, "GH7");
                assert_eq!(name, "Renamed title");
                assert_eq!(tags, &vec!["bug".to_string(), "p1".to_string()]);
            }
            other => panic!("expected UpdateContent, got {other:?}"),
        }
        assert!(
            !p.target_ops
                .iter()
                .any(|o| matches!(o, TargetOp::SetStatus { .. })),
            "status must not change on a content-only edit"
        );
    }

    #[test]
    fn local_achieved_pushes_close_and_records_closed_base() {
        let issues = vec![issue(7, "x", IssueState::Open)];
        let mut targets = BTreeMap::new();
        targets.insert("GH7".to_string(), mirrored(&issues[0], Status::Achieved));
        let prior = prior_with(7, Status::Identified, IssueState::Open);

        let p = plan(
            "o/r",
            &issues,
            &targets,
            &prior,
            date(),
            PlanOpts::default(),
        );
        assert_eq!(p.pushed, 1);
        assert_eq!(
            p.gh_actions[0],
            GhAction::Close {
                number: 7,
                reason: CloseReason::Completed,
                comment: action_comment(Status::Achieved),
            }
        );
        // Base advances to the intended post-push state.
        assert_eq!(
            p.next_state.issues[&7],
            rec(Status::Achieved, IssueState::Closed)
        );
    }

    #[test]
    fn set_aside_pushes_not_planned_close() {
        let issues = vec![issue(7, "x", IssueState::Open)];
        let mut t = mirrored(&issues[0], Status::SetAside);
        t.set_aside_reason = Some("wontfix".into());
        let mut targets = BTreeMap::new();
        targets.insert("GH7".to_string(), t);
        let prior = prior_with(7, Status::Identified, IssueState::Open);

        let p = plan(
            "o/r",
            &issues,
            &targets,
            &prior,
            date(),
            PlanOpts::default(),
        );
        assert!(matches!(
            p.gh_actions[0],
            GhAction::Close {
                number: 7,
                reason: CloseReason::NotPlanned,
                ..
            }
        ));
    }

    #[test]
    fn reverted_target_reopens_closed_issue() {
        // Base: closed/achieved. Issue still closed (absent). Local
        // reverted to Converging ⇒ local-only change ⇒ push reopen.
        let mut targets = BTreeMap::new();
        targets.insert("GH7".to_string(), {
            let mut t = mirrored(&issue(7, "x", IssueState::Open), Status::Converging);
            t.status = Status::Converging;
            t
        });
        let prior = prior_with(7, Status::Achieved, IssueState::Closed);

        // Issue is closed, so absent from the (unfiltered) open listing.
        let p = plan("o/r", &[], &targets, &prior, date(), PlanOpts::default());
        // Absent + base closed ⇒ skipped by the absent-loop (base closed),
        // so no push. The reopen must come from the listing instead: a
        // closed issue surfaced via include_closed.
        assert_eq!(p.pushed, 0);

        // Now surface the closed issue in the listing (include_closed run).
        let p2 = plan(
            "o/r",
            &[issue(7, "x", IssueState::Closed)],
            &targets,
            &prior,
            date(),
            PlanOpts::default(),
        );
        assert_eq!(p2.pushed, 1);
        assert!(matches!(
            p2.gh_actions[0],
            GhAction::Reopen { number: 7, .. }
        ));
        assert_eq!(p2.next_state.issues[&7].last_state, IssueState::Open);
    }

    #[test]
    fn remote_close_mirrors_in_when_local_unchanged() {
        // Issue absent from the (unfiltered) listing ⇒ closed remotely;
        // local target unchanged since base ⇒ adopt the close.
        let mut targets = BTreeMap::new();
        targets.insert(
            "GH7".to_string(),
            mirrored(&issue(7, "x", IssueState::Open), Status::Identified),
        );
        let prior = prior_with(7, Status::Identified, IssueState::Open);

        let p = plan("o/r", &[], &targets, &prior, date(), PlanOpts::default());
        assert_eq!(p.mirrored_in, 1);
        assert_eq!(
            p.target_ops[0],
            TargetOp::SetStatus {
                id: "GH7".to_string(),
                status: Status::Achieved,
            }
        );
        assert_eq!(p.next_state.issues[&7].last_state, IssueState::Closed);
        assert!(p.notes.is_empty(), "Identified → Achieved is unremarkable");
    }

    #[test]
    fn remote_close_of_in_progress_target_is_noted() {
        let mut targets = BTreeMap::new();
        targets.insert(
            "GH7".to_string(),
            mirrored(&issue(7, "x", IssueState::Open), Status::Converging),
        );
        let prior = prior_with(7, Status::Converging, IssueState::Open);

        let p = plan("o/r", &[], &targets, &prior, date(), PlanOpts::default());
        assert_eq!(p.mirrored_in, 1);
        assert_eq!(
            p.target_ops[0],
            TargetOp::SetStatus {
                id: "GH7".to_string(),
                status: Status::Achieved,
            }
        );
        assert_eq!(p.notes.len(), 1, "closing in-progress work is surfaced");
        assert!(p.notes[0].contains("in progress"));
    }

    #[test]
    fn filtered_absence_does_not_infer_close() {
        let mut targets = BTreeMap::new();
        targets.insert(
            "GH7".to_string(),
            mirrored(&issue(7, "x", IssueState::Open), Status::Identified),
        );
        let prior = prior_with(7, Status::Identified, IssueState::Open);

        let opts = PlanOpts {
            filtered: true,
            ..PlanOpts::default()
        };
        let p = plan("o/r", &[], &targets, &prior, date(), opts);
        assert_eq!(p.mirrored_in, 0);
        assert!(p.target_ops.is_empty());
    }

    #[test]
    fn round_trip_reaches_fixpoint() {
        let issues = vec![issue(7, "Fix", IssueState::Open)];
        let p1 = plan(
            "o/r",
            &issues,
            &BTreeMap::new(),
            &SyncState::default(),
            date(),
            PlanOpts::default(),
        );
        let mut targets = BTreeMap::new();
        for op in &p1.target_ops {
            if let TargetOp::Create { id, target } = op {
                targets.insert(id.clone(), (**target).clone());
            }
        }
        let p2 = plan(
            "o/r",
            &issues,
            &targets,
            &p1.next_state,
            date(),
            PlanOpts::default(),
        );
        assert!(p2.target_ops.is_empty(), "second pull must be a no-op");
        assert!(p2.gh_actions.is_empty());
        assert_eq!(p2.created, 0);
        assert_eq!(p2.updated, 0);
    }

    #[test]
    fn pull_only_suppresses_push() {
        let issues = vec![issue(7, "x", IssueState::Open)];
        let mut targets = BTreeMap::new();
        targets.insert("GH7".to_string(), mirrored(&issues[0], Status::Achieved));
        let prior = prior_with(7, Status::Identified, IssueState::Open);

        let opts = PlanOpts {
            push: false,
            ..PlanOpts::default()
        };
        let p = plan("o/r", &issues, &targets, &prior, date(), opts);
        assert_eq!(p.pushed, 0);
        assert!(p.gh_actions.is_empty());
    }

    // --- A fake client exercises run_with end to end. ---

    struct FakeGh {
        issues: Vec<Issue>,
        closed: RefCell<Vec<u64>>,
        reopened: RefCell<Vec<u64>>,
    }

    impl GhClient for FakeGh {
        fn auth_status(&self) -> Result<(), String> {
            Ok(())
        }
        fn current_repo(&self) -> Result<String, String> {
            Ok("o/r".to_string())
        }
        fn list_issues(&self, _repo: &str, _opts: &ListOpts) -> Result<Vec<Issue>, String> {
            Ok(self.issues.clone())
        }
        fn close_issue(
            &self,
            _repo: &str,
            number: u64,
            _reason: CloseReason,
            _comment: &str,
        ) -> Result<(), String> {
            self.closed.borrow_mut().push(number);
            Ok(())
        }
        fn reopen_issue(&self, _repo: &str, number: u64, _comment: &str) -> Result<(), String> {
            self.reopened.borrow_mut().push(number);
            Ok(())
        }
    }

    #[test]
    fn run_with_creates_targets_and_writes_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();
        std::fs::write(
            cwd.join("bullseye.yaml"),
            "schema_version: 5\ntargets: {}\n",
        )
        .unwrap();

        let client = FakeGh {
            issues: vec![issue(42, "Hello", IssueState::Open)],
            closed: RefCell::new(vec![]),
            reopened: RefCell::new(vec![]),
        };
        let args = GithubArgs {
            cwd: cwd.clone(),
            repo: Some("o/r".to_string()),
            label: None,
            assignee: None,
            pull: true,
            push: true,
            dry_run: false,
        };
        let report = run_with(&client, &args, date()).unwrap();
        assert_eq!(report.created, 1);

        let loaded = store::load(&cwd.join("bullseye.yaml")).unwrap();
        assert!(loaded.targets.contains_key("GH42"));
        assert_eq!(loaded.targets["GH42"].name, "Hello");

        let state = SyncState::load(&SyncState::path_for(&cwd)).unwrap();
        assert!(state.issues.contains_key(&42));
        assert_eq!(state.repo, "o/r");
    }

    #[test]
    fn run_with_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();
        std::fs::write(
            cwd.join("bullseye.yaml"),
            "schema_version: 5\ntargets: {}\n",
        )
        .unwrap();

        let client = FakeGh {
            issues: vec![issue(42, "Hello", IssueState::Open)],
            closed: RefCell::new(vec![]),
            reopened: RefCell::new(vec![]),
        };
        let args = GithubArgs {
            cwd: cwd.clone(),
            repo: Some("o/r".to_string()),
            label: None,
            assignee: None,
            pull: true,
            push: true,
            dry_run: true,
        };
        let report = run_with(&client, &args, date()).unwrap();
        assert_eq!(report.created, 1);
        // Nothing persisted.
        let loaded = store::load(&cwd.join("bullseye.yaml")).unwrap();
        assert!(loaded.targets.is_empty());
        assert!(!SyncState::path_for(&cwd).exists());
    }

    #[test]
    fn parse_args_sync_defaults_and_flags() {
        let a = parse_args(&["sync".into()]).unwrap();
        assert!(a.pull && a.push && !a.dry_run);

        let a = parse_args(&[
            "sync".into(),
            "--repo".into(),
            "o/r".into(),
            "--label".into(),
            "bug".into(),
            "--dry-run".into(),
            "--pull-only".into(),
        ])
        .unwrap();
        assert_eq!(a.repo.as_deref(), Some("o/r"));
        assert_eq!(a.label.as_deref(), Some("bug"));
        assert!(a.dry_run);
        assert!(a.pull && !a.push);
    }

    #[test]
    fn parse_args_unknown_and_help() {
        assert_eq!(
            parse_args(&["bogus".into()]).unwrap_err(),
            "unknown github subcommand: bogus"
        );
        assert_eq!(
            parse_args(&["sync".into(), "--nope".into()]).unwrap_err(),
            "unknown argument: --nope"
        );
        assert_eq!(parse_args(&["--help".into()]).unwrap_err(), "HELP");
    }
}
