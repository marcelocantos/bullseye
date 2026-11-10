// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Opt-in T3 Code sidebar panel endpoint (🎯T114).
//!
//! T3 Code's panel-plugin protocol (version 1) lets an out-of-process
//! server supply live, thread-scoped sidebar sections over a WebSocket.
//! When enabled, this module serves that protocol at [`PATH`] on the
//! daemon's existing listener — no second port, no adapter process, no
//! CLI-text parsing. Rows come straight from the ledger graph
//! ([`crate::graph::frontier`] and [`crate::schema`]).
//!
//! **Off by default.** `bullseye serve --t3-plugin` or
//! `BULLSEYE_T3_PLUGIN=1` mounts the route; otherwise the path is a 404
//! like any other.
//!
//! **Read only.** The endpoint never mutates a ledger, and its
//! discovery ([`crate::store::discover_anywhere_readonly`]) does not
//! even perform the path-keyed → repo-keyed ledger move that MCP
//! discovery does.
//!
//! **Local only.** The listener must be loopback ([`require_loopback`]),
//! and any request carrying an `Origin` header is refused: T3's server
//! is a Node client and sends none, while a browser always does, so
//! this closes the cross-site and DNS-rebinding paths to a local ledger
//! the daemon otherwise protects only by being loopback.
//!
//! Protocol, per `docs/operations/panel-plugins.md` in T3 Code:
//! the client sends `subscribe` / `unsubscribe` frames; this side
//! answers every subscription with complete `panels` replacements — one
//! immediately, then one per observed change. Changes are found by
//! polling ([`POLL_INTERVAL`]) rather than by watching, because ledgers
//! are rewritten by the CLI, other agents and hand edits as well as by
//! this daemon, and the parse cache already makes an unchanged poll a
//! `stat` plus one header-line read.
//!
//! **Discovery.** The same path answers a plain `GET` with a
//! context-free probe, and the daemon advertises itself to T3 Code
//! through [`crate::t3_discovery`]; T3 never needs hand-written
//! configuration, only its user's consent.
//!
//! A malformed request closes the connection with 1008: the protocol
//! has no error frame, and T3 treats any unknown frame as fatal anyway.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::graph::{frontier_tolerant, open_blockers, rank_frontier, unblocking_fanout};
use crate::schema::{Status, Target, TargetsFile};
use crate::store;

/// Environment switch for the endpoint (`1`/`true`/`yes`/`on` enable).
pub const ENV_FLAG: &str = "BULLSEYE_T3_PLUGIN";
/// Path served on the daemon's listener while enabled.
pub const PATH: &str = "/plugins/t3";
/// `pluginId` T3 addresses this panel provider by.
pub const PLUGIN_ID: &str = "bullseye";
/// The one panel this provider supplies.
pub const PANEL_ID: &str = "targets";

const PROTOCOL_VERSION: u64 = 1;

/// Largest frame in either direction (T3: `MAX_FRAME_BYTES`).
const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Headroom under [`MAX_FRAME_BYTES`] for the envelope and filter list.
const FRAME_MARGIN_BYTES: usize = 16 * 1024;
/// T3 contract: rows per panel.
const MAX_ROWS: usize = 2000;
/// T3 contract: `Label` length, in UTF-16 code units (JS `string.length`).
const MAX_TEXT_UNITS: usize = 4096;
/// T3 contract: `Id` length, in UTF-16 code units.
const MAX_ID_UNITS: usize = 128;
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

/// The ledger view a subscription asked for.
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

    fn parse(id: &str) -> Option<Self> {
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

/// A validated `subscribe` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribe {
    pub subscription_id: String,
    pub thread_id: String,
    pub cwd: PathBuf,
    pub filter: PanelFilter,
}

/// A validated client frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    Subscribe(Subscribe),
    Unsubscribe { subscription_id: String },
}

fn utf16_len(s: &str) -> usize {
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

/// Validate one incoming text frame.
///
/// Unknown extra fields are ignored so a newer T3 can add to the
/// request; everything the handler acts on is checked.
pub fn parse_client_message(text: &str) -> Result<ClientMessage, String> {
    let msg: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    if !msg.is_object() {
        return Err("a frame must be a JSON object".into());
    }
    match string_field(&msg, "type")? {
        "unsubscribe" => Ok(ClientMessage::Unsubscribe {
            subscription_id: id_field(&msg, "subscriptionId")?,
        }),
        "subscribe" => {
            if msg.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
                return Err(format!("`protocol` must be {PROTOCOL_VERSION}"));
            }
            let filters = msg
                .get("filters")
                .and_then(Value::as_array)
                .ok_or("`filters` must be an array")?;
            if filters.len() > MAX_REQUEST_FILTERS {
                return Err(format!(
                    "`filters` holds more than {MAX_REQUEST_FILTERS} entries"
                ));
            }
            // A missing selection is the default view; entries for other
            // plugins or panels are not ours to judge.
            let mut filter = PanelFilter::Frontier;
            for entry in filters {
                if !entry.is_object() {
                    return Err("each filter must be an object".into());
                }
                let plugin = string_field(entry, "pluginId")?;
                let panel = string_field(entry, "panelId")?;
                let selected = string_field(entry, "filter")?;
                if plugin == PLUGIN_ID && panel == PANEL_ID {
                    filter = PanelFilter::parse(selected)
                        .ok_or_else(|| format!("unknown filter `{}`", clip(selected, 64)))?;
                }
            }
            Ok(ClientMessage::Subscribe(Subscribe {
                subscription_id: id_field(&msg, "subscriptionId")?,
                thread_id: id_field(&msg, "threadId")?,
                cwd: validate_cwd(string_field(&msg, "cwd")?)?,
                filter,
            }))
        }
        other => Err(format!("unknown frame type `{}`", clip(other, 64))),
    }
}

/// Truncate to at most `max_units` UTF-16 code units without splitting
/// a character.
fn clip(s: &str, max_units: usize) -> String {
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
fn line(s: &str, max_units: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clip(flat.trim(), max_units)
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

struct Row {
    id: String,
    label: String,
    detail: Option<String>,
}

fn status_word(status: Status) -> &'static str {
    match status {
        Status::Identified => "identified",
        Status::Converging => "converging",
        Status::Achieved => "achieved",
        Status::SetAside => "set aside",
    }
}

fn row(file: &TargetsFile, id: &str, target: &Target, fanout: Option<usize>) -> Option<Row> {
    // T3 rejects the whole snapshot for an over-long id, so one
    // hand-edited absurd id must cost one row, not the panel.
    if id.is_empty() || utf16_len(id) > MAX_ID_UNITS {
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
    Some(Row {
        id: id.to_string(),
        label: line(&format!("🎯{id} {}", target.name), MAX_TEXT_UNITS),
        detail: Some(line(&parts.join(" · "), MAX_TEXT_UNITS)),
    })
}

fn rows_for(file: &TargetsFile, filter: PanelFilter) -> Vec<Row> {
    match filter {
        PanelFilter::Frontier => {
            let ready = frontier_tolerant(file).targets;
            rank_frontier(file, &ready)
                .into_iter()
                .filter_map(|r| {
                    let target = file.targets.get(&r.target.id)?;
                    row(file, &r.target.id, target, Some(r.fanout))
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
                    row(file, id, target, fanout)
                })
                .collect()
        }
    }
}

fn row_json(r: &Row) -> Value {
    match &r.detail {
        Some(d) if !d.is_empty() => json!({"id": r.id, "label": r.label, "detail": d}),
        _ => json!({"id": r.id, "label": r.label}),
    }
}

/// The panel for a ledger, or a one-row notice when it cannot be read.
fn panel_json(rows: Vec<Row>, filter: PanelFilter) -> Value {
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
        "id": PANEL_ID,
        "title": "Bullseye",
        "rows": shown,
        "filters": PanelFilter::ALL
            .iter()
            .map(|f| json!({"id": f.id(), "label": f.label()}))
            .collect::<Vec<_>>(),
        "filter": filter.id(),
    })
}

type LedgerRead = Result<Arc<TargetsFile>, String>;

/// Panels for one workspace. A directory without a ledger has nothing
/// to show, so it yields no panel at all (T3: an empty array removes
/// the section).
fn panels_for(
    cwd: &Path,
    filter: PanelFilter,
    ledgers: &mut HashMap<PathBuf, LedgerRead>,
) -> Vec<Value> {
    let Some(path) = store::discover_anywhere_readonly(cwd) else {
        return Vec::new();
    };
    let read = ledgers
        .entry(path.clone())
        .or_insert_with(|| store::load(&path).map(Arc::new).map_err(|e| e.to_string()));
    let rows = match read {
        Ok(file) => rows_for(file, filter),
        Err(e) => vec![Row {
            id: "error".into(),
            label: "Ledger unreadable".into(),
            detail: Some(line(e, MAX_TEXT_UNITS)),
        }],
    };
    vec![panel_json(rows, filter)]
}

fn snapshot_frame(subscription_id: &str, panels: Vec<Value>) -> String {
    json!({"type": "panels", "subscriptionId": subscription_id, "panels": panels}).to_string()
}

/// A subscription as the connection tracks it.
struct Sub {
    cwd: PathBuf,
    filter: PanelFilter,
    /// Digest of the last frame sent; `None` until the first.
    sent: Option<[u8; 32]>,
}

struct Job {
    id: String,
    cwd: PathBuf,
    filter: PanelFilter,
    sent: Option<[u8; 32]>,
}

/// Frames that differ from what each job last sent. Blocking: reads
/// ledgers and may run `git`. One ledger is read once per call however
/// many subscriptions share it.
fn changed_frames(jobs: Vec<Job>) -> Vec<(String, [u8; 32], String)> {
    let mut ledgers = HashMap::new();
    let mut out = Vec::new();
    for job in jobs {
        let frame = snapshot_frame(&job.id, panels_for(&job.cwd, job.filter, &mut ledgers));
        let digest: [u8; 32] = Sha256::digest(frame.as_bytes()).into();
        if job.sent != Some(digest) {
            out.push((job.id, digest, frame));
        }
    }
    out
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
            cwd: s.cwd.clone(),
            filter: s.filter,
            sent: s.sent,
        })
        .collect();
    let frames = tokio::task::spawn_blocking(move || changed_frames(jobs))
        .await
        .map_err(|_| ())?;
    for (id, digest, frame) in frames {
        debug_assert!(frame.len() <= MAX_FRAME_BYTES);
        match tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::text(frame))).await {
            Ok(Ok(())) => {}
            _ => return Err(()),
        }
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
                        // Re-subscribing an id replaces it (a filter change).
                        subs.insert(
                            req.subscription_id.clone(),
                            Sub { cwd: req.cwd, filter: req.filter, sent: None },
                        );
                        if refresh(&mut socket, &mut subs, Some(&req.subscription_id)).await.is_err() {
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
        let ClientMessage::Subscribe(s) = parse_client_message(&subscribe("")).unwrap() else {
            panic!("expected subscribe")
        };
        assert_eq!(s.filter, PanelFilter::Frontier);
        assert_eq!(s.cwd, PathBuf::from("/work/x"));
    }

    #[test]
    fn filter_selection_applies_only_to_this_panel() {
        let other = r#"{"pluginId":"other","panelId":"targets","filter":"nonsense"}"#;
        let ours = r#"{"pluginId":"bullseye","panelId":"targets","filter":"all"}"#;
        let msg = parse_client_message(&subscribe(&format!("{other},{ours}"))).unwrap();
        let ClientMessage::Subscribe(s) = msg else {
            panic!()
        };
        assert_eq!(s.filter, PanelFilter::All);
    }

    #[test]
    fn malformed_requests_are_rejected() {
        let ours =
            |f: &str| format!(r#"{{"pluginId":"bullseye","panelId":"targets","filter":{f}}}"#);
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
            (subscribe(r#"{"pluginId":"bullseye"}"#), "panelId"),
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
    fn clipping_counts_utf16_units_on_character_boundaries() {
        assert_eq!(clip("abcdef", 3), "abc");
        // 🎯 is two UTF-16 units: three of them do not fit in five.
        assert_eq!(clip("🎯🎯🎯", 5), "🎯🎯");
        assert_eq!(line("a\nb\tc\r", 10), "a b c");
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
        assert_eq!(ids(&rows_for(&file, PanelFilter::Frontier)), ["T1", "T10"]);
        assert_eq!(
            ids(&rows_for(&file, PanelFilter::Active)),
            ["T1", "T2", "T10"]
        );
        assert_eq!(
            ids(&rows_for(&file, PanelFilter::All)),
            ["T1", "T2", "T10", "T3", "T4"]
        );
    }

    #[test]
    fn rows_carry_label_and_context() {
        let file = sample();
        let rows = rows_for(&file, PanelFilter::All);
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

    #[test]
    fn oversized_ledgers_degrade_to_a_bounded_frame() {
        let rows: Vec<Row> = (0..MAX_ROWS + 500)
            .map(|i| Row {
                id: format!("T{i}"),
                label: "x".repeat(MAX_TEXT_UNITS),
                detail: Some("y".repeat(MAX_TEXT_UNITS)),
            })
            .collect();
        let frame = snapshot_frame("s", vec![panel_json(rows, PanelFilter::All)]);
        assert!(frame.len() <= MAX_FRAME_BYTES, "{}", frame.len());
        let v: Value = serde_json::from_str(&frame).unwrap();
        let shown = v["panels"][0]["rows"].as_array().unwrap();
        assert!(shown.len() <= MAX_ROWS);
        assert_eq!(shown.last().unwrap()["id"], "more");

        let few: Vec<Row> = (0..3)
            .map(|i| Row {
                id: format!("T{i}"),
                label: "l".into(),
                detail: None,
            })
            .collect();
        let panel = panel_json(few, PanelFilter::Active);
        assert_eq!(panel["rows"].as_array().unwrap().len(), 3);
        assert_eq!(panel["filter"], "active");
        assert_eq!(panel["filters"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn rows_with_unusable_ids_are_dropped_not_sent() {
        let mut file = sample();
        let t = file.targets.get("T1").unwrap().clone();
        file.targets.insert("x".repeat(MAX_ID_UNITS + 1), t);
        assert_eq!(
            ids(&rows_for(&file, PanelFilter::Active)),
            ["T1", "T2", "T10"]
        );
    }
}
