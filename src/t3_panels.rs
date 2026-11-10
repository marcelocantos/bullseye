// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Ledger content for the T3 panel endpoint (🎯T114, 🎯T116).
//!
//! [`crate::t3_plugin`] owns the wire and the socket; this module turns
//! ledgers into what T3 renders — panel rows with badges and actions,
//! hover cards and dialogs — and turns T3's row actions into ledger
//! mutations.
//!
//! Every write goes through [`crate::handler::handle_commit`] or
//! [`crate::handler::handle_apply`], built from the same JSON arguments
//! an MCP `bullseye_commit` / `bullseye_apply` call carries, so locking,
//! CAS retries, validation, the evidence policy and repo guards are the
//! ones every other writer gets. Reads use
//! [`store::discover_anywhere_readonly`], which never moves a ledger.
//!
//! The T3 vocabulary maps onto existing ledger state rather than adding
//! any: *defer until* is `postponed_until`, *park* is a postponement
//! with only a predicate ([`PARK_PREDICATE`]), *abandon* is set_aside
//! with a reason, and *reopen* returns a set-aside target to identified.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Datelike, NaiveDate};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::graph::{frontier_tolerant, open_blockers, rank_frontier, unblocking_fanout};
use crate::schema::{Status, Target, TargetsFile};
use crate::store;
use crate::t3_plugin::{
    ActionValue, FRAME_MARGIN_BYTES, MAX_FRAME_BYTES, MAX_ID_UNITS, MAX_TEXT_UNITS, Project,
    Request, RequestKind, Scope, clip, line, utf16_len,
};

/// The thread-scoped panel: the ledger of the thread's workspace.
pub const PANEL_ID: &str = "targets";
/// The environment-scoped panel: every project's targets by due date.
pub const ENV_PANEL_ID: &str = "due";
/// The postpone predicate T3's *Park* writes. Any predicate parks a
/// target (the frontier holds it until `op=wake`); this one says who.
pub const PARK_PREDICATE: &str = "Parked in T3 Code";

/// T3 contract: rows per panel.
const MAX_ROWS: usize = 2000;
/// T3 contract: projects a row may navigate to.
const MAX_NAVIGATE_PROJECTS: usize = 16;
/// T3 contract: items in one surface list.
const MAX_LIST_ITEMS: usize = 500;
/// The target name is quoted into the agent prompt; a whole name can be
/// a paragraph, and every row carries the prompt.
const PROMPT_NAME_UNITS: usize = 200;
/// Hex digits of a ledger path's digest in environment row ids.
const LEDGER_KEY_HEX: usize = 8;
/// Longest ledger name shown in an environment row label.
const LEDGER_NAME_UNITS: usize = 80;
/// Card width for the details and dependencies surfaces.
const SURFACE_WIDTH: u64 = 420;

const SURFACE_DETAILS: &str = "details";
const SURFACE_DEPENDENCIES: &str = "dependencies";
const SURFACE_ABANDON: &str = "abandon";

const ACTION_WORK: &str = "work";
const ACTION_DUE: &str = "due";
const ACTION_DEFER: &str = "defer";
const ACTION_PARK: &str = "park";
const ACTION_UNPARK: &str = "unpark";
const ACTION_ABANDON: &str = "abandon";
const ACTION_REOPEN: &str = "reopen";
const ACTION_DEPENDENCIES: &str = "dependencies";

const ABANDON_REASON_FIELD: &str = "reason";
const REOPEN_REASON: &str = "Reopened from T3 Code";

/// The view a thread subscription asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFilter {
    /// Unblocked active targets, in repo-level frontier order.
    Frontier,
    /// Every non-terminal target.
    Active,
    /// Every target, active ones first.
    All,
}

impl PanelFilter {
    const ALL: [PanelFilter; 3] = [PanelFilter::Frontier, PanelFilter::Active, PanelFilter::All];

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }

    fn id(self) -> &'static str {
        match self {
            PanelFilter::Frontier => "frontier",
            PanelFilter::Active => "active",
            PanelFilter::All => "all",
        }
    }

    fn label(self) -> &'static str {
        match self {
            PanelFilter::Frontier => "Frontier",
            PanelFilter::Active => "Active",
            PanelFilter::All => "All",
        }
    }
}

/// The view an environment subscription asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvFilter {
    /// Actionable now: not deferred to a later day and not parked,
    /// except that an overdue target always shows.
    Upcoming,
    /// Deferred to a later day, or parked.
    Deferred,
    /// Every active target.
    All,
}

impl EnvFilter {
    const ALL: [EnvFilter; 3] = [EnvFilter::Upcoming, EnvFilter::Deferred, EnvFilter::All];

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }

    fn id(self) -> &'static str {
        match self {
            EnvFilter::Upcoming => "upcoming",
            EnvFilter::Deferred => "deferred",
            EnvFilter::All => "all",
        }
    }

    fn label(self) -> &'static str {
        match self {
            EnvFilter::Upcoming => "Upcoming",
            EnvFilter::Deferred => "Deferred",
            EnvFilter::All => "All",
        }
    }
}

/// Natural ordering key: `T2` before `T10`, `T1` before `T1.1`.
fn id_key(id: &str) -> Vec<u64> {
    let mut runs = Vec::new();
    let mut current: Option<u64> = None;
    for c in id.chars() {
        match (c.to_digit(10), current) {
            (Some(d), Some(n)) => {
                current = Some(n.saturating_mul(10).saturating_add(u64::from(d)));
            }
            (Some(d), None) => current = Some(u64::from(d)),
            (None, Some(n)) => {
                runs.push(n);
                current = None;
            }
            (None, None) => {}
        }
    }
    runs.extend(current);
    runs
}

/// Multi-line ledger prose, bounded: newlines survive, other control
/// characters do not.
fn prose(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    clip(flat.trim(), MAX_TEXT_UNITS)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && utf16_len(id) <= MAX_ID_UNITS
}

pub(crate) struct Row {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    /// Badges, actions, hover surface and navigation, as wire JSON.
    pub extra: Map<String, Value>,
}

impl Row {
    fn notice(id: &str, label: &str, detail: Option<String>) -> Row {
        Row {
            id: id.to_string(),
            label: line(label, MAX_TEXT_UNITS),
            detail,
            extra: Map::new(),
        }
    }
}

fn status_word(status: Status) -> &'static str {
    match status {
        Status::Identified => "identified",
        Status::Converging => "converging",
        Status::Achieved => "achieved",
        Status::SetAside => "set aside",
    }
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Identified => "Identified",
        Status::Converging => "Converging",
        Status::Achieved => "Achieved",
        Status::SetAside => "Abandoned",
    }
}

fn status_tone(status: Status) -> &'static str {
    match status {
        Status::Identified => "default",
        Status::Converging => "info",
        Status::Achieved => "success",
        Status::SetAside => "muted",
    }
}

fn badge(label: &str, tone: &str) -> Value {
    json!({"label": clip(label, 128), "tone": tone})
}

fn status_badge(status: Status) -> Value {
    badge(status_label(status), status_tone(status))
}

fn parked(t: &Target) -> bool {
    t.postpone_predicate
        .as_deref()
        .is_some_and(|p| !p.trim().is_empty())
}

/// The future day a target is deferred to; a passed date defers nothing.
fn deferred_to(t: &Target, today: NaiveDate) -> Option<NaiveDate> {
    t.postponed_until.filter(|d| *d > today)
}

fn day(d: NaiveDate, today: NaiveDate) -> String {
    if d.year() == today.year() {
        d.format("%-d %b").to_string()
    } else {
        d.format("%-d %b %Y").to_string()
    }
}

/// Badge text and tone for a due date: overdue is danger, today and
/// tomorrow are warnings, anything later is plain.
fn due_badge(due: NaiveDate, today: NaiveDate) -> Value {
    match (due - today).num_days() {
        n if n < 0 => badge(&format!("Overdue {}", day(due, today)), "danger"),
        0 => badge("Due today", "warning"),
        1 => badge("Due tomorrow", "warning"),
        _ => badge(&format!("Due {}", day(due, today)), "default"),
    }
}

fn badges(t: &Target, today: NaiveDate) -> Vec<Value> {
    let mut out = Vec::new();
    if t.status != Status::Identified {
        out.push(status_badge(t.status));
    }
    if t.is_active() {
        if let Some(due) = t.due {
            out.push(due_badge(due, today));
        }
        if let Some(until) = deferred_to(t, today) {
            out.push(badge(
                &format!("Deferred to {}", day(until, today)),
                "muted",
            ));
        }
        if parked(t) {
            out.push(badge("Parked", "muted"));
        }
    }
    out
}

/// The row's actions and its primary action. `agent` is false in the
/// environment panel, where T3 has no thread to hand a prompt to.
fn actions(id: &str, t: &Target, agent: bool) -> (Vec<Value>, Option<&'static str>) {
    let mut out = Vec::new();
    let mut primary = None;
    if t.is_active() {
        if agent && !parked(t) {
            let name = line(&t.name, PROMPT_NAME_UNITS);
            out.push(json!({
                "kind": "agent", "id": ACTION_WORK, "label": "Work on this target",
                "prompt": format!(
                    "Work on 🎯{id}: {name}. Use the Bullseye tools to track progress."
                ),
            }));
            primary = Some(ACTION_WORK);
        }
        let mut due = json!({"kind": "date", "id": ACTION_DUE, "label": "Due date"});
        if let Some(d) = t.due {
            due["value"] = json!(d.to_string());
        }
        out.push(due);
        let mut defer = json!({
            "kind": "date", "id": ACTION_DEFER, "label": "Defer until",
            "clearLabel": "Clear deferral",
        });
        if let Some(d) = t.postponed_until {
            defer["value"] = json!(d.to_string());
        }
        out.push(defer);
        out.push(if parked(t) {
            json!({"kind": "plugin", "id": ACTION_UNPARK, "label": "Unpark"})
        } else {
            json!({"kind": "plugin", "id": ACTION_PARK, "label": "Park"})
        });
        out.push(json!({
            "kind": "surface", "id": ACTION_ABANDON, "label": "Abandon",
            "surfaceId": SURFACE_ABANDON, "modal": true,
        }));
    } else if t.status == Status::SetAside {
        out.push(json!({"kind": "plugin", "id": ACTION_REOPEN, "label": "Reopen"}));
    }
    out.push(json!({
        "kind": "surface", "id": ACTION_DEPENDENCIES, "label": "Show dependencies",
        "surfaceId": SURFACE_DEPENDENCIES,
    }));
    (out, primary)
}

fn row(
    file: &TargetsFile,
    id: &str,
    target: &Target,
    fanout: Option<usize>,
    today: NaiveDate,
    agent: bool,
) -> Option<Row> {
    // T3 rejects the whole snapshot for an over-long id, so one
    // hand-edited absurd id must cost one row, not the panel.
    if !valid_id(id) {
        return None;
    }
    let mut parts = vec![status_word(target.status).to_string()];
    if let Some(n) = fanout.filter(|n| *n > 0) {
        parts.push(format!("unblocks {n}"));
    }
    if target.is_active() {
        let blockers = open_blockers(file, target);
        if !blockers.is_empty() {
            parts.push(format!("blocked by {}", blockers.join(", ")));
        }
        if let Some(owner) = &target.owned_by {
            parts.push(format!("owned by {}", owner.owner));
        }
    } else if let Some(reason) = &target.set_aside_reason {
        parts.push(reason.clone());
    }
    let mut extra = Map::new();
    let badges = badges(target, today);
    if !badges.is_empty() {
        extra.insert("badges".into(), Value::Array(badges));
    }
    let (actions, primary) = actions(id, target, agent);
    extra.insert("actions".into(), Value::Array(actions));
    if let Some(p) = primary {
        extra.insert("primaryActionId".into(), json!(p));
    }
    extra.insert("hoverSurfaceId".into(), json!(SURFACE_DETAILS));
    Some(Row {
        id: id.to_string(),
        label: line(&format!("🎯{id} {}", target.name), MAX_TEXT_UNITS),
        detail: Some(line(&parts.join(" · "), MAX_TEXT_UNITS)),
        extra,
    })
}

pub(crate) fn rows_for(file: &TargetsFile, filter: PanelFilter, today: NaiveDate) -> Vec<Row> {
    match filter {
        PanelFilter::Frontier => {
            let ready = frontier_tolerant(file).targets;
            rank_frontier(file, &ready)
                .into_iter()
                .filter_map(|r| {
                    let target = file.targets.get(&r.target.id)?;
                    row(file, &r.target.id, target, Some(r.fanout), today, true)
                })
                .collect()
        }
        PanelFilter::Active | PanelFilter::All => {
            let mut ids: Vec<&String> = file
                .targets
                .iter()
                .filter(|(_, t)| filter == PanelFilter::All || t.is_active())
                .map(|(id, _)| id)
                .collect();
            // Active first, then natural id order.
            ids.sort_by(|a, b| {
                let rank = |id: &String| (!file.targets[id].is_active(), id_key(id));
                rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
            });
            ids.into_iter()
                .filter_map(|id| {
                    let target = &file.targets[id];
                    let fanout = target.is_active().then(|| unblocking_fanout(file, id));
                    row(file, id, target, fanout, today, true)
                })
                .collect()
        }
    }
}

fn row_json(r: &Row) -> Value {
    let mut obj = Map::new();
    obj.insert("id".into(), json!(r.id));
    obj.insert("label".into(), json!(r.label));
    if let Some(d) = r.detail.as_deref().filter(|d| !d.is_empty()) {
        obj.insert("detail".into(), json!(d));
    }
    obj.extend(r.extra.clone());
    Value::Object(obj)
}

/// A panel holding `rows`, truncated with a "more" marker rather than
/// exceeding T3's row or frame limits.
pub(crate) fn panel_json(
    id: &str,
    title: &str,
    rows: Vec<Row>,
    filters: &[(&str, &str)],
    filter: &str,
) -> Value {
    // Reserve the last slot and the byte margin for a "more" marker, so
    // an oversized ledger degrades to a truncated list instead of a frame
    // T3 would reject.
    let mut budget = MAX_FRAME_BYTES - FRAME_MARGIN_BYTES;
    let total = rows.len();
    let cap = if total > MAX_ROWS {
        MAX_ROWS - 1
    } else {
        MAX_ROWS
    };
    let mut shown = Vec::new();
    for r in &rows {
        let value = row_json(r);
        let cost = value.to_string().len() + 1;
        if shown.len() >= cap || cost > budget {
            break;
        }
        budget -= cost;
        shown.push(value);
    }
    if shown.len() < total {
        shown.push(json!({
            "id": "more",
            "label": format!("… {} more not shown", total - shown.len()),
        }));
    }
    json!({
        "kind": "list",
        "id": id,
        "title": title,
        "rows": shown,
        "filters": filters
            .iter()
            .map(|(id, label)| json!({"id": id, "label": label}))
            .collect::<Vec<_>>(),
        "filter": filter,
    })
}

pub(crate) type LedgerRead = Result<Arc<TargetsFile>, String>;

fn read<'a>(ledgers: &'a mut HashMap<PathBuf, LedgerRead>, path: &Path) -> &'a LedgerRead {
    ledgers
        .entry(path.to_path_buf())
        .or_insert_with(|| store::load(path).map(Arc::new).map_err(|e| e.to_string()))
}

fn unreadable(id: &str, label: &str, error: &str) -> Row {
    Row::notice(id, label, Some(line(error, MAX_TEXT_UNITS)))
}

/// Panels for one workspace. A directory without a ledger has nothing
/// to show, so it yields no panel at all (T3: an empty array removes
/// the section).
pub(crate) fn thread_panels(
    cwd: &Path,
    filter: PanelFilter,
    today: NaiveDate,
    ledgers: &mut HashMap<PathBuf, LedgerRead>,
) -> Vec<Value> {
    let Some(path) = store::discover_anywhere_readonly(cwd) else {
        return Vec::new();
    };
    let rows = match read(ledgers, &path) {
        Ok(file) => rows_for(file, filter, today),
        Err(e) => vec![unreadable("error", "Ledger unreadable", e)],
    };
    let filters: Vec<(&str, &str)> = PanelFilter::ALL
        .iter()
        .map(|f| (f.id(), f.label()))
        .collect();
    vec![panel_json(
        PANEL_ID,
        "Bullseye",
        rows,
        &filters,
        filter.id(),
    )]
}

/// The ledger each project resolves to, deduplicated: several projects
/// (clones, worktrees, nested directories) can share one ledger, which
/// must then contribute its targets once. First-seen order.
fn ledger_groups(projects: &[Project]) -> Vec<(PathBuf, Vec<&Project>)> {
    let mut groups: Vec<(PathBuf, Vec<&Project>)> = Vec::new();
    for project in projects {
        let Some(found) = store::discover_anywhere_readonly(&project.cwd) else {
            continue;
        };
        // One file reached through different spellings is one ledger.
        let path = std::fs::canonicalize(&found).unwrap_or(found);
        match groups.iter_mut().find(|(p, _)| *p == path) {
            Some((_, members)) => members.push(project),
            None => groups.push((path, vec![project])),
        }
    }
    groups
}

/// A short, stable name for a ledger path, used to make environment row
/// ids unique across ledgers and to route their actions back.
fn ledger_key(path: &Path) -> String {
    let digest = Sha256::digest(path.as_os_str().as_encoded_bytes());
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..LEDGER_KEY_HEX]
        .to_string()
}

fn ledger_name(members: &[&Project]) -> String {
    let mut names: Vec<String> = Vec::new();
    for p in members {
        let title = p.title.trim();
        let name = if title.is_empty() {
            p.cwd
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.cwd.display().to_string())
        } else {
            title.to_string()
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    line(&names.join(", "), LEDGER_NAME_UNITS)
}

/// Whether an environment view shows an active target.
fn env_includes(filter: EnvFilter, t: &Target, today: NaiveDate) -> bool {
    let held = deferred_to(t, today).is_some() || parked(t);
    match filter {
        EnvFilter::Upcoming => !held || t.due.is_some_and(|d| d < today),
        EnvFilter::Deferred => held,
        EnvFilter::All => true,
    }
}

/// Sort key: overdue dates sort before today's, so ascending due order
/// already puts overdue first; undated targets go last. The deferred
/// view leads with the day each target comes back.
type EnvKey = (
    bool,
    Option<NaiveDate>,
    bool,
    Option<NaiveDate>,
    String,
    Vec<u64>,
);

fn env_key(filter: EnvFilter, t: &Target, today: NaiveDate, ledger: &str, id: &str) -> EnvKey {
    let wake = match filter {
        EnvFilter::Deferred => deferred_to(t, today),
        _ => None,
    };
    let leads_with_wake = filter == EnvFilter::Deferred;
    (
        leads_with_wake && wake.is_none(),
        wake,
        t.due.is_none(),
        t.due,
        ledger.to_string(),
        id_key(id),
    )
}

/// The environment panel over every project's ledger.
pub(crate) fn environment_panels(
    projects: &[Project],
    filter: EnvFilter,
    today: NaiveDate,
    ledgers: &mut HashMap<PathBuf, LedgerRead>,
) -> Vec<Value> {
    let groups = ledger_groups(projects);
    if groups.is_empty() {
        return Vec::new();
    }
    let mut problems = Vec::new();
    let mut keyed: Vec<(EnvKey, String, Row)> = Vec::new();
    for (path, members) in &groups {
        let key = ledger_key(path);
        let name = ledger_name(members);
        let file = match read(ledgers, path) {
            Ok(file) => Arc::clone(file),
            Err(e) => {
                problems.push(unreadable(
                    &format!("error:{key}"),
                    &format!("{name}: ledger unreadable"),
                    e,
                ));
                continue;
            }
        };
        let project_ids: Vec<&str> = members
            .iter()
            .take(MAX_NAVIGATE_PROJECTS)
            .map(|p| p.id.as_str())
            .collect();
        for (id, target) in &file.targets {
            if !target.is_active() || !env_includes(filter, target, today) {
                continue;
            }
            let row_id = format!("{key}:{id}");
            if !valid_id(&row_id) {
                continue;
            }
            let fanout = Some(unblocking_fanout(&file, id));
            let Some(mut r) = row(&file, id, target, fanout, today, false) else {
                continue;
            };
            r.label = line(&format!("{name} · 🎯{id} {}", target.name), MAX_TEXT_UNITS);
            r.id = row_id.clone();
            r.extra
                .insert("navigate".into(), json!({"projectIds": project_ids}));
            keyed.push((env_key(filter, target, today, &name, id), row_id, r));
        }
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let rows = problems
        .into_iter()
        .chain(keyed.into_iter().map(|(_, _, r)| r))
        .collect();
    let filters: Vec<(&str, &str)> = EnvFilter::ALL.iter().map(|f| (f.id(), f.label())).collect();
    vec![panel_json(
        ENV_PANEL_ID,
        "Targets by due date",
        rows,
        &filters,
        filter.id(),
    )]
}

// ---- Surfaces and actions ----------------------------------------------

/// A successful answer to an action or surface request.
#[derive(Debug, PartialEq)]
pub struct Reply {
    pub surface: Option<Value>,
    pub close: bool,
    /// The ledger was written; subscriptions should be refreshed now.
    pub mutated: bool,
}

/// The ledger and target a request's row names.
struct Located {
    /// A directory whose discovery reaches the ledger, for the write path.
    cwd: PathBuf,
    file: TargetsFile,
    target: String,
}

fn locate(req: &Request) -> Result<Located, String> {
    let (cwd, path, target) = match &req.scope {
        Scope::Thread { cwd } => {
            if req.panel_id != PANEL_ID {
                return Err(format!("Unknown panel `{}`.", clip(&req.panel_id, 64)));
            }
            let path = store::discover_anywhere_readonly(cwd)
                .ok_or("This workspace has no Bullseye ledger.")?;
            (cwd.clone(), path, req.row_id.clone())
        }
        Scope::Environment { projects } => {
            if req.panel_id != ENV_PANEL_ID {
                return Err(format!("Unknown panel `{}`.", clip(&req.panel_id, 64)));
            }
            let (key, target) = req
                .row_id
                .split_once(':')
                .ok_or_else(|| format!("Unknown row `{}`.", clip(&req.row_id, 64)))?;
            let (path, members) = ledger_groups(projects)
                .into_iter()
                .find(|(path, _)| ledger_key(path) == key)
                .ok_or("That target's ledger is not in any project of this environment.")?;
            (members[0].cwd.clone(), path, target.to_string())
        }
    };
    let file = store::load(&path).map_err(|e| format!("The ledger could not be read: {e}"))?;
    if !file.targets.contains_key(&target) {
        return Err(format!("Unknown target `{}`.", clip(&target, 64)));
    }
    Ok(Located { cwd, file, target })
}

/// Answer one action or surface request. Blocking: reads ledgers and
/// may write one.
pub fn handle_request(req: &Request, today: NaiveDate) -> Result<Reply, String> {
    let at = locate(req)?;
    match &req.kind {
        RequestKind::Surface {
            surface_id,
            item_id,
        } => {
            // A list item asks for the same card about another target.
            let subject = item_id.as_deref().unwrap_or(&at.target);
            let surface = match surface_id.as_str() {
                SURFACE_DETAILS => details_surface(&at.file, subject, today)?,
                SURFACE_DEPENDENCIES => dependencies_surface(&at.file, subject)?,
                SURFACE_ABANDON => abandon_surface(&at.file, subject)?,
                other => return Err(format!("Unknown surface `{}`.", clip(other, 64))),
            };
            Ok(Reply {
                surface: Some(surface),
                close: false,
                mutated: false,
            })
        }
        RequestKind::Action {
            action_id, value, ..
        } => {
            let close = act(&at, action_id, value)?;
            Ok(Reply {
                surface: None,
                close,
                mutated: true,
            })
        }
    }
}

fn subject<'a>(file: &'a TargetsFile, id: &str) -> Result<&'a Target, String> {
    file.targets
        .get(id)
        .ok_or_else(|| format!("Unknown target `{}`.", clip(id, 64)))
}

fn title(id: &str, t: &Target) -> String {
    line(&format!("🎯{id} {}", t.name), MAX_TEXT_UNITS)
}

/// A list block, bounded by T3's item limit and a byte budget shared
/// across the surface, ending in a "more" item when cut short.
fn list_block(items: Vec<Value>, budget: &mut usize) -> Value {
    let total = items.len();
    let mut shown = Vec::new();
    for item in items {
        let cost = item.to_string().len() + 1;
        if shown.len() + 1 >= MAX_LIST_ITEMS || cost > *budget {
            break;
        }
        *budget -= cost;
        shown.push(item);
    }
    if shown.len() < total {
        shown.push(json!({"id": "more", "label": format!("… {} more", total - shown.len())}));
    }
    json!({"type": "list", "items": shown})
}

/// List items for target ids, each opening that target's details card.
fn target_items(file: &TargetsFile, ids: &[&str]) -> Vec<Value> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for id in ids {
        if !valid_id(id) || seen.contains(id) {
            continue;
        }
        seen.push(*id);
        out.push(match file.targets.get(*id) {
            Some(t) => json!({
                "id": id, "label": title(id, t),
                "badges": [status_badge(t.status)], "surfaceId": SURFACE_DETAILS,
            }),
            None => json!({"id": id, "label": format!("🎯{id} (not in this ledger)")}),
        });
    }
    out
}

fn surface_budget() -> usize {
    MAX_FRAME_BYTES - FRAME_MARGIN_BYTES
}

fn details_surface(file: &TargetsFile, id: &str, today: NaiveDate) -> Result<Value, String> {
    let t = subject(file, id)?;
    let mut budget = surface_budget();
    let field = |label: &str, value: String, tone: &str| json!({"label": label, "value": line(&value, MAX_TEXT_UNITS), "tone": tone});
    let mut fields = vec![field(
        "Status",
        status_label(t.status).to_string(),
        status_tone(t.status),
    )];
    if let Some(due) = t.due {
        let tone = if t.is_active() {
            due_badge(due, today)["tone"]
                .as_str()
                .unwrap_or("default")
                .to_string()
        } else {
            "default".to_string()
        };
        fields.push(field("Due", day(due, today), &tone));
    }
    if let Some(until) = t.postponed_until {
        fields.push(field("Deferred until", day(until, today), "muted"));
    }
    if let Some(predicate) = t.postpone_predicate.as_deref().filter(|_| parked(t)) {
        fields.push(field("Parked", predicate.to_string(), "muted"));
    }
    if let Some(owner) = &t.owned_by {
        fields.push(field("Owned by", owner.owner.clone(), "default"));
    }
    if let Some(reason) = &t.set_aside_reason {
        fields.push(field("Abandoned because", reason.clone(), "muted"));
    }
    if t.is_active() {
        let blockers = open_blockers(file, t);
        if !blockers.is_empty() {
            fields.push(field("Blocked by", blockers.join(", "), "warning"));
        }
    }
    let mut blocks = vec![json!({"type": "fields", "items": fields})];
    if !t.acceptance.is_empty() {
        blocks.push(json!({"type": "heading", "text": "Acceptance"}));
        let items = t
            .acceptance
            .iter()
            .enumerate()
            .map(
                |(i, a)| json!({"id": format!("acceptance-{i}"), "label": line(a, MAX_TEXT_UNITS)}),
            )
            .collect();
        blocks.push(list_block(items, &mut budget));
    }
    if !t.context.trim().is_empty() {
        blocks.push(json!({"type": "heading", "text": "Context"}));
        blocks.push(json!({"type": "text", "text": prose(&t.context)}));
    }
    if !t.depends_on.is_empty() {
        blocks.push(json!({"type": "heading", "text": "Dependencies"}));
        let deps: Vec<&str> = t.depends_on.iter().map(String::as_str).collect();
        blocks.push(list_block(target_items(file, &deps), &mut budget));
    }
    Ok(json!({"title": title(id, t), "blocks": blocks, "width": SURFACE_WIDTH}))
}

fn dependencies_surface(file: &TargetsFile, id: &str) -> Result<Value, String> {
    let t = subject(file, id)?;
    let mut budget = surface_budget();
    let deps: Vec<&str> = t.depends_on.iter().map(String::as_str).collect();
    let mut dependents: Vec<&str> = file
        .targets
        .iter()
        .filter(|(_, other)| other.depends_on.iter().any(|d| d == id))
        .map(|(other, _)| other.as_str())
        .collect();
    dependents.sort_by(|a, b| id_key(a).cmp(&id_key(b)).then_with(|| a.cmp(b)));
    let mut blocks = Vec::new();
    for (heading, ids) in [("Depends on", deps), ("Needed by", dependents)] {
        blocks.push(json!({"type": "heading", "text": heading}));
        let items = target_items(file, &ids);
        blocks.push(if items.is_empty() {
            json!({"type": "text", "text": "None", "tone": "muted"})
        } else {
            list_block(items, &mut budget)
        });
    }
    Ok(json!({
        "title": line(&format!("Dependencies of 🎯{id}"), MAX_TEXT_UNITS),
        "blocks": blocks,
        "width": SURFACE_WIDTH,
    }))
}

fn abandon_surface(file: &TargetsFile, id: &str) -> Result<Value, String> {
    let t = subject(file, id)?;
    if !t.is_active() {
        return Err(format!("🎯{id} is already {}.", status_word(t.status)));
    }
    Ok(json!({
        "title": line(&format!("Abandon 🎯{id}"), MAX_TEXT_UNITS),
        "blocks": [
            {"type": "text", "text": title(id, t)},
            {"type": "text", "tone": "muted", "text":
                "The target is set aside with your reason. Targets that depend on it are no \
                 longer blocked by it. You can reopen it later."},
            {"type": "form", "id": "abandon", "actionId": ACTION_ABANDON, "submitLabel": "Abandon",
             "fields": [{
                "id": ABANDON_REASON_FIELD, "label": "Reason", "kind": "textarea",
                "required": true, "placeholder": "Why is this target no longer worth pursuing?",
             }]},
        ],
    }))
}

/// A `date` action's value: a calendar date, or `None` to clear.
fn date_value(value: &ActionValue) -> Result<Option<NaiveDate>, String> {
    match value {
        ActionValue::None => Ok(None),
        ActionValue::Text(s) => {
            let s = s.trim();
            match NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                Ok(d) if s.len() == 10 => Ok(Some(d)),
                _ => Err(format!("`{}` is not a YYYY-MM-DD date.", clip(s, 32))),
            }
        }
        ActionValue::Fields(_) => Err("This action takes a date.".into()),
    }
}

/// The human sentence of a coded tool error (`code=…\nmessage: …`).
fn human(e: rust_mcp_sdk::schema::schema_utils::CallToolError) -> String {
    let text = e.to_string();
    let message = text
        .split_once("message: ")
        .map_or(text.as_str(), |(_, m)| m);
    line(message, MAX_TEXT_UNITS)
}

/// Run a `bullseye_commit` call with these arguments, exactly as MCP would.
fn commit(cwd: &Path, mut args: Value) -> Result<(), String> {
    args["cwd"] = json!(cwd.to_string_lossy());
    let tool: crate::tools::CommitTool =
        serde_json::from_value(args).map_err(|e| format!("internal error: {e}"))?;
    crate::handler::handle_commit(tool).map(drop).map_err(human)
}

/// Perform a row action; `Ok(true)` asks T3 to close the surface it came from.
fn act(at: &Located, action: &str, value: &ActionValue) -> Result<bool, String> {
    let id = at.target.as_str();
    match action {
        ACTION_DUE => {
            let due = date_value(value)?
                .map(|d| d.to_string())
                .unwrap_or_default();
            commit(&at.cwd, json!({"op": "track", "id": id, "due": due}))?;
        }
        // Setting a date replaces the whole wake condition (as op=postpone
        // always does), so deferring a parked target unparks it.
        ACTION_DEFER => match date_value(value)? {
            Some(d) => commit(
                &at.cwd,
                json!({"op": "postpone", "id": id, "postponed_until": d.to_string()}),
            )?,
            None => commit(&at.cwd, json!({"op": "wake", "id": id}))?,
        },
        ACTION_PARK => commit(
            &at.cwd,
            json!({"op": "postpone", "id": id, "postpone_predicate": PARK_PREDICATE}),
        )?,
        ACTION_UNPARK => commit(&at.cwd, json!({"op": "wake", "id": id}))?,
        ACTION_ABANDON => {
            let reason = match value {
                ActionValue::Fields(fields) => fields
                    .get(ABANDON_REASON_FIELD)
                    .map(|r| r.trim())
                    .filter(|r| !r.is_empty()),
                _ => None,
            }
            .ok_or("A reason is required to abandon a target.")?;
            commit(&at.cwd, json!({"op": "defer", "id": id, "reason": reason}))?;
            return Ok(true);
        }
        ACTION_REOPEN => {
            if at.file.targets[id].status != Status::SetAside {
                return Err(format!("🎯{id} is not abandoned."));
            }
            // op=reopen only reverts achievements; resuming a set-aside
            // target is the apply transition its error message points to.
            let fragment = json!({"targets": {id: {
                "if_status": "set_aside", "status": "identified", "reason": REOPEN_REASON,
            }}});
            let tool = crate::tools::ApplyTool {
                cwd: at.cwd.to_string_lossy().into_owned(),
                fragment: fragment.to_string(),
                base: None,
                reason: None,
            };
            crate::handler::handle_apply(tool).map_err(human)?;
        }
        ACTION_WORK => return Err("T3 Code performs this action itself.".into()),
        other => return Err(format!("Unknown action `{}`.", clip(other, 64))),
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()
    }

    #[test]
    fn natural_order_sorts_numbers_numerically() {
        let mut ids = vec!["T10", "T2", "T1.1", "T1", "T9.10", "T9.2"];
        ids.sort_by(|a, b| id_key(a).cmp(&id_key(b)).then_with(|| a.cmp(b)));
        assert_eq!(ids, ["T1", "T1.1", "T2", "T9.2", "T9.10", "T10"]);
    }

    fn sample() -> TargetsFile {
        serde_yaml_ng::from_str(
            r#"
schema_version: 5
targets:
  T1:
    name: base
    status: converging
    value: 1.0
    cost: 1.0
    acceptance: [a]
    discovered: 2026-01-01
  T2:
    name: needs base
    status: identified
    depends_on: [T1]
    value: 1.0
    cost: 1.0
    acceptance: [a]
    discovered: 2026-01-01
  T3:
    name: done
    status: achieved
    value: 1.0
    cost: 1.0
    acceptance: [a]
    discovered: 2026-01-01
  T10:
    name: also ready
    status: identified
    value: 1.0
    cost: 1.0
    acceptance: [a]
    discovered: 2026-01-01
  T4:
    name: parked
    status: set_aside
    set_aside_reason: not now
    value: 1.0
    cost: 1.0
    acceptance: [a]
    discovered: 2026-01-01
"#,
        )
        .unwrap()
    }

    fn ids(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn views_select_and_order_targets() {
        let file = sample();
        // T1 unblocks T2, so it outranks T10 despite sorting later by id.
        let rows = |f| rows_for(&file, f, today());
        assert_eq!(ids(&rows(PanelFilter::Frontier)), ["T1", "T10"]);
        assert_eq!(ids(&rows(PanelFilter::Active)), ["T1", "T2", "T10"]);
        assert_eq!(
            ids(&rows(PanelFilter::All)),
            ["T1", "T2", "T10", "T3", "T4"]
        );
    }

    #[test]
    fn rows_carry_label_and_context() {
        let file = sample();
        let rows = rows_for(&file, PanelFilter::All, today());
        let by_id = |id: &str| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(by_id("T2").label, "🎯T2 needs base");
        assert_eq!(
            by_id("T2").detail.as_deref(),
            Some("identified · blocked by T1")
        );
        assert_eq!(
            by_id("T1").detail.as_deref(),
            Some("converging · unblocks 1")
        );
        assert_eq!(by_id("T4").detail.as_deref(), Some("set aside · not now"));
    }

    fn action_ids(r: &Row) -> Vec<&str> {
        r.extra["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap())
            .collect()
    }

    fn badge_labels(r: &Row) -> Vec<&str> {
        r.extra
            .get("badges")
            .and_then(Value::as_array)
            .map(|b| b.iter().map(|b| b["label"].as_str().unwrap()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn badges_and_actions_follow_target_state() {
        let mut file = sample();
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let t1 = file.targets.get_mut("T1").unwrap();
        t1.due = Some(d("2026-11-09"));
        let t2 = file.targets.get_mut("T2").unwrap();
        t2.due = Some(d("2026-11-11"));
        t2.postpone_predicate = Some(PARK_PREDICATE.into());
        let t10 = file.targets.get_mut("T10").unwrap();
        t10.due = Some(d("2027-01-05"));
        t10.postponed_until = Some(d("2026-12-01"));
        let rows = rows_for(&file, PanelFilter::All, today());
        let by_id = |id: &str| rows.iter().find(|r| r.id == id).unwrap();

        assert_eq!(badge_labels(by_id("T1")), ["Converging", "Overdue 9 Nov"]);
        assert_eq!(by_id("T1").extra["badges"][1]["tone"], "danger");
        assert_eq!(by_id("T1").extra["primaryActionId"], "work");
        assert_eq!(
            action_ids(by_id("T1")),
            ["work", "due", "defer", "park", "abandon", "dependencies"]
        );
        assert_eq!(by_id("T1").extra["actions"][1]["value"], "2026-11-09");
        assert_eq!(by_id("T1").extra["hoverSurfaceId"], "details");
        let prompt = by_id("T1").extra["actions"][0]["prompt"].as_str().unwrap();
        assert!(prompt.starts_with("Work on 🎯T1: base."), "{prompt}");

        // Parked: no agent action, Unpark instead of Park.
        assert_eq!(badge_labels(by_id("T2")), ["Due tomorrow", "Parked"]);
        assert!(by_id("T2").extra.get("primaryActionId").is_none());
        assert_eq!(
            action_ids(by_id("T2")),
            ["due", "defer", "unpark", "abandon", "dependencies"]
        );

        assert_eq!(
            badge_labels(by_id("T10")),
            ["Due 5 Jan 2027", "Deferred to 1 Dec"]
        );
        assert_eq!(by_id("T10").extra["actions"][2]["value"], "2026-12-01");

        assert_eq!(badge_labels(by_id("T4")), ["Abandoned"]);
        assert_eq!(action_ids(by_id("T4")), ["reopen", "dependencies"]);
        assert_eq!(badge_labels(by_id("T3")), ["Achieved"]);
        assert_eq!(action_ids(by_id("T3")), ["dependencies"]);
    }

    #[test]
    fn upcoming_hides_held_targets_unless_overdue() {
        let mut t = sample().targets["T10"].clone();
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert!(env_includes(EnvFilter::Upcoming, &t, today()));
        assert!(!env_includes(EnvFilter::Deferred, &t, today()));
        t.postponed_until = Some(d("2026-11-10"));
        assert!(
            env_includes(EnvFilter::Upcoming, &t, today()),
            "a due wake date holds nothing"
        );
        t.postponed_until = Some(d("2026-11-20"));
        assert!(!env_includes(EnvFilter::Upcoming, &t, today()));
        assert!(env_includes(EnvFilter::Deferred, &t, today()));
        t.due = Some(d("2026-11-01"));
        assert!(
            env_includes(EnvFilter::Upcoming, &t, today()),
            "overdue always shows"
        );
        t.postponed_until = None;
        t.due = None;
        t.postpone_predicate = Some("x".into());
        assert!(!env_includes(EnvFilter::Upcoming, &t, today()));
        assert!(env_includes(EnvFilter::All, &t, today()));
    }

    #[test]
    fn oversized_ledgers_degrade_to_a_bounded_frame() {
        let rows: Vec<Row> = (0..MAX_ROWS + 500)
            .map(|i| Row {
                id: format!("T{i}"),
                label: "x".repeat(MAX_TEXT_UNITS),
                detail: Some("y".repeat(MAX_TEXT_UNITS)),
                extra: Map::new(),
            })
            .collect();
        let panel = panel_json("p", "P", rows, &[("all", "All")], "all");
        let frame = json!({"type": "panels", "subscriptionId": "s", "panels": [panel]}).to_string();
        assert!(frame.len() <= MAX_FRAME_BYTES, "{}", frame.len());
        let v: Value = serde_json::from_str(&frame).unwrap();
        let shown = v["panels"][0]["rows"].as_array().unwrap();
        assert!(shown.len() <= MAX_ROWS);
        assert_eq!(shown.last().unwrap()["id"], "more");

        let few: Vec<Row> = (0..3)
            .map(|i| Row::notice(&format!("T{i}"), "l", None))
            .collect();
        let panel = panel_json("p", "P", few, &[("a", "A"), ("b", "B")], "a");
        assert_eq!(panel["rows"].as_array().unwrap().len(), 3);
        assert_eq!(panel["filter"], "a");
        assert_eq!(panel["filters"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn rows_with_unusable_ids_are_dropped_not_sent() {
        let mut file = sample();
        let t = file.targets.get("T1").unwrap().clone();
        file.targets.insert("x".repeat(MAX_ID_UNITS + 1), t);
        assert_eq!(
            ids(&rows_for(&file, PanelFilter::Active, today())),
            ["T1", "T2", "T10"]
        );
    }

    #[test]
    fn surfaces_link_dependencies_to_child_cards_and_stay_bounded() {
        let mut file = sample();
        let details = details_surface(&file, "T2", today()).unwrap();
        assert_eq!(details["title"], "🎯T2 needs base");
        let blocks = details["blocks"].as_array().unwrap();
        let deps = blocks.last().unwrap();
        assert_eq!(deps["items"][0]["id"], "T1");
        assert_eq!(deps["items"][0]["surfaceId"], "details");
        assert_eq!(deps["items"][0]["badges"][0]["label"], "Converging");

        let surface = dependencies_surface(&file, "T1").unwrap();
        let blocks = surface["blocks"].as_array().unwrap();
        assert_eq!(blocks[1]["text"], "None");
        assert_eq!(blocks[3]["items"][0]["id"], "T2");

        // A target with thousands of dependents still fits one frame.
        let template = file.targets["T2"].clone();
        for i in 0..3000 {
            let mut t = template.clone();
            t.name = "n".repeat(MAX_TEXT_UNITS);
            file.targets.insert(format!("D{i}"), t);
        }
        let big = dependencies_surface(&file, "T1").unwrap();
        assert!(big.to_string().len() <= MAX_FRAME_BYTES);
        let items = big["blocks"][3]["items"].as_array().unwrap();
        assert!(items.len() <= MAX_LIST_ITEMS);
        assert_eq!(items.last().unwrap()["id"], "more");

        assert!(
            abandon_surface(&file, "T3").is_err(),
            "achieved cannot be abandoned"
        );
        let form = abandon_surface(&file, "T1").unwrap();
        assert_eq!(form["blocks"][2]["actionId"], "abandon");
        assert_eq!(form["blocks"][2]["fields"][0]["required"], true);
    }

    #[test]
    fn date_values_are_strict() {
        let text = |s: &str| ActionValue::Text(s.into());
        assert_eq!(date_value(&ActionValue::None), Ok(None));
        assert_eq!(
            date_value(&text("2026-11-12")),
            Ok(NaiveDate::from_ymd_opt(2026, 11, 12))
        );
        for bad in ["2026-1-5", "12/11/2026", "2026-02-30", ""] {
            assert!(date_value(&text(bad)).is_err(), "{bad}");
        }
        assert!(date_value(&ActionValue::Fields(Default::default())).is_err());
    }
}
