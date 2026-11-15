// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! 🎯T117: config files drive the actual CLI, daemon listener, MCP creation
//! and T3 advertisement. Every HOME, port and ledger is disposable.

use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_bullseye");
const DEADLINE: Duration = Duration::from_secs(15);

fn command(home: &Path) -> Command {
    let mut cmd = Command::new(BIN);
    for key in [
        "BULLSEYE_CONFIG",
        "BULLSEYE_ADDR",
        "BULLSEYE_T3_PLUGIN",
        "BULLSEYE_DATA_DIR",
        "BULLSEYE_DEFAULT_LOCATION",
        "BULLSEYE_COMMIT",
        "BULLSEYE_INTEGRATION_REF",
        "BULLSEYE_CACHE_TTL_SECS",
        "XDG_CONFIG_HOME",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("HOME", home)
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .current_dir(home);
    cmd
}

fn config(home: &Path, text: &str) -> PathBuf {
    let path = home.join(".config/bullseye/config.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

fn port() -> u16 {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    assert_ne!(port, 18743, "never use the daily daemon's port");
    port
}

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(mut cmd: Command, expected_port: u16) -> Daemon {
    let mut child = Daemon(
        cmd.stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + DEADLINE;
    while TcpStream::connect(("127.0.0.1", expected_port)).is_err() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "daemon exited before binding configured port"
        );
        assert!(Instant::now() < deadline, "configured port did not open");
        std::thread::sleep(Duration::from_millis(25));
    }
    child
}

fn http(port: u16, path: &str, request: Option<&Value>, session: Option<&str>) -> (String, String) {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--max-time",
        "10",
        "-D",
        "-",
        &format!("http://127.0.0.1:{port}{path}"),
    ]);
    if let Some(request) = request {
        cmd.args([
            "-H",
            "Content-Type: application/json",
            "-H",
            "Accept: application/json, text/event-stream",
            "--data",
            &request.to_string(),
        ]);
    }
    if let Some(session) = session {
        cmd.args(["-H", &format!("Mcp-Session-Id: {session}")]);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "curl: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let (headers, body) = text.split_once("\r\n\r\n").unwrap();
    (headers.into(), body.into())
}

fn response(body: &str) -> Value {
    serde_json::from_str(
        body.lines()
            .find_map(|l| l.strip_prefix("data: "))
            .unwrap_or(body),
    )
    .unwrap()
}

#[test]
fn a_plain_serve_reads_the_default_file_and_serves_its_plugin_and_ledger_location() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let port = port();
    let cfg = config(
        home,
        &format!(
            "addr: 127.0.0.1:{port}\nt3_plugin: true\ndata_dir: ./ledgers\ndefault_location: external\ncache_ttl_secs: 0\n"
        ),
    );
    let _daemon = start(
        {
            let mut cmd = command(home);
            cmd.arg("serve");
            cmd
        },
        port,
    );
    let (headers, body) = http(port, "/plugins/t3", None, None);
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert_eq!(response(&body)["scopes"], json!(["thread", "environment"]));
    let manifest = home.join("xdg-data/t3code/plugins/github.com+marcelocantos+bullseye.json");
    let deadline = Instant::now() + DEADLINE;
    while !manifest.exists() {
        assert!(
            Instant::now() < deadline,
            "configured daemon did not advertise itself"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let advertised: Value = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    assert!(
        advertised
            .to_string()
            .contains(&format!("127.0.0.1:{port}"))
    );

    let (headers, _) = http(
        port,
        "/mcp",
        Some(
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"config-journey","version":"1"}}}),
        ),
        None,
    );
    let session = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .starts_with("mcp-session-id:")
                .then(|| line.split_once(':').unwrap().1.trim().to_string())
        })
        .unwrap();
    http(
        port,
        "/mcp",
        Some(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})),
        Some(&session),
    );
    let repo = home.join("project");
    std::fs::create_dir(&repo).unwrap();
    let (_, body) = http(
        port,
        "/mcp",
        Some(
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"bullseye_open","arguments":{"cwd":repo}}}),
        ),
        Some(&session),
    );
    let result = response(&body);
    assert!(result.get("error").is_none(), "{result}");
    assert_ne!(result["result"]["isError"], true, "{result}");
    let data = cfg.parent().unwrap().join("ledgers");
    let expected = bullseye::store::shadow_path(&data, &repo).join("bullseye.yaml");
    assert!(
        expected.exists(),
        "MCP creation ignored configured data_dir/default_location: {result}"
    );
    assert!(!repo.join("bullseye.yaml").exists());
    let cli = command(home)
        .args(["query", "--view", "frontier", "--format", "json", "--cwd"])
        .arg(&repo)
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let output: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(
        Path::new(output["file"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        expected.canonicalize().unwrap()
    );
}

#[test]
fn environment_overrides_file_and_command_line_overrides_both_with_false_supported() {
    let home = tempfile::tempdir().unwrap();
    config(home.path(), "addr: 127.0.0.1:1\nt3_plugin: true\n");
    let env_port = port();
    let _env = start(
        {
            let mut cmd = command(home.path());
            cmd.arg("serve")
                .env("BULLSEYE_ADDR", format!("127.0.0.1:{env_port}"))
                .env("BULLSEYE_T3_PLUGIN", "false");
            cmd
        },
        env_port,
    );
    assert!(
        http(env_port, "/plugins/t3", None, None)
            .0
            .starts_with("HTTP/1.1 404")
    );
    let cli_port = port();
    let _cli = start(
        {
            let mut cmd = command(home.path());
            cmd.env("BULLSEYE_ADDR", "127.0.0.1:1")
                .env("BULLSEYE_T3_PLUGIN", "false")
                .args([
                    "serve",
                    "--addr",
                    &format!("127.0.0.1:{cli_port}"),
                    "--t3-plugin",
                ]);
            cmd
        },
        cli_port,
    );
    assert!(
        http(cli_port, "/plugins/t3", None, None)
            .0
            .starts_with("HTTP/1.1 200")
    );
    let negative_port = port();
    let _negative = start(
        {
            let mut cmd = command(home.path());
            cmd.env("BULLSEYE_T3_PLUGIN", "true").args([
                "serve",
                "--addr",
                &format!("127.0.0.1:{negative_port}"),
                "--no-t3-plugin",
            ]);
            cmd
        },
        negative_port,
    );
    assert!(
        http(negative_port, "/plugins/t3", None, None)
            .0
            .starts_with("HTTP/1.1 404")
    );
}

#[test]
fn configuration_errors_are_actionable_and_can_be_bypassed() {
    let home = tempfile::tempdir().unwrap();
    config(home.path(), "t3_plugn: true\n");
    let bad = command(home.path())
        .args(["serve", "--print-addr"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    let error = String::from_utf8_lossy(&bad.stderr);
    assert!(
        error.contains("config.yaml") && error.contains("unknown field"),
        "{error}"
    );
    let bypass = command(home.path())
        .args(["serve", "--no-config", "--print-addr"])
        .output()
        .unwrap();
    assert!(bypass.status.success());
    assert_eq!(
        String::from_utf8_lossy(&bypass.stdout).trim(),
        bullseye::settings::DEFAULT_ADDR
    );
    let missing = command(home.path())
        .args(["--config", "missing.yaml", "serve", "--print-addr"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("missing.yaml"));
    assert!(
        command(home.path())
            .arg("--help")
            .output()
            .unwrap()
            .status
            .success()
    );
    for subcommand in ["serve", "query", "commit", "apply", "run-checks"] {
        let help = command(home.path())
            .args([subcommand, "--help"])
            .output()
            .unwrap();
        assert!(help.status.success(), "{subcommand}: {:?}", help.stderr);
        assert!(String::from_utf8_lossy(&help.stdout).contains(subcommand));
    }
}

#[test]
fn supervised_startup_does_not_bake_in_defaults_that_mask_the_file() {
    let home = tempfile::tempdir().unwrap();
    let port = port();
    config(
        home.path(),
        &format!("addr: 127.0.0.1:{port}\nt3_plugin: true\n"),
    );
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let run = |flag: Option<&str>| {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg(repo.join("supervisor/install.sh"));
        cmd.env_remove("BULLSEYE_ADDR")
            .env_remove("BULLSEYE_PORT")
            .env_remove("BULLSEYE_T3_PLUGIN")
            .env_remove("BULLSEYE_CONFIG")
            .env_remove("XDG_CONFIG_HOME");
        cmd.env("HOME", home.path())
            .env("BULLSEYE_BIN", BIN)
            .env("SUPERVISOR_SKIP_CTL", "1")
            .env("SUPERVISOR_CONF_DIR", home.path().join("supervisor.d"));
        if let Some(flag) = flag {
            cmd.env("BULLSEYE_T3_PLUGIN", flag);
        }
        let result = cmd.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let ini = std::fs::read_to_string(home.path().join("supervisor.d/bullseye.ini")).unwrap();
        ini.lines()
            .find(|line| line.starts_with("environment="))
            .unwrap()
            .to_string()
    };
    let environment = run(None);
    assert!(!environment.contains("BULLSEYE_ADDR="));
    assert!(!environment.contains("BULLSEYE_T3_PLUGIN="));
    assert!(run(Some("false")).contains("BULLSEYE_T3_PLUGIN=\"0\""));
    assert!(run(Some("TrUe")).contains("BULLSEYE_T3_PLUGIN=\"1\""));
    let out = command(home.path())
        .args(["serve", "--print-addr"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("127.0.0.1:{port}")
    );
}
