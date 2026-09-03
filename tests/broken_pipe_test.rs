// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! A reader that goes away is not an error (🎯T77).
//!
//! Rust's runtime sets SIGPIPE to `SIG_IGN`, so a write to a closed
//! pipe returns EPIPE and `println!` panics. `bullseye query … | head`
//! then prints a panic and a backtrace note where every other Unix
//! tool exits quietly — and agents pipe CLI output into `head`/`grep`
//! constantly, so that panic reads as a bullseye failure rather than a
//! truncated read.
//!
//! The fixture is deliberately large. The original report could not be
//! reproduced because bullseye's own ledger renders ~38 KB, which fits
//! inside the 64 KiB pipe buffer — the write completes and EPIPE never
//! happens. A 1500-target ledger renders ~190 KB, three times the
//! buffer, which makes the failure deterministic rather than a race.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_bullseye");

/// Big enough that any view's output must exceed one pipe buffer.
fn big_ledger(dir: &Path) {
    let mut s = String::from("schema_version: 5\ntargets:\n");
    for i in 1..=1500 {
        s.push_str(&format!(
            "  T{i}:\n    name: target {i} with a name long enough to bulk out the rendered view\n\
             \x20   status: identified\n    value: 0.0\n    cost: 0.0\n    acceptance: [a]\n\
             \x20   context: {}\n    discovered: 2026-01-01\n",
            "padding ".repeat(20)
        ));
    }
    std::fs::write(dir.join("bullseye.yaml"), s).expect("write ledger");
}

/// Run a view, read a little, then drop the pipe — the shape `| head`
/// produces. Returns the child's stderr.
fn stderr_when_reader_closes_early(dir: &Path, view: &str) -> String {
    let mut child = Command::new(BIN)
        .args(["query", "--view", view, "--filter", "all", "--cwd"])
        .arg(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    {
        // Read one small chunk, then drop the handle. Dropping closes
        // the read end while the child still has ~190 KB to write.
        let mut out = child.stdout.take().expect("stdout");
        let mut buf = [0u8; 64];
        let _ = out.read(&mut buf);
    }

    let mut err = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut err)
        .expect("read stderr");
    let _ = child.wait();
    err
}

#[test]
fn a_reader_that_closes_early_produces_no_panic_on_any_view() {
    let dir = tempfile::tempdir().expect("tempdir");
    big_ledger(dir.path());

    for view in ["list", "summary", "context", "graph", "frontier"] {
        let err = stderr_when_reader_closes_early(dir.path(), view);
        assert!(
            !err.contains("panicked"),
            "view={view}: a closed reader must not panic, got:\n{err}"
        );
        assert!(
            !err.contains("RUST_BACKTRACE"),
            "view={view}: no backtrace note should reach a shell pipeline, got:\n{err}"
        );
    }
}

#[test]
fn the_fixture_actually_exceeds_one_pipe_buffer() {
    // Guards the test above from passing vacuously: if the rendered
    // output ever shrinks below the buffer the write completes, EPIPE
    // never occurs, and the panic assertion proves nothing.
    let dir = tempfile::tempdir().expect("tempdir");
    big_ledger(dir.path());
    let out = Command::new(BIN)
        .args(["query", "--view", "list", "--filter", "all", "--cwd"])
        .arg(dir.path())
        .output()
        .expect("runs");
    assert!(
        out.stdout.len() > 65_536,
        "fixture must render more than one 64 KiB pipe buffer, got {} bytes",
        out.stdout.len()
    );
}

/// The daemon survives abrupt client disconnects (🎯T77 / 🎯T78).
///
/// Read what this does and does not establish before trusting it.
///
/// It IS a real smoke test: 25 clients connect, send an in-flight
/// request, and hang up without reading, and the server must still be
/// running and accepting afterwards.
///
/// It is NOT an oracle for the CLI-only scoping of
/// `restore_default_sigpipe()`. That was the intent, and it failed:
/// hoisting the restore above the `serve` branch — so the daemon runs
/// with `SIG_DFL` — leaves this test passing. Verified by mutation, not
/// assumed. The likely reason is that tokio sets `SO_NOSIGPIPE` on its
/// sockets (or uses `MSG_NOSIGNAL`), so a failed socket write returns
/// EPIPE as an error rather than raising the signal at all, making the
/// disposition irrelevant on the server path.
///
/// So the scoping stays because it is the correct shape — a filter and
/// a server want opposite dispositions, and only the filter path needs
/// the change — not because a demonstrated hazard forced it. If someone
/// later finds a write path that does raise SIGPIPE in the server, this
/// is the test to strengthen.
#[test]
fn abrupt_client_disconnects_do_not_kill_the_server() {
    use std::io::Write;
    use std::net::TcpStream;

    const PORT: u16 = 18791;
    let addr = format!("127.0.0.1:{PORT}");

    let mut server = Command::new(BIN)
        .args(["serve", "--addr", &addr])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn serve");

    // Wait for the listener.
    let mut up = false;
    for _ in 0..100 {
        if TcpStream::connect(&addr).is_ok() {
            up = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(up, "server never listened on {addr}");

    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"rude","version":"1"}}}"#;
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );

    // Hang up without reading. Closing with unread data makes the kernel
    // send RST, so the server's write fails rather than filling a buffer
    // — the failure is provoked, not raced.
    for _ in 0..25 {
        if let Ok(mut s) = TcpStream::connect(&addr) {
            let _ = s.write_all(req.as_bytes());
            let _ = s.flush();
            drop(s);
        }
    }

    // The server must still be alive and answering.
    let alive = (0..50).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        TcpStream::connect(&addr).is_ok()
    });
    let exited = server.try_wait().expect("try_wait");
    let _ = server.kill();
    let _ = server.wait();

    assert!(
        exited.is_none(),
        "server exited after rude disconnects ({exited:?}) — SIGPIPE is probably SIG_DFL \
         on the serve path, which would kill the daemon whenever a client hangs up"
    );
    assert!(
        alive,
        "server stopped accepting connections after rude disconnects"
    );
}
