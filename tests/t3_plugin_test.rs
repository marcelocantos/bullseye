// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! T3 panel endpoint oracle (🎯T114).
//!
//! Boots the shipped binary and talks to `/plugins/t3` the way T3's
//! Node server does: a plain WebSocket with no `Origin`. Every daemon
//! runs on its own free loopback port with `BULLSEYE_DATA_DIR` and
//! `HOME` pointed into a tempdir, so nothing here can reach the real
//! external ledger root (🎯T102). Ledgers are mutated by the real CLI
//! in a separate process — the way an agent or a human would — never
//! through the daemon, which is exactly the case polling exists for.
//!
//! The same daemons are checked for what T3 Code's plugin discovery
//! relies on: the advertised manifest, the context-free probe, and
//! withdrawal of the manifest on exit.

use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::{HandshakeError, Message, WebSocket};

const BIN: &str = env!("CARGO_BIN_EXE_bullseye");

/// Longer than the daemon's 1 s poll, with room for a loaded machine.
const CHANGE_DEADLINE: Duration = Duration::from_secs(15);
/// Long enough for several polls to have passed unnoticed.
const QUIET_WINDOW: Duration = Duration::from_millis(2500);

// Reserve a free address through child bind; parallel launches must not reuse the released port.
static START_LOCK: Mutex<()> = Mutex::new(());

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

/// A daemon plus the sandbox it runs in.
struct Daemon {
    child: Child,
    port: u16,
    sandbox: TempDir,
}

impl Daemon {
    /// `args` follow `serve --addr 127.0.0.1:<port>`; `env` is extra.
    fn start(args: &[&str], env: &[(&str, &str)]) -> Daemon {
        let _startup = START_LOCK.lock().expect("startup lock");
        let sandbox = TempDir::new().expect("sandbox");
        let port = free_port();
        let mut cmd = Command::new(BIN);
        cmd.args(["serve", "--addr", &format!("127.0.0.1:{port}")])
            .args(args)
            .env_remove("BULLSEYE_T3_PLUGIN")
            .env_remove("XDG_DATA_HOME")
            .env("BULLSEYE_DATA_DIR", sandbox.path().join("data"))
            .env("HOME", sandbox.path())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(sandbox.path().join("daemon.log")).expect("log"));
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("spawn bullseye serve");
        let mut daemon = Daemon {
            child,
            port,
            sandbox,
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                daemon.child.try_wait().expect("child status").is_none(),
                "daemon exited before listening: {}",
                std::fs::read_to_string(daemon.sandbox.path().join("daemon.log"))
                    .unwrap_or_default()
            );
            assert!(Instant::now() < deadline, "daemon never listened");
            std::thread::sleep(Duration::from_millis(50));
        }
        daemon
    }

    fn data_dir(&self) -> PathBuf {
        self.sandbox.path().join("data")
    }

    /// A fresh directory inside the sandbox.
    fn dir(&self, name: &str) -> PathBuf {
        let dir = self.sandbox.path().join(name);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    /// An in-repo ledger holding targets named `T1..` in order.
    fn ledger(&self, name: &str, count: usize) -> PathBuf {
        let dir = self.dir(name);
        std::fs::write(
            dir.join("bullseye.yaml"),
            "schema_version: 5\ntargets: {}\n",
        )
        .expect("seed ledger");
        for n in 1..=count {
            self.track(&dir, &format!("T{n}"));
        }
        dir
    }

    /// Run the CLI against a workspace, as a separate process.
    fn cli(&self, cwd: &Path, args: &[&str]) {
        let out = Command::new(BIN)
            .arg(args[0])
            .args(&args[1..])
            .arg("--cwd")
            .arg(cwd)
            .env("BULLSEYE_DATA_DIR", self.data_dir())
            .env("HOME", self.sandbox.path())
            .output()
            .expect("run cli");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && !text.contains("\ncode="),
            "bullseye {args:?} failed:\n{text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Add target `id` (named after itself) through the CLI.
    fn track(&self, cwd: &Path, id: &str) {
        self.cli(
            cwd,
            &[
                "commit",
                "--op",
                "track",
                "--name",
                id,
                "--acceptance",
                "a",
                "--id",
                id,
            ],
        );
    }

    fn connect(&self) -> Client {
        Client::connect(self.port, None).unwrap_or_else(|error| {
            panic!(
                "handshake on {}: {error}; daemon log: {}",
                self.port,
                std::fs::read_to_string(self.sandbox.path().join("daemon.log")).unwrap_or_default()
            );
        })
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

enum Frame {
    Panels(Value),
    Closed(CloseCode),
    Quiet,
}

struct Client(WebSocket<TcpStream>);

impl Client {
    fn connect(port: u16, origin: Option<&str>) -> Result<Client, tungstenite::Error> {
        let mut req = format!("ws://127.0.0.1:{port}/plugins/t3")
            .into_client_request()
            .expect("request");
        if let Some(origin) = origin {
            req.headers_mut()
                .insert("Origin", origin.parse().expect("origin header"));
        }
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("tcp");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        match tungstenite::client(req, stream) {
            Ok((ws, _)) => Ok(Client(ws)),
            Err(HandshakeError::Failure(e)) => Err(e),
            Err(HandshakeError::Interrupted(_)) => panic!("handshake timed out"),
        }
    }

    fn send(&mut self, frame: &Value) {
        self.send_text(&frame.to_string());
    }

    fn send_text(&mut self, text: &str) {
        self.0.send(Message::text(text)).expect("send");
    }

    fn subscribe(&mut self, id: &str, thread: &str, cwd: &Path, filter: Option<&str>) {
        self.subscribe_as(CANONICAL_ID, id, thread, cwd, filter);
    }

    /// Subscribe addressing the panel by `plugin_id`, as T3 does with the id
    /// of the configuration the panel came from.
    fn subscribe_as(
        &mut self,
        plugin_id: &str,
        id: &str,
        thread: &str,
        cwd: &Path,
        filter: Option<&str>,
    ) {
        let filters: Vec<Value> = filter
            .map(|f| json!({"pluginId": plugin_id, "panelId": "targets", "filter": f}))
            .into_iter()
            .collect();
        self.send(&json!({
            "type": "subscribe", "protocol": 1, "subscriptionId": id,
            "threadId": thread, "cwd": cwd, "filters": filters,
        }));
    }

    fn unsubscribe(&mut self, id: &str) {
        self.send(&json!({"type": "unsubscribe", "subscriptionId": id}));
    }

    /// Next frame, or `Quiet` if none arrives within `wait`.
    fn next(&mut self, wait: Duration) -> Frame {
        self.0
            .get_ref()
            .set_read_timeout(Some(wait))
            .expect("timeout");
        loop {
            match self.0.read() {
                Ok(Message::Text(t)) => {
                    return Frame::Panels(serde_json::from_str(t.as_str()).expect("json frame"));
                }
                Ok(Message::Close(frame)) => {
                    return Frame::Closed(frame.map_or(CloseCode::Status, |f| f.code));
                }
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Frame::Quiet;
                }
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return Frame::Closed(CloseCode::Abnormal);
                }
                Err(e) => panic!("read failed: {e}"),
            }
        }
    }

    /// The next snapshot, which must arrive within the change deadline.
    fn snapshot(&mut self) -> Value {
        match self.next(CHANGE_DEADLINE) {
            Frame::Panels(v) => v,
            Frame::Closed(c) => panic!("closed ({c:?}) while waiting for a snapshot"),
            Frame::Quiet => panic!("no snapshot within {CHANGE_DEADLINE:?}"),
        }
    }

    /// Read snapshots until one for `sub` has exactly these row ids.
    fn await_rows(&mut self, sub: &str, want: &[&str]) -> Value {
        let deadline = Instant::now() + CHANGE_DEADLINE;
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            let frame = self.snapshot();
            if frame["subscriptionId"] == sub && row_ids(&frame) == want {
                return frame;
            }
            seen.push(row_ids(&frame));
        }
        panic!("never saw rows {want:?} for {sub}; saw {seen:?}");
    }

    fn assert_quiet(&mut self, what: &str) {
        match self.next(QUIET_WINDOW) {
            Frame::Quiet => {}
            Frame::Panels(v) => panic!("{what}: unexpected frame {v}"),
            Frame::Closed(c) => panic!("{what}: connection closed ({c:?})"),
        }
    }

    fn expect_close(&mut self, want: CloseCode) {
        match self.next(Duration::from_secs(10)) {
            Frame::Closed(code) => assert_eq!(code, want),
            Frame::Panels(v) => panic!("expected close, got frame {v}"),
            Frame::Quiet => panic!("expected close, got silence"),
        }
    }
}

fn row_ids(frame: &Value) -> Vec<String> {
    frame["panels"]
        .as_array()
        .and_then(|p| p.first())
        .and_then(|p| p["rows"].as_array())
        .map(|rows| {
            rows.iter()
                .map(|r| r["id"].as_str().expect("row id").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn http_status(err: tungstenite::Error) -> u16 {
    match err {
        tungstenite::Error::Http(resp) => resp.status().as_u16(),
        other => panic!("expected an HTTP rejection, got {other}"),
    }
}

/// Run `serve` args that must fail before binding; returns stderr.
fn serve_fails(args: &[&str], env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(BIN);
    cmd.arg("serve").args(args).env_remove("BULLSEYE_T3_PLUGIN");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run");
    assert!(!out.status.success(), "serve {args:?} should have failed");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn mcp_initialize(port: u16) -> String {
    let out = Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "10",
            "-X",
            "POST",
            &format!("http://127.0.0.1:{port}/mcp"),
            "-H",
            "Content-Type: application/json",
            "-H",
            "Accept: application/json, text/event-stream",
            "-d",
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
        ])
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn endpoint_is_off_by_default_and_mcp_is_unaffected() {
    let off = Daemon::start(&[], &[]);
    let err = Client::connect(off.port, None)
        .err()
        .expect("must not upgrade");
    assert_eq!(http_status(err), 404);
    assert!(mcp_initialize(off.port).contains("\"name\":\"bullseye\""));

    // An explicit 0 is still off.
    let zero = Daemon::start(&[], &[("BULLSEYE_T3_PLUGIN", "0")]);
    assert_eq!(
        http_status(Client::connect(zero.port, None).err().unwrap()),
        404
    );
}

#[test]
fn flag_and_env_each_enable_the_endpoint_beside_mcp() {
    for daemon in [
        Daemon::start(&["--t3-plugin"], &[]),
        Daemon::start(&[], &[("BULLSEYE_T3_PLUGIN", "1")]),
    ] {
        let mut client = daemon.connect();
        let ledger = daemon.ledger("ws", 1);
        client.subscribe("s1", "th1", &ledger, None);
        let frame = client.snapshot();
        assert_eq!(frame["type"], "panels");
        assert_eq!(row_ids(&frame), ["T1"]);
        assert!(mcp_initialize(daemon.port).contains("\"name\":\"bullseye\""));
    }
}

#[test]
fn startup_refuses_unsafe_or_unclear_configuration() {
    let err = serve_fails(&["--t3-plugin", "--addr", "0.0.0.0:18999"], &[]);
    assert!(err.contains("loopback"), "got: {err}");

    // A typo must not silently leave the panel missing.
    let err = serve_fails(&[], &[("BULLSEYE_T3_PLUGIN", "enable")]);
    assert!(err.contains("BULLSEYE_T3_PLUGIN"), "got: {err}");

    // Without the plugin a non-loopback --addr is the operator's existing choice;
    // only the pairing is refused (checked by the case above).
}

#[test]
fn browser_origins_are_rejected() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    for origin in ["http://evil.example", "null", "http://127.0.0.1:18743"] {
        let err = Client::connect(daemon.port, Some(origin))
            .err()
            .unwrap_or_else(|| panic!("Origin {origin} must be refused"));
        assert_eq!(http_status(err), 403, "origin {origin}");
    }
    // Refusal leaves the endpoint usable for the origin-less T3 server.
    daemon.connect();
}

#[test]
fn malformed_requests_close_the_connection() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let ledger = daemon.ledger("ws", 1);
    let good = json!({
        "type": "subscribe", "protocol": 1, "subscriptionId": "s", "threadId": "t",
        "cwd": ledger, "filters": [],
    });
    let with = |key: &str, value: Value| {
        let mut v = good.clone();
        v[key] = value;
        v
    };
    let bad_filter = with(
        "filters",
        json!([{"pluginId": CANONICAL_ID, "panelId": "targets", "filter": "bogus"}]),
    );
    let cases = [
        json!("not an object"),
        json!({"type": "explode"}),
        with("protocol", json!(2)),
        with("subscriptionId", json!("")),
        with("threadId", json!(5)),
        with("cwd", json!("relative/path")),
        with("cwd", json!("/a/../b")),
        with("filters", json!("frontier")),
        bad_filter,
    ];
    for case in cases {
        let mut client = daemon.connect();
        client.send(&case);
        client.expect_close(CloseCode::Policy);
    }

    let mut client = daemon.connect();
    client.send_text("{ not json");
    client.expect_close(CloseCode::Policy);

    let mut client = daemon.connect();
    client
        .0
        .send(Message::binary(vec![1u8, 2, 3]))
        .expect("send");
    client.expect_close(CloseCode::Unsupported);

    // One bad peer does not disturb another.
    let mut client = daemon.connect();
    client.subscribe("s", "t", &ledger, None);
    assert_eq!(row_ids(&client.snapshot()), ["T1"]);
}

#[test]
fn snapshots_follow_the_requested_filter_and_the_wire_shape() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let ws = daemon.ledger("ws", 3);
    // T1 blocks T2; T3 is independent; then T3 is set aside.
    daemon.cli(
        &ws,
        &["commit", "--op", "block", "--id", "T1", "--blocks", "T2"],
    );
    daemon.cli(
        &ws,
        &[
            "commit", "--op", "defer", "--id", "T3", "--reason", "parked",
        ],
    );

    let mut client = daemon.connect();
    client.subscribe("front", "th", &ws, None);
    client.subscribe("active", "th", &ws, Some("active"));
    client.subscribe("all", "th", &ws, Some("all"));

    let mut by_sub = std::collections::HashMap::new();
    for _ in 0..3 {
        let frame = client.snapshot();
        by_sub.insert(frame["subscriptionId"].as_str().unwrap().to_string(), frame);
    }
    assert_eq!(row_ids(&by_sub["front"]), ["T1"], "default is the frontier");
    assert_eq!(row_ids(&by_sub["active"]), ["T1", "T2"]);
    assert_eq!(row_ids(&by_sub["all"]), ["T1", "T2", "T3"]);

    let panel = &by_sub["all"]["panels"][0];
    assert_eq!(panel["kind"], "list");
    assert_eq!(panel["id"], "targets");
    assert_eq!(panel["title"], "Bullseye");
    assert_eq!(panel["filter"], "all");
    let filters: Vec<&str> = panel["filters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    assert_eq!(filters, ["frontier", "active", "all"]);
    assert_eq!(panel["rows"][0]["label"], "🎯T1 T1");
    assert_eq!(panel["rows"][2]["detail"], "set aside · parked");

    // Re-subscribing an id switches its view with an immediate snapshot.
    client.subscribe("front", "th", &ws, Some("all"));
    let switched = client.snapshot();
    assert_eq!(switched["subscriptionId"], "front");
    assert_eq!(row_ids(&switched), ["T1", "T2", "T3"]);

    // A hand-written configuration named `bullseye` addresses the same panel.
    client.subscribe_as("bullseye", "front", "th", &ws, Some("frontier"));
    let legacy = client.snapshot();
    assert_eq!(legacy["panels"][0]["filter"], "frontier");
    assert_eq!(row_ids(&legacy), ["T1"]);
}

#[test]
fn ledger_changes_made_elsewhere_refresh_only_what_changed() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let ws = daemon.ledger("ws", 2);
    let mut client = daemon.connect();
    client.subscribe("s", "th", &ws, None);
    assert_eq!(row_ids(&client.snapshot()), ["T1", "T2"]);

    // Unchanged ledger: polls happen, nothing is sent.
    client.assert_quiet("idle ledger");

    // The CLI (another process) edits the ledger; the panel follows.
    daemon.track(&ws, "T3");
    client.await_rows("s", &["T1", "T2", "T3"]);
    daemon.cli(
        &ws,
        &[
            "commit", "--op", "defer", "--id", "T2", "--reason", "parked",
        ],
    );
    client.await_rows("s", &["T1", "T3"]);
    client.assert_quiet("after settling");

    // A hand edit is caught too.
    let path = ws.join("bullseye.yaml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("name: T3", "name: renamed by hand")).unwrap();
    let frame = client.await_rows("s", &["T1", "T3"]);
    assert_eq!(
        frame["panels"][0]["rows"][1]["label"],
        "🎯T3 renamed by hand"
    );
}

#[test]
fn subscriptions_are_independent_per_thread_and_workspace() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let a = daemon.ledger("a", 1);
    let b = daemon.ledger("b", 2);
    let none = daemon.dir("no-ledger");

    let mut client = daemon.connect();
    client.subscribe("sa", "thread-a", &a, None);
    client.subscribe("sb", "thread-b", &b, Some("all"));
    client.subscribe("sn", "thread-n", &none, None);
    let mut first = std::collections::HashMap::new();
    for _ in 0..3 {
        let f = client.snapshot();
        first.insert(f["subscriptionId"].as_str().unwrap().to_string(), f);
    }
    assert_eq!(row_ids(&first["sa"]), ["T1"]);
    assert_eq!(row_ids(&first["sb"]), ["T1", "T2"]);
    // No ledger: an empty `panels` array, which removes the section.
    assert_eq!(first["sn"]["panels"], json!([]));

    // Changing A touches only A's subscription.
    daemon.track(&a, "T2");
    client.await_rows("sa", &["T1", "T2"]);
    client.assert_quiet("workspace B must not move");

    // Unsubscribing A stops its updates; B is unaffected.
    client.unsubscribe("sa");
    daemon.track(&a, "T3");
    daemon.track(&b, "T3");
    let frame = client.snapshot();
    assert_eq!(frame["subscriptionId"], "sb");
    assert_eq!(row_ids(&frame), ["T1", "T2", "T3"]);
    client.assert_quiet("unsubscribed A must stay silent");

    // Unsubscribing something unknown is harmless.
    client.unsubscribe("never-existed");
    client.assert_quiet("unknown unsubscribe");
}

#[test]
fn two_sockets_do_not_see_each_others_subscriptions() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let ws = daemon.ledger("ws", 1);
    let mut one = daemon.connect();
    let mut two = daemon.connect();
    one.subscribe("same-id", "th", &ws, None);
    assert_eq!(row_ids(&one.snapshot()), ["T1"]);
    two.assert_quiet("second socket never subscribed");
    two.unsubscribe("same-id");
    daemon.track(&ws, "T2");
    one.await_rows("same-id", &["T1", "T2"]);
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn external_ledgers_are_found_by_repo_identity_and_never_moved() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let remote = "https://github.com/example/t3-plugin-test.git";

    // Two clones of one remote share a repo-keyed external ledger.
    let clone_a = daemon.dir("clone-a");
    let clone_b = daemon.dir("clone-b");
    for clone in [&clone_a, &clone_b] {
        git(clone, &["init", "-q"]);
        git(clone, &["remote", "add", "origin", remote]);
    }
    daemon.cli(&clone_a, &["open", "--location", "external"]);
    assert!(
        daemon.data_dir().join("by-remote").is_dir(),
        "ledger should be repo-keyed"
    );
    let mut client = daemon.connect();
    client.subscribe("a", "th-a", &clone_a, Some("all"));
    client.subscribe("b", "th-b", &clone_b, Some("all"));
    let first = client.snapshot();
    let second = client.snapshot();
    assert!(!row_ids(&first).is_empty());
    assert_eq!(row_ids(&first), row_ids(&second));

    // A path-keyed ledger (created before the remote existed) is read
    // where it lies. MCP discovery would move it; this endpoint must not.
    let legacy = daemon.dir("legacy");
    git(&legacy, &["init", "-q"]);
    daemon.cli(&legacy, &["open", "--location", "external"]);
    git(
        &legacy,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/legacy-test.git",
        ],
    );
    let path_keyed: Vec<PathBuf> = walk_ledgers(&daemon.data_dir())
        .into_iter()
        .filter(|p| !p.starts_with(daemon.data_dir().join("by-remote")))
        .collect();
    assert_eq!(path_keyed.len(), 1, "expected one path-keyed ledger");
    client.subscribe("legacy", "th-l", &legacy, Some("all"));
    assert!(!row_ids(&client.await_snapshot_for("legacy")).is_empty());
    assert!(path_keyed[0].is_file(), "read-only endpoint moved a ledger");
}

fn walk_ledgers(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "bullseye.yaml") {
                found.push(path);
            }
        }
    }
    found
}

impl Client {
    /// Next snapshot for `sub`, skipping others.
    fn await_snapshot_for(&mut self, sub: &str) -> Value {
        loop {
            let frame = self.snapshot();
            if frame["subscriptionId"] == sub {
                return frame;
            }
        }
    }
}

#[test]
fn disconnects_release_connection_slots() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let mut held: Vec<Client> = (0..16).map(|_| daemon.connect()).collect();

    let err = Client::connect(daemon.port, None)
        .err()
        .expect("the 17th connection must be refused");
    assert_eq!(http_status(err), 503);

    // Closing one frees its slot (the daemon notices asynchronously).
    drop(held.pop());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match Client::connect(daemon.port, None) {
            Ok(client) => {
                held.push(client);
                break;
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("slot never freed: {e}"),
        }
    }
}

#[test]
fn subscriptions_per_connection_are_bounded_and_survivable() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let none = daemon.dir("no-ledger");
    let mut client = daemon.connect();
    for n in 0..128 {
        client.subscribe(&format!("s{n}"), "th", &none, None);
    }
    for _ in 0..128 {
        assert_eq!(client.snapshot()["panels"], json!([]));
    }
    // Re-subscribing an existing id is not a new subscription.
    client.subscribe("s0", "th", &none, Some("all"));
    assert_eq!(client.snapshot()["subscriptionId"], "s0");
    client.subscribe("s128", "th", &none, None);
    client.expect_close(CloseCode::Policy);
}

// ---- T3 Code plugin discovery -------------------------------------------

/// The plugin id a discovered Bullseye panel is addressed by.
const CANONICAL_ID: &str = "github.com/marcelocantos/bullseye";

const MANIFEST_DEADLINE: Duration = Duration::from_secs(15);

/// The directory discovery reads, under a `HOME` with no `XDG_DATA_HOME`.
fn manifest_dir_under(home: &Path) -> PathBuf {
    home.join(".local/share/t3code/plugins")
}

/// Manifest file name for the daemon on `port`: the canonical id with `/`
/// written as `+`, then the port as the instance.
fn manifest_name(port: u16) -> String {
    format!("github.com+marcelocantos+bullseye+{port}.json")
}

fn manifest_path(daemon: &Daemon) -> PathBuf {
    manifest_dir_under(daemon.sandbox.path()).join(manifest_name(daemon.port))
}

fn expected_manifest(port: u16) -> String {
    format!(
        "{{\"protocol\":1,\"id\":\"github.com/marcelocantos/bullseye\",\"name\":\"Bullseye\",\"connection\":{{\"kind\":\"server\",\"baseUrl\":\"http://127.0.0.1:{port}/plugins/t3\"}}}}\n"
    )
}

fn wait_for_file(path: &Path) -> String {
    let deadline = Instant::now() + MANIFEST_DEADLINE;
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn dir_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Plain HTTP GET with optional extra headers; returns status, lowercased
/// header lines and body.
fn http_get(port: u16, path: &str, extra: &[(&str, &str)]) -> (u16, String, String) {
    use std::io::{Read, Write};
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("tcp");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    let mut request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    for (k, v) in extra {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).expect("write");
    let mut raw = String::new();
    stream.read_to_string(&mut raw).expect("read");
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status");
    (status, head.to_ascii_lowercase(), body.to_string())
}

/// Ask the daemon to stop the way a supervisor does, and wait for exit.
fn terminate(child: &mut Child) {
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill");
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(40);
    while child.try_wait().expect("wait").is_none() {
        assert!(Instant::now() < deadline, "daemon ignored SIGTERM");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn disabled_endpoint_advertises_nothing_and_has_no_probe() {
    let daemon = Daemon::start(&[], &[]);
    let (status, _, _) = http_get(daemon.port, "/plugins/t3", &[]);
    assert_eq!(status, 404);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!manifest_dir_under(daemon.sandbox.path()).exists());
}

#[test]
fn enabled_daemon_advertises_the_agreed_manifest_and_answers_the_probe() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let path = manifest_path(&daemon);
    assert_eq!(wait_for_file(&path), expected_manifest(daemon.port));
    // Atomic publication leaves no temporary files behind.
    assert_eq!(
        dir_entries(path.parent().expect("dir")),
        [manifest_name(daemon.port)]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().expect("dir")), 0o700);
    }

    let (status, head, body) = http_get(daemon.port, "/plugins/t3", &[]);
    assert_eq!(status, 200);
    assert!(head.contains("content-type: application/json"), "{head}");
    assert_eq!(
        body,
        r#"{"protocol":1,"id":"github.com/marcelocantos/bullseye","name":"Bullseye"}"#
    );

    // A browser's request is refused, probe or not.
    let (status, _, _) = http_get(
        daemon.port,
        "/plugins/t3",
        &[("Origin", "http://evil.example")],
    );
    assert_eq!(status, 403);

    // The panel WebSocket is unaffected by the probe.
    let ledger = daemon.ledger("ws", 1);
    let mut client = daemon.connect();
    client.subscribe("s1", "th1", &ledger, None);
    assert_eq!(row_ids(&client.snapshot()), ["T1"]);
}

#[test]
fn a_broken_upgrade_is_rejected_rather_than_probed() {
    let daemon = Daemon::start(&["--t3-plugin"], &[]);
    let (status, _, body) = http_get(
        daemon.port,
        "/plugins/t3",
        &[("Connection", "Upgrade"), ("Upgrade", "websocket")],
    );
    assert_eq!(status, 400);
    assert!(!body.contains("\"protocol\""), "{body}");
}

#[test]
fn xdg_data_home_is_honoured_and_a_relative_one_is_ignored() {
    let xdg = TempDir::new().expect("xdg");
    let daemon = Daemon::start(
        &["--t3-plugin"],
        &[("XDG_DATA_HOME", xdg.path().to_str().expect("utf8"))],
    );
    let path = xdg
        .path()
        .join("t3code/plugins")
        .join(manifest_name(daemon.port));
    assert_eq!(wait_for_file(&path), expected_manifest(daemon.port));
    assert!(!manifest_dir_under(daemon.sandbox.path()).exists());

    let relative = Daemon::start(&["--t3-plugin"], &[("XDG_DATA_HOME", "relative/data")]);
    assert_eq!(
        wait_for_file(&manifest_path(&relative)),
        expected_manifest(relative.port)
    );
}

#[test]
fn termination_withdraws_the_manifest() {
    let mut daemon = Daemon::start(&["--t3-plugin"], &[]);
    let path = manifest_path(&daemon);
    wait_for_file(&path);
    // An open panel connection must not delay withdrawal.
    let _client = daemon.connect();
    terminate(&mut daemon.child);
    assert!(!path.exists(), "manifest survived a graceful shutdown");
}

#[test]
fn withdrawal_leaves_a_file_this_daemon_did_not_write() {
    let mut daemon = Daemon::start(&["--t3-plugin"], &[]);
    let path = manifest_path(&daemon);
    wait_for_file(&path);
    std::fs::write(&path, "somebody else's\n").expect("replace");
    terminate(&mut daemon.child);
    assert_eq!(
        std::fs::read_to_string(&path).expect("still there"),
        "somebody else's\n"
    );
}

#[test]
fn daemons_advertise_independently() {
    // Daemons on distinct ports share one directory without colliding.
    let shared = TempDir::new().expect("shared");
    let env = [("XDG_DATA_HOME", shared.path().to_str().expect("utf8"))];
    let mut a = Daemon::start(&["--t3-plugin"], &env);
    let b = Daemon::start(&["--t3-plugin"], &env);
    let dir = shared.path().join("t3code/plugins");
    wait_for_file(&dir.join(manifest_name(a.port)));
    wait_for_file(&dir.join(manifest_name(b.port)));
    terminate(&mut a.child);
    assert_eq!(dir_entries(&dir), [manifest_name(b.port)]);
}

#[test]
fn a_daemon_that_loses_the_port_neither_advertises_nor_withdraws() {
    let owner = Daemon::start(&["--t3-plugin"], &[]);
    let path = manifest_path(&owner);
    wait_for_file(&path);
    let before = std::fs::read_to_string(&path).expect("manifest");

    let mut loser = Command::new(BIN)
        .args([
            "serve",
            "--t3-plugin",
            "--addr",
            &format!("127.0.0.1:{}", owner.port),
        ])
        .env("HOME", owner.sandbox.path())
        .env_remove("XDG_DATA_HOME")
        .env_remove("BULLSEYE_T3_PLUGIN")
        .env("BULLSEYE_DATA_DIR", owner.data_dir())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn loser");
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = loser.try_wait().expect("wait") {
            break status;
        }
        assert!(Instant::now() < deadline, "second daemon never gave up");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!status.success(), "binding a taken port must fail");
    assert_eq!(std::fs::read_to_string(&path).expect("manifest"), before);
}
