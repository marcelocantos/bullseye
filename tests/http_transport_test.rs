// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! HTTP transport oracle (🎯T78).
//!
//! HTTP is the only MCP transport (🎯T81), so this test carries the
//! whole server surface: if a tool is registered but not advertised
//! here, it is not reachable by any agent at all. It refuses to take
//! that on trust and boots the shipped binary to read back what the
//! server actually advertises.
//!
//! Uses `curl` rather than an HTTP crate on purpose — enabling
//! `hyper-server` already cost 76 crates (docs/build-perf-2026-04-11.md),
//! and a test client has no business adding more.

use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_bullseye");

/// A port high enough to avoid the fleet's daemons (spyder 3030,
/// vellum-view 18742, bullseye's own default 18743).
const TEST_PORT: u16 = 18797;

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn post(port: u16, body: &str) -> String {
    post_with_session(port, body, None).0
}

/// POST to /mcp, optionally carrying a session id, returning
/// `(body, session_id_from_response_headers)`.
///
/// Streamable HTTP is session-oriented: `initialize` mints an
/// `Mcp-Session-Id` and every later call must present it, or the server
/// answers "Session not found". Discovered by this test failing that
/// way, which is exactly what it is for.
fn post_with_session(port: u16, body: &str, session: Option<&str>) -> (String, Option<String>) {
    let url = format!("http://127.0.0.1:{port}/mcp");
    let mut cmd = Command::new("curl");
    cmd.args([
        "-s",
        "-D",
        "-",
        "--max-time",
        "10",
        "-X",
        "POST",
        &url,
        "-H",
        "Content-Type: application/json",
        "-H",
        "Accept: application/json, text/event-stream",
    ]);
    if let Some(id) = session {
        cmd.args(["-H", &format!("Mcp-Session-Id: {id}")]);
    }
    cmd.args(["-d", body]);
    let out = cmd.output().expect("curl runs");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    let session_id = text
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("mcp-session-id:"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string());

    // Split headers from body at the blank line so assertions match on
    // payload rather than on header text.
    let body_text = match text.find("\r\n\r\n") {
        Some(i) => text[i + 4..].to_string(),
        None => match text.find("\n\n") {
            Some(i) => text[i + 2..].to_string(),
            None => text.clone(),
        },
    };
    (body_text, session_id)
}

fn start(port: u16) -> Server {
    let child = Command::new(BIN)
        .args(["serve", "--addr", &format!("127.0.0.1:{port}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn bullseye serve");
    let server = Server(child);

    // Wait for the listener rather than sleeping a fixed amount.
    for _ in 0..100 {
        if !post(port, r#"{"jsonrpc":"2.0","id":0,"method":"ping"}"#).is_empty() {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("bullseye serve never answered on port {port}");
}

fn initialize(port: u16) -> (String, String) {
    let (body, session) = post_with_session(
        port,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
        None,
    );
    let session = session.expect("initialize must mint an Mcp-Session-Id");
    // The spec requires this before any other request.
    post_with_session(
        port,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        Some(&session),
    );
    (body, session)
}

#[test]
fn http_transport_serves_mcp_and_advertises_every_registered_tool() {
    let port = TEST_PORT;
    let _server = start(port);

    let (init, session) = initialize(port);
    assert!(
        init.contains("\"name\":\"bullseye\""),
        "initialize should identify the server, got:\n{init}"
    );
    assert!(
        init.contains("protocolVersion"),
        "initialize should negotiate a protocol version, got:\n{init}"
    );

    // Every tool the binary registers must be reachable over HTTP.
    // With stdio gone there is no second path that could compensate for
    // a gap here.
    let (listed, _) = post_with_session(
        port,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
        Some(&session),
    );
    for tool in bullseye::tools::TargetTools::tools() {
        assert!(
            listed.contains(tool.name.as_str()),
            "tool `{}` is registered but not advertised over HTTP:\n{listed}",
            tool.name
        );
    }
    // And the write verb specifically, since that is the one 🎯T76 added.
    assert!(listed.contains("bullseye_apply"), "got:\n{listed}");
}

#[test]
fn serve_rejects_a_malformed_address_rather_than_binding_something_unexpected() {
    let out = Command::new(BIN)
        .args(["serve", "--addr", "127.0.0.1:not-a-port"])
        .output()
        .expect("runs");
    assert!(!out.status.success(), "malformed --addr must fail");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("HOST:PORT"), "got: {err}");
}

#[test]
fn serve_rejects_an_unrecognised_flag() {
    let out = Command::new(BIN)
        .args(["serve", "--not-a-flag", "1"])
        .output()
        .expect("runs");
    assert!(!out.status.success(), "unknown flag must fail");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unrecognised flag"),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A bare invocation is the shape a stale stdio registration uses. It
/// must fail, and it must fail *silently on stdout* (🎯T81): an MCP
/// client spawning this expects a handshake there, so usage text would
/// be read as protocol noise and hang the client waiting — a far worse
/// failure than a process that exits explaining what to do.
#[test]
fn a_bare_invocation_fails_loudly_on_stderr_and_writes_nothing_to_stdout() {
    let out = Command::new(BIN).output().expect("runs");
    assert!(!out.status.success(), "bare invocation must not succeed");
    assert!(
        out.stdout.is_empty(),
        "stdout must stay empty so an MCP client sees no protocol noise, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no longer speaks stdio"), "got: {err}");
    assert!(
        err.contains("bullseye serve"),
        "must name the replacement: {err}"
    );
    assert!(
        err.contains("/mcp"),
        "must name the endpoint to register: {err}"
    );
}
