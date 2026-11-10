// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Opt-in T3 Code sidebar panel endpoint (🎯T114, 🎯T116).
//!
//! T3 Code's panel-plugin protocol (version 1) lets an out-of-process
//! server supply live sidebar sections over a WebSocket. When enabled,
//! this module serves that protocol at [`PATH`] on the daemon's existing
//! listener — no second port, no adapter process, no CLI-text parsing.
//! This file owns the wire and the socket; [`crate::t3_panels`] owns the
//! ledger content.
//!
//! **Off by default.** `bullseye serve --t3-plugin` or
//! `BULLSEYE_T3_PLUGIN=1` mounts the route; otherwise the path is a 404
//! like any other.
//!
//! **Two scopes.** A *thread* subscription (`threadId` + `cwd`, no
//! `scope`) gets the `targets` panel for that workspace's ledger. An
//! *environment* subscription (`scope: "environment"` + `projects`)
//! gets the `due` panel across every project, each ledger counted once
//! however many projects share it.
//!
//! **Writes go through the MCP path.** Subscriptions only read, through
//! [`crate::store::discover_anywhere_readonly`], which never moves a
//! ledger. Row actions (due date, defer, park, abandon, reopen) write,
//! but only by calling the same handlers an MCP `bullseye_commit` /
//! `bullseye_apply` call reaches, so they take the same lock, CAS,
//! validation, evidence policy and repo guards — never ad-hoc YAML.
//!
//! **Local only.** The listener must be loopback ([`require_loopback`]),
//! and any request carrying an `Origin` header is refused: T3's server
//! is a Node client and sends none, while a browser always does, so
//! this closes the cross-site and DNS-rebinding paths to a local ledger
//! the daemon otherwise protects only by being loopback. That refusal
//! now also guards the write path.
//!
//! Protocol, per `docs/operations/panel-plugins.md` in T3 Code: the
//! client sends `subscribe` / `unsubscribe` frames; this side answers
//! every subscription with complete `panels` replacements — one
//! immediately, then one per observed change. Changes are found by
//! polling ([`POLL_INTERVAL`]) rather than by watching, because ledgers
//! are rewritten by the CLI, other agents and hand edits as well as by
//! this daemon, and the parse cache already makes an unchanged poll a
//! `stat` plus one header-line read. A write made here refreshes the
//! connection's subscriptions at once instead of waiting for the poll.
//! `action` and `surface` requests each get one `result` frame.
//!
//! **Discovery.** The same path answers a plain `GET` with a
//! context-free probe, and the daemon advertises itself to T3 Code
//! through [`crate::t3_discovery`]; T3 never needs hand-written
//! configuration, only its user's consent.
//!
//! A malformed frame closes the connection with 1008, except a request
//! with a usable `requestId`, which is answered with an error result.

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get};
use chrono::NaiveDate;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::t3_panels::{self, ENV_PANEL_ID, EnvFilter, LedgerRead, PANEL_ID, PanelFilter, Reply};

/// Environment switch for the endpoint (`1`/`true`/`yes`/`on` enable).
pub const ENV_FLAG: &str = "BULLSEYE_T3_PLUGIN";
/// Path served on the daemon's listener while enabled.
pub const PATH: &str = "/plugins/t3";
/// The identity Bullseye advertises: the `host/owner/repo` of its project.
/// T3 uses it as the plugin id of a discovered panel, so it is also the
/// `pluginId` of that panel's filter selections.
pub const PLUGIN_ID: &str = "github.com/marcelocantos/bullseye";
/// The id a hand-written `panel-plugins.json` entry (and every build before
/// the canonical identity) uses. Filter selections naming it still apply.
pub const LEGACY_PLUGIN_ID: &str = "bullseye";
const PROTOCOL_VERSION: u64 = 1;

/// Largest frame in either direction (T3: `MAX_FRAME_BYTES`).
pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Headroom under [`MAX_FRAME_BYTES`] for the envelope and filter list.
pub(crate) const FRAME_MARGIN_BYTES: usize = 16 * 1024;
/// T3 contract: `Label` length, in UTF-16 code units (JS `string.length`).
pub(crate) const MAX_TEXT_UNITS: usize = 4096;
/// T3 contract: `Id` length, in UTF-16 code units.
pub(crate) const MAX_ID_UNITS: usize = 128;
/// T3 contract: a string action value.
const MAX_VALUE_UNITS: usize = 16_384;
/// Fields in one form value (T3's forms hold at most 16).
const MAX_VALUE_FIELDS: usize = 64;
/// Projects in one environment frame. Each costs a discovery walk per
/// poll, so the bound is on work as much as on memory.
const MAX_PROJECTS: usize = 1000;
/// T3 contract: filters per subscribe request.
const MAX_REQUEST_FILTERS: usize = 100;
/// Longest accepted `cwd`, in bytes (PATH_MAX on the platforms we ship).
const MAX_CWD_BYTES: usize = 4096;

/// Concurrent sockets. T3 shares one per plugin per environment, so a
/// handful is generous; the cap bounds what a runaway local client can
/// pin.
const MAX_CONNECTIONS: usize = 16;
/// Subscriptions on one socket (one per open thread sidebar).
const MAX_SUBSCRIPTIONS: usize = 128;
/// How often a connection re-reads its subscribed ledgers.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// A client that cannot take a frame this long is dropped, not waited on.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

const CLOSE_POLICY: u16 = 1008;
const CLOSE_UNSUPPORTED: u16 = 1003;

/// Interpret the `BULLSEYE_T3_PLUGIN` value.
///
/// Unset or empty is off. An unrecognised value is an error rather than
/// "off": a typo such as `=enable` silently leaving the panel missing
/// is a worse failure than a daemon that refuses to start.
pub fn parse_flag_value(value: Option<&str>) -> Result<bool, String> {
    let Some(raw) = value else { return Ok(false) };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        other => Err(format!(
            "{ENV_FLAG}=`{other}` is not recognised (use 1 or 0)"
        )),
    }
}

/// Whether the environment asks for the endpoint.
pub fn enabled_from_env() -> Result<bool, String> {
    parse_flag_value(std::env::var(ENV_FLAG).ok().as_deref())
}

/// The endpoint serves an unauthenticated local ledger, so it may only
/// ride a loopback listener.
pub fn require_loopback(host: &str) -> Result<(), String> {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }
    match bare.parse::<IpAddr>() {
        Ok(ip) if ip.is_loopback() => Ok(()),
        _ => Err(format!(
            "the T3 panel endpoint serves an unauthenticated local ledger and requires a \
             loopback listener, but the address host is `{host}`"
        )),
    }
}

/// A project of the T3 environment, as an environment frame lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
}

/// Which workspace(s) a request is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// One thread's workspace.
    Thread { cwd: PathBuf },
    /// Every project of the environment.
    Environment { projects: Vec<Project> },
}

/// What a subscription watches, and through which view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watch {
    Thread {
        cwd: PathBuf,
        filter: PanelFilter,
    },
    Environment {
        projects: Vec<Project>,
        filter: EnvFilter,
    },
}

/// A validated `subscribe` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribe {
    pub subscription_id: String,
    pub watch: Watch,
}

/// An action's value: absent/`null`, a string, or form fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionValue {
    None,
    Text(String),
    Fields(BTreeMap<String, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestKind {
    Action {
        action_id: String,
        surface_id: Option<String>,
        value: ActionValue,
    },
    Surface {
        surface_id: String,
        item_id: Option<String>,
    },
}

/// A validated `action` or `surface` request, answered by one `result`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub request_id: String,
    pub scope: Scope,
    pub panel_id: String,
    pub row_id: String,
    pub kind: RequestKind,
}

/// A validated client frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    Subscribe(Subscribe),
    Unsubscribe {
        subscription_id: String,
    },
    Request(Request),
    /// A request with a usable `requestId` but unusable fields: answered
    /// with an error result rather than by closing the socket.
    Invalid {
        request_id: String,
        error: String,
    },
}

pub(crate) fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn string_field<'a>(obj: &'a Value, name: &str) -> Result<&'a str, String> {
    obj.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{name}` must be a string"))
}

fn id_field(obj: &Value, name: &str) -> Result<String, String> {
    let s = string_field(obj, name)?;
    if s.is_empty() || utf16_len(s) > MAX_ID_UNITS {
        return Err(format!(
            "`{name}` must be 1 to {MAX_ID_UNITS} characters long"
        ));
    }
    Ok(s.to_string())
}

fn optional_id_field(obj: &Value, name: &str) -> Result<Option<String>, String> {
    match obj.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => id_field(obj, name).map(Some),
    }
}

fn validate_cwd(cwd: &str) -> Result<PathBuf, String> {
    if cwd.is_empty() || cwd.len() > MAX_CWD_BYTES || cwd.contains('\0') {
        return Err("`cwd` must be a non-empty path of at most 4096 bytes".into());
    }
    let path = Path::new(cwd);
    if !path.is_absolute() {
        return Err("`cwd` must be an absolute path".into());
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("`cwd` must not contain `..`".into());
    }
    Ok(path.to_path_buf())
}

fn parse_projects(msg: &Value) -> Result<Vec<Project>, String> {
    let list = msg
        .get("projects")
        .and_then(Value::as_array)
        .ok_or("`projects` must be an array")?;
    if list.len() > MAX_PROJECTS {
        return Err(format!("`projects` holds more than {MAX_PROJECTS} entries"));
    }
    list.iter()
        .map(|p| {
            if !p.is_object() {
                return Err("each project must be an object".to_string());
            }
            let title = string_field(p, "title")?;
            if utf16_len(title) > MAX_TEXT_UNITS {
                return Err("a project `title` is too long".into());
            }
            Ok(Project {
                id: id_field(p, "projectId")?,
                title: title.to_string(),
                cwd: validate_cwd(string_field(p, "cwd")?)?,
            })
        })
        .collect()
}

/// The frame's scope. Thread frames predate scopes, so a missing
/// `scope` is a thread.
fn parse_scope(msg: &Value) -> Result<Scope, String> {
    match msg.get("scope").map(Value::as_str) {
        None | Some(Some("thread")) => {
            id_field(msg, "threadId")?;
            Ok(Scope::Thread {
                cwd: validate_cwd(string_field(msg, "cwd")?)?,
            })
        }
        Some(Some("environment")) => Ok(Scope::Environment {
            projects: parse_projects(msg)?,
        }),
        _ => Err("`scope` must be \"thread\" or \"environment\"".into()),
    }
}

/// The filter selected for `panel` of this plugin, if any. Entries for
/// other plugins or panels are not ours to judge.
fn selected_filter<'a>(msg: &'a Value, panel: &str) -> Result<Option<&'a str>, String> {
    let filters = msg
        .get("filters")
        .and_then(Value::as_array)
        .ok_or("`filters` must be an array")?;
    if filters.len() > MAX_REQUEST_FILTERS {
        return Err(format!(
            "`filters` holds more than {MAX_REQUEST_FILTERS} entries"
        ));
    }
    let mut selected = None;
    for entry in filters {
        if !entry.is_object() {
            return Err("each filter must be an object".into());
        }
        let plugin = string_field(entry, "pluginId")?;
        let panel_id = string_field(entry, "panelId")?;
        let filter = string_field(entry, "filter")?;
        if (plugin == PLUGIN_ID || plugin == LEGACY_PLUGIN_ID) && panel_id == panel {
            selected = Some(filter);
        }
    }
    Ok(selected)
}

fn unknown_filter(id: &str) -> String {
    format!("unknown filter `{}`", clip(id, 64))
}

fn parse_subscribe(msg: &Value) -> Result<Subscribe, String> {
    if msg.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
        return Err(format!("`protocol` must be {PROTOCOL_VERSION}"));
    }
    let watch = match parse_scope(msg)? {
        Scope::Thread { cwd } => {
            let filter = match selected_filter(msg, PANEL_ID)? {
                Some(id) => PanelFilter::parse(id).ok_or_else(|| unknown_filter(id))?,
                None => PanelFilter::Frontier,
            };
            Watch::Thread { cwd, filter }
        }
        Scope::Environment { projects } => {
            let filter = match selected_filter(msg, ENV_PANEL_ID)? {
                Some(id) => EnvFilter::parse(id).ok_or_else(|| unknown_filter(id))?,
                None => EnvFilter::Upcoming,
            };
            Watch::Environment { projects, filter }
        }
    };
    Ok(Subscribe {
        subscription_id: id_field(msg, "subscriptionId")?,
        watch,
    })
}

fn parse_value(msg: &Value) -> Result<ActionValue, String> {
    match msg.get("value") {
        None | Some(Value::Null) => Ok(ActionValue::None),
        Some(Value::String(s)) if utf16_len(s) <= MAX_VALUE_UNITS => {
            Ok(ActionValue::Text(s.clone()))
        }
        Some(Value::Object(fields)) if fields.len() <= MAX_VALUE_FIELDS => fields
            .iter()
            .map(|(k, v)| match v.as_str() {
                Some(v)
                    if !k.is_empty()
                        && utf16_len(k) <= MAX_ID_UNITS
                        && utf16_len(v) <= MAX_TEXT_UNITS =>
                {
                    Ok((k.clone(), v.to_string()))
                }
                _ => Err("each `value` field must be a bounded string".to_string()),
            })
            .collect::<Result<_, _>>()
            .map(ActionValue::Fields),
        _ => Err("`value` must be null, a bounded string, or an object of strings".into()),
    }
}

fn parse_request(msg: &Value, request_id: String, surface: bool) -> Result<Request, String> {
    let kind = if surface {
        RequestKind::Surface {
            surface_id: id_field(msg, "surfaceId")?,
            item_id: optional_id_field(msg, "itemId")?,
        }
    } else {
        RequestKind::Action {
            action_id: id_field(msg, "actionId")?,
            surface_id: optional_id_field(msg, "surfaceId")?,
            value: parse_value(msg)?,
        }
    };
    Ok(Request {
        request_id,
        scope: parse_scope(msg)?,
        panel_id: id_field(msg, "panelId")?,
        row_id: id_field(msg, "rowId")?,
        kind,
    })
}

/// Validate one incoming text frame.
///
/// Unknown extra fields are ignored so a newer T3 can add to the
/// request; everything the handler acts on is checked. A request whose
/// `requestId` is usable is answered even when the rest is not.
pub fn parse_client_message(text: &str) -> Result<ClientMessage, String> {
    let msg: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    if !msg.is_object() {
        return Err("a frame must be a JSON object".into());
    }
    match string_field(&msg, "type")? {
        "unsubscribe" => Ok(ClientMessage::Unsubscribe {
            subscription_id: id_field(&msg, "subscriptionId")?,
        }),
        "subscribe" => parse_subscribe(&msg).map(ClientMessage::Subscribe),
        kind @ ("action" | "surface") => {
            let request_id = id_field(&msg, "requestId")?;
            Ok(
                match parse_request(&msg, request_id.clone(), kind == "surface") {
                    Ok(req) => ClientMessage::Request(req),
                    Err(error) => ClientMessage::Invalid { request_id, error },
                },
            )
        }
        other => Err(format!("unknown frame type `{}`", clip(other, 64))),
    }
}

/// Truncate to at most `max_units` UTF-16 code units without splitting
/// a character.
pub(crate) fn clip(s: &str, max_units: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in s.chars() {
        units += c.len_utf16();
        if units > max_units {
            break;
        }
        out.push(c);
    }
    out
}

/// One line, bounded: ledger text is hand-editable and may carry
/// newlines or control characters that a one-line sidebar row cannot.
pub(crate) fn line(s: &str, max_units: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clip(flat.trim(), max_units)
}

fn snapshot_frame(subscription_id: &str, panels: Vec<Value>) -> String {
    json!({"type": "panels", "subscriptionId": subscription_id, "panels": panels}).to_string()
}

fn panels_for(
    watch: &Watch,
    today: NaiveDate,
    ledgers: &mut HashMap<PathBuf, LedgerRead>,
) -> Vec<Value> {
    match watch {
        Watch::Thread { cwd, filter } => t3_panels::thread_panels(cwd, *filter, today, ledgers),
        Watch::Environment { projects, filter } => {
            t3_panels::environment_panels(projects, *filter, today, ledgers)
        }
    }
}

/// The calendar day the user's panels are drawn for.
fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn result_frame(request_id: &str, outcome: Result<Reply, String>) -> String {
    match outcome {
        Ok(reply) => {
            let mut frame = json!({"type": "result", "requestId": request_id, "ok": true});
            if let Some(surface) = reply.surface {
                frame["surface"] = surface;
            }
            if reply.close {
                frame["close"] = json!(true);
            }
            frame.to_string()
        }
        Err(error) => json!({
            "type": "result", "requestId": request_id, "ok": false,
            "error": line(&error, MAX_TEXT_UNITS),
        })
        .to_string(),
    }
}

/// A subscription as the connection tracks it.
struct Sub {
    watch: Watch,
    /// Digest of the last frame sent; `None` until the first.
    sent: Option<[u8; 32]>,
}

struct Job {
    id: String,
    watch: Watch,
    sent: Option<[u8; 32]>,
}

/// Frames that differ from what each job last sent. Blocking: reads
/// ledgers and may run `git`. One ledger is read once per call however
/// many subscriptions share it.
fn changed_frames(jobs: Vec<Job>, today: NaiveDate) -> Vec<(String, [u8; 32], String)> {
    let mut ledgers = HashMap::new();
    let mut out = Vec::new();
    for job in jobs {
        let frame = snapshot_frame(&job.id, panels_for(&job.watch, today, &mut ledgers));
        let digest: [u8; 32] = Sha256::digest(frame.as_bytes()).into();
        if job.sent != Some(digest) {
            out.push((job.id, digest, frame));
        }
    }
    out
}

/// Send one text frame. `Err` means the socket is unusable.
async fn send(socket: &mut WebSocket, frame: String) -> Result<(), ()> {
    debug_assert!(frame.len() <= MAX_FRAME_BYTES);
    match tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::text(frame))).await {
        Ok(Ok(())) => Ok(()),
        _ => Err(()),
    }
}

/// Re-evaluate subscriptions (all, or just `only`) and send what changed.
/// `Err` means the socket is unusable.
async fn refresh(
    socket: &mut WebSocket,
    subs: &mut HashMap<String, Sub>,
    only: Option<&str>,
) -> Result<(), ()> {
    let jobs: Vec<Job> = subs
        .iter()
        .filter(|(id, _)| only.is_none_or(|o| o == id.as_str()))
        .map(|(id, s)| Job {
            id: id.clone(),
            watch: s.watch.clone(),
            sent: s.sent,
        })
        .collect();
    let day = today();
    let frames = tokio::task::spawn_blocking(move || changed_frames(jobs, day))
        .await
        .map_err(|_| ())?;
    for (id, digest, frame) in frames {
        send(socket, frame).await?;
        // The client may have unsubscribed while we waited; never revive.
        if let Some(sub) = subs.get_mut(&id) {
            sub.sent = Some(digest);
        }
    }
    Ok(())
}

async fn close_with(socket: &mut WebSocket, code: u16, reason: &str) {
    let frame = CloseFrame {
        code,
        // Close reasons are limited to 123 bytes.
        reason: clip(reason, 100).into(),
    };
    let _ = tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Close(Some(frame)))).await;
}

/// Serve one socket until the peer leaves, misbehaves, or stalls.
/// Dropping the socket on return is the whole cleanup: subscriptions are
/// plain state in this task, and the semaphore permit frees the slot.
async fn serve_connection(mut socket: WebSocket, _slot: OwnedSemaphorePermit) {
    let mut subs: HashMap<String, Sub> = HashMap::new();
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let text = match incoming {
                    None | Some(Err(_) | Ok(Message::Close(_))) => return,
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Binary(_))) => {
                        close_with(&mut socket, CLOSE_UNSUPPORTED, "text frames only").await;
                        return;
                    }
                };
                match parse_client_message(text.as_str()) {
                    Err(reason) => {
                        close_with(&mut socket, CLOSE_POLICY, &reason).await;
                        return;
                    }
                    Ok(ClientMessage::Unsubscribe { subscription_id }) => {
                        subs.remove(&subscription_id);
                    }
                    Ok(ClientMessage::Subscribe(req)) => {
                        if subs.len() >= MAX_SUBSCRIPTIONS && !subs.contains_key(&req.subscription_id) {
                            close_with(&mut socket, CLOSE_POLICY, "too many subscriptions").await;
                            return;
                        }
                        // Re-subscribing an id replaces it (a filter or project change).
                        subs.insert(
                            req.subscription_id.clone(),
                            Sub { watch: req.watch, sent: None },
                        );
                        if refresh(&mut socket, &mut subs, Some(&req.subscription_id)).await.is_err() {
                            return;
                        }
                    }
                    Ok(ClientMessage::Invalid { request_id, error }) => {
                        if send(&mut socket, result_frame(&request_id, Err(error))).await.is_err() {
                            return;
                        }
                    }
                    Ok(ClientMessage::Request(req)) => {
                        let request_id = req.request_id.clone();
                        let day = today();
                        let outcome = tokio::task::spawn_blocking(move || {
                            t3_panels::handle_request(&req, day)
                        })
                        .await
                        .unwrap_or_else(|_| Err("The request failed inside Bullseye.".into()));
                        let mutated = matches!(&outcome, Ok(reply) if reply.mutated);
                        if send(&mut socket, result_frame(&request_id, outcome)).await.is_err() {
                            return;
                        }
                        // Show the write now rather than at the next poll.
                        if mutated && refresh(&mut socket, &mut subs, None).await.is_err() {
                            return;
                        }
                    }
                }
            }
            _ = ticker.tick(), if !subs.is_empty() => {
                if refresh(&mut socket, &mut subs, None).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn upgrade(headers: &HeaderMap, ws: WebSocketUpgrade, slots: Arc<Semaphore>) -> Response {
    if headers.contains_key(header::ORIGIN) {
        return (StatusCode::FORBIDDEN, "browser origins are not allowed\n").into_response();
    }
    let Ok(slot) = slots.try_acquire_owned() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "too many connections\n").into_response();
    };
    ws.max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| serve_connection(socket, slot))
}

/// Context-free compatibility probe: a plain `GET` (no `Upgrade`) names
/// the service and protocol and nothing else. T3 Code uses it to check a
/// discovered manifest before it asks its user anything; no thread,
/// workspace or ledger is involved. The instance header lets the daemon
/// that advertises the endpoint confirm it is the one listening.
fn probe(headers: &HeaderMap) -> Response {
    if headers.contains_key(header::ORIGIN) {
        return (StatusCode::FORBIDDEN, "browser origins are not allowed\n").into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::HeaderName::from_static(crate::t3_discovery::INSTANCE_HEADER),
                crate::t3_discovery::instance_token(),
            ),
        ],
        crate::t3_discovery::probe_body(),
    )
        .into_response()
}

/// The route to mount at [`PATH`] via `HyperServer::with_route`.
pub fn route() -> MethodRouter {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    get(
        move |headers: HeaderMap, ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>| {
            let slots = Arc::clone(&slots);
            async move {
                match ws {
                    Ok(ws) => upgrade(&headers, ws, slots),
                    // A request that asked to upgrade but did so wrongly
                    // gets axum's rejection; anything else is the probe.
                    Err(rejection) if headers.contains_key(header::UPGRADE) => {
                        rejection.into_response()
                    }
                    Err(_) => probe(&headers),
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subscribe(extra: &str) -> String {
        format!(
            r#"{{"type":"subscribe","protocol":1,"subscriptionId":"s1","threadId":"t1","cwd":"/work/x","filters":[{extra}]}}"#
        )
    }

    fn env_subscribe(extra: &str) -> String {
        format!(
            r#"{{"type":"subscribe","protocol":1,"scope":"environment","subscriptionId":"e1","projects":[{{"projectId":"p1","title":"One","cwd":"/work/a"}}],"filters":[{extra}]}}"#
        )
    }

    fn thread_watch(text: &str) -> (PathBuf, PanelFilter) {
        match parse_client_message(text) {
            Ok(ClientMessage::Subscribe(Subscribe {
                watch: Watch::Thread { cwd, filter },
                ..
            })) => (cwd, filter),
            other => panic!("expected a thread subscribe, got {other:?}"),
        }
    }

    #[test]
    fn flag_value_defaults_off_and_rejects_typos() {
        assert_eq!(parse_flag_value(None), Ok(false));
        assert_eq!(parse_flag_value(Some("")), Ok(false));
        assert_eq!(parse_flag_value(Some("0")), Ok(false));
        assert_eq!(parse_flag_value(Some("1")), Ok(true));
        assert_eq!(parse_flag_value(Some(" TRUE ")), Ok(true));
        assert!(parse_flag_value(Some("enable")).is_err());
    }

    #[test]
    fn only_loopback_listeners_are_allowed() {
        for ok in ["127.0.0.1", "localhost", "::1", "[::1]", "127.0.0.2"] {
            assert!(require_loopback(ok).is_ok(), "{ok}");
        }
        for bad in ["0.0.0.0", "192.168.1.5", "example.com", "", "::"] {
            assert!(require_loopback(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn missing_filter_selection_defaults_to_frontier() {
        let (cwd, filter) = thread_watch(&subscribe(""));
        assert_eq!(filter, PanelFilter::Frontier);
        assert_eq!(cwd, PathBuf::from("/work/x"));
    }

    #[test]
    fn filter_selection_applies_only_to_this_panel() {
        let other = r#"{"pluginId":"other","panelId":"targets","filter":"nonsense"}"#;
        // The canonical id (a discovered panel) and the short id (a
        // hand-written configuration) both select this panel's filter.
        for id in [PLUGIN_ID, LEGACY_PLUGIN_ID] {
            let ours = format!(r#"{{"pluginId":"{id}","panelId":"targets","filter":"all"}}"#);
            assert_eq!(
                thread_watch(&subscribe(&format!("{other},{ours}"))).1,
                PanelFilter::All,
                "{id}"
            );
        }
        let lookalike =
            r#"{"pluginId":"github.com/other/bullseye","panelId":"targets","filter":"nonsense"}"#;
        assert_eq!(thread_watch(&subscribe(lookalike)).1, PanelFilter::Frontier);
        // The environment panel's selection is not the thread panel's.
        let env = format!(r#"{{"pluginId":"{PLUGIN_ID}","panelId":"due","filter":"deferred"}}"#);
        assert_eq!(thread_watch(&subscribe(&env)).1, PanelFilter::Frontier);
    }

    #[test]
    fn environment_subscribe_parses_projects_and_its_own_filter() {
        let parsed = |text: &str| match parse_client_message(text) {
            Ok(ClientMessage::Subscribe(Subscribe {
                subscription_id,
                watch: Watch::Environment { projects, filter },
            })) => (subscription_id, projects, filter),
            other => panic!("expected an environment subscribe, got {other:?}"),
        };
        let (id, projects, filter) = parsed(&env_subscribe(""));
        assert_eq!(id, "e1");
        assert_eq!(filter, EnvFilter::Upcoming);
        assert_eq!(
            projects,
            [Project {
                id: "p1".into(),
                title: "One".into(),
                cwd: PathBuf::from("/work/a"),
            }]
        );
        let sel = format!(r#"{{"pluginId":"{PLUGIN_ID}","panelId":"due","filter":"deferred"}}"#);
        assert_eq!(parsed(&env_subscribe(&sel)).2, EnvFilter::Deferred);
        let thread_sel =
            format!(r#"{{"pluginId":"{PLUGIN_ID}","panelId":"targets","filter":"bogus"}}"#);
        assert_eq!(parsed(&env_subscribe(&thread_sel)).2, EnvFilter::Upcoming);
    }

    #[test]
    fn malformed_requests_are_rejected() {
        let ours =
            |f: &str| format!(r#"{{"pluginId":"{PLUGIN_ID}","panelId":"targets","filter":{f}}}"#);
        let long = "x".repeat(MAX_ID_UNITS + 1);
        let cases: Vec<(String, &str)> = vec![
            ("not json".into(), "invalid JSON"),
            ("[]".into(), "JSON object"),
            (r#"{"type":"explode"}"#.into(), "unknown frame type"),
            (r#"{"type":"unsubscribe"}"#.into(), "subscriptionId"),
            (
                r#"{"type":"unsubscribe","subscriptionId":""}"#.into(),
                "subscriptionId",
            ),
            (
                subscribe("").replace("\"protocol\":1", "\"protocol\":2"),
                "protocol",
            ),
            (subscribe("").replace("\"protocol\":1,", ""), "protocol"),
            (
                subscribe("").replace("\"s1\"", &format!("\"{long}\"")),
                "subscriptionId",
            ),
            (subscribe("").replace("\"t1\"", "7"), "threadId"),
            (subscribe("").replace("/work/x", "relative/dir"), "absolute"),
            (subscribe("").replace("/work/x", "/work/../etc"), ".."),
            (subscribe("").replace("/work/x", ""), "cwd"),
            (subscribe("").replace("/work/x", "/a\\u0000b"), "cwd"),
            (subscribe("").replace("[]", "{}"), "filters"),
            (subscribe("3"), "object"),
            (subscribe(&ours("\"bogus\"")), "unknown filter"),
            (subscribe(&ours("1")), "filter"),
            (
                subscribe(&format!(r#"{{"pluginId":"{PLUGIN_ID}"}}"#)),
                "panelId",
            ),
            (
                subscribe("").replace("\"protocol\":1", "\"protocol\":1,\"scope\":\"galaxy\""),
                "scope",
            ),
            (
                env_subscribe("").replace("[{\"projectId\"", "[3,{\"projectId\""),
                "project",
            ),
            (env_subscribe("").replace("/work/a", "rel"), "absolute"),
            (env_subscribe("").replace("\"p1\"", "\"\""), "projectId"),
            (
                env_subscribe(&format!(
                    r#"{{"pluginId":"{PLUGIN_ID}","panelId":"due","filter":"x"}}"#
                )),
                "unknown filter",
            ),
            // A request without a usable id cannot be answered, only refused.
            (r#"{"type":"action","rowId":"T1"}"#.into(), "requestId"),
        ];
        for (text, needle) in cases {
            let err = parse_client_message(&text).expect_err(&text);
            assert!(err.contains(needle), "`{text}` -> `{err}` lacks `{needle}`");
        }
        let many = vec![r#"{"pluginId":"p","panelId":"q","filter":"r"}"#; MAX_REQUEST_FILTERS + 1];
        assert!(parse_client_message(&subscribe(&many.join(","))).is_err());
    }

    #[test]
    fn unsubscribe_parses() {
        assert_eq!(
            parse_client_message(r#"{"type":"unsubscribe","subscriptionId":"s9"}"#),
            Ok(ClientMessage::Unsubscribe {
                subscription_id: "s9".into()
            })
        );
    }

    #[test]
    fn requests_parse_with_their_scope_and_value() {
        let thread = r#"{"type":"action","requestId":"r1","scope":"thread","threadId":"t","cwd":"/w","panelId":"targets","rowId":"T1","actionId":"due","value":"2026-11-12"}"#;
        assert_eq!(
            parse_client_message(thread),
            Ok(ClientMessage::Request(Request {
                request_id: "r1".into(),
                scope: Scope::Thread {
                    cwd: PathBuf::from("/w")
                },
                panel_id: "targets".into(),
                row_id: "T1".into(),
                kind: RequestKind::Action {
                    action_id: "due".into(),
                    surface_id: None,
                    value: ActionValue::Text("2026-11-12".into()),
                },
            }))
        );
        let form = r#"{"type":"action","requestId":"r2","scope":"environment","projects":[],"panelId":"due","rowId":"k:T1","actionId":"abandon","surfaceId":"abandon","value":{"reason":"no"}}"#;
        let Ok(ClientMessage::Request(req)) = parse_client_message(form) else {
            panic!()
        };
        assert_eq!(req.scope, Scope::Environment { projects: vec![] });
        assert_eq!(
            req.kind,
            RequestKind::Action {
                action_id: "abandon".into(),
                surface_id: Some("abandon".into()),
                value: ActionValue::Fields([("reason".to_string(), "no".to_string())].into()),
            }
        );
        // Older thread frames carry no `scope`; `value: null` clears.
        let surface = r#"{"type":"surface","requestId":"r3","threadId":"t","cwd":"/w","panelId":"targets","rowId":"T1","surfaceId":"details","itemId":"T2"}"#;
        let Ok(ClientMessage::Request(req)) = parse_client_message(surface) else {
            panic!()
        };
        assert_eq!(
            req.kind,
            RequestKind::Surface {
                surface_id: "details".into(),
                item_id: Some("T2".into()),
            }
        );
        let cleared = thread.replace("\"2026-11-12\"", "null");
        let Ok(ClientMessage::Request(req)) = parse_client_message(&cleared) else {
            panic!()
        };
        assert!(matches!(
            req.kind,
            RequestKind::Action {
                value: ActionValue::None,
                ..
            }
        ));
    }

    #[test]
    fn bad_requests_with_an_id_become_error_results() {
        let base = r#"{"type":"action","requestId":"r1","threadId":"t","cwd":"/w","panelId":"targets","rowId":"T1","actionId":"due"}"#;
        for (bad, needle) in [
            (
                base.replace("\"actionId\":\"due\"", "\"actionId\":5"),
                "actionId",
            ),
            (base.replace("\"/w\"", "\"w\""), "absolute"),
            (base.replace("}", ",\"value\":[1]}"), "value"),
            (base.replace("}", ",\"value\":{\"reason\":3}}"), "value"),
            (
                base.replace("\"type\":\"action\"", "\"type\":\"surface\""),
                "surfaceId",
            ),
        ] {
            match parse_client_message(&bad) {
                Ok(ClientMessage::Invalid { request_id, error }) => {
                    assert_eq!(request_id, "r1");
                    assert!(error.contains(needle), "`{bad}` -> `{error}`");
                }
                other => panic!("`{bad}` -> {other:?}"),
            }
        }
    }

    #[test]
    fn results_carry_surface_close_and_errors() {
        let ok: Value = serde_json::from_str(&result_frame(
            "r",
            Ok(Reply {
                surface: Some(json!({"title": "t", "blocks": []})),
                close: true,
                mutated: false,
            }),
        ))
        .unwrap();
        assert_eq!(
            ok,
            json!({"type": "result", "requestId": "r", "ok": true,
                   "surface": {"title": "t", "blocks": []}, "close": true})
        );
        let plain: Value = serde_json::from_str(&result_frame(
            "r",
            Ok(Reply {
                surface: None,
                close: false,
                mutated: true,
            }),
        ))
        .unwrap();
        assert_eq!(
            plain,
            json!({"type": "result", "requestId": "r", "ok": true})
        );
        let err: Value = serde_json::from_str(&result_frame("r", Err("nope\nno".into()))).unwrap();
        assert_eq!(
            err,
            json!({"type": "result", "requestId": "r", "ok": false, "error": "nope no"})
        );
    }

    #[test]
    fn clipping_counts_utf16_units_on_character_boundaries() {
        assert_eq!(clip("abcdef", 3), "abc");
        // 🎯 is two UTF-16 units: three of them do not fit in five.
        assert_eq!(clip("🎯🎯🎯", 5), "🎯🎯");
        assert_eq!(line("a\nb\tc\r", 10), "a b c");
    }
}
