// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Advertising the T3 panel endpoint for T3 Code's plugin discovery.
//!
//! While the endpoint is enabled ([`crate::t3_plugin`]), the daemon writes
//! one small JSON manifest into T3 Code's per-user plugin directory so a
//! T3 environment on the same machine can offer to enable the panel.
//! Nothing here trusts or executes anything: the manifest is a pointer to
//! a loopback URL, and T3 asks its user before it connects.
//!
//! **Location.** `$XDG_DATA_HOME/t3code/plugins/` when `XDG_DATA_HOME` is
//! set to an absolute path, else `$HOME/.local/share/t3code/plugins/` (a
//! relative `XDG_DATA_HOME` is ignored, as the XDG specification says).
//! The file is `github.com+marcelocantos+bullseye.json`: the identity
//! with `/` written as `+` (which never occurs inside an identity
//! segment, so the name decodes exactly). There is no instance suffix
//! (T3 accepts an optional `+<instance>`; Bullseye does not use one), so
//! the file is a single slot: the port appears only in `baseUrl`, and
//! the latest daemon to publish wins. T3 ignores a manifest whose name
//! does not match its `id`.
//!
//! **Contract (protocol 1).** Exactly this object, compact JSON:
//!
//! ```json
//! {"protocol":1,"id":"github.com/marcelocantos/bullseye","name":"Bullseye",
//!  "connection":{"kind":"server","baseUrl":"http://127.0.0.1:18743/plugins/t3"}}
//! ```
//!
//! `baseUrl` is the endpoint's own URL. A plain `GET` of it (no
//! `Upgrade`) returns
//! `{"protocol":1,"id":"github.com/marcelocantos/bullseye","name":"Bullseye"}`
//! and is how T3 checks, without sending any workspace or thread, that a
//! compatible service is really there. A `GET` that upgrades is the panel
//! WebSocket.
//!
//! **Lifecycle.** The manifest appears only once this process owns the
//! listener (proved by an instance header on its own probe, so a daemon
//! that lost the bind race never advertises or deletes another's file),
//! is written atomically (temp file in the same directory, then rename),
//! and is removed on SIGINT/SIGTERM and on normal return — but only if it
//! is still the very file this process renamed into place (device, inode,
//! mtime) with the contents it wrote, so a newer daemon's advertisement,
//! even one for the same endpoint, survives an older daemon's exit. A
//! killed daemon leaves a stale file; T3 discards manifests whose service
//! does not answer the probe.

use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::t3_plugin::{PATH, PLUGIN_ID};

/// Display name advertised to T3.
pub const PLUGIN_NAME: &str = "Bullseye";
/// Response header proving which process answered a probe.
pub const INSTANCE_HEADER: &str = "x-bullseye-instance";

const MANIFEST_PROTOCOL: u64 = 1;
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const READY_POLL: Duration = Duration::from_millis(50);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PROBE_BYTES: usize = 16 * 1024;

/// Random per-process token; not a secret, only a way to tell this
/// process's listener from another daemon's on the same port.
pub fn instance_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let digest = Sha256::digest(format!("{}:{nanos}:{:p}", std::process::id(), &TOKEN));
        digest[..8].iter().map(|b| format!("{b:02x}")).collect()
    })
}

/// T3 Code's plugin directory for the given environment, if one can be
/// determined.
pub fn manifest_dir(xdg_data_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_data_home {
        Some(dir) if Path::new(dir).is_absolute() => PathBuf::from(dir),
        _ => match home {
            Some(home) if !home.is_empty() => Path::new(home).join(".local/share"),
            _ => return None,
        },
    };
    Some(base.join("t3code").join("plugins"))
}

fn manifest_dir_from_env() -> Option<PathBuf> {
    manifest_dir(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// The manifest file name: the identity with `/` encoded as `+`. T3 also
/// accepts a trailing `+<instance>`; Bullseye does not use one, so the
/// file is a single slot and the latest publisher wins.
pub fn manifest_file_name() -> String {
    format!("{}.json", PLUGIN_ID.replace('/', "+"))
}

/// The manifest text for a daemon listening on `host:port`.
pub fn manifest_contents(host: &str, port: u16) -> String {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let authority = match bare.parse::<IpAddr>() {
        Ok(IpAddr::V6(ip)) => format!("[{ip}]:{port}"),
        _ => format!("{}:{port}", bare.to_ascii_lowercase()),
    };
    // Written by hand so the keys keep the documented order.
    let base_url = json!(format!("http://{authority}{PATH}"));
    format!(
        "{{\"protocol\":{MANIFEST_PROTOCOL},\"id\":\"{PLUGIN_ID}\",\"name\":\"{PLUGIN_NAME}\",\
         \"connection\":{{\"kind\":\"server\",\"baseUrl\":{base_url}}}}}\n"
    )
}

/// The body a probe of [`PATH`] returns.
pub fn probe_body() -> String {
    format!(
        "{{\"protocol\":{MANIFEST_PROTOCOL},\"id\":\"{PLUGIN_ID}\",\"name\":\"{PLUGIN_NAME}\"}}"
    )
}

/// One daemon's advertisement.
pub struct Advertisement {
    path: PathBuf,
    contents: String,
    /// Identity of the file this process renamed into place; `None` until
    /// published. Contents alone cannot prove ownership: a restarted daemon
    /// on the same endpoint writes identical bytes.
    published: Option<FileIdentity>,
}

/// Device, inode and modification time. A successor's publish is a new
/// temp file renamed over ours, so it differs in at least one of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FileIdentity {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    mtime: (i64, i64),
}

impl FileIdentity {
    fn of(meta: &fs::Metadata) -> FileIdentity {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            FileIdentity {
                dev: meta.dev(),
                ino: meta.ino(),
                mtime: (meta.mtime(), meta.mtime_nsec()),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = meta;
            FileIdentity {}
        }
    }
}

impl Advertisement {
    /// `None` when no data directory can be determined or the port is
    /// not a concrete one (port 0 cannot be advertised: the listener's
    /// real port is not observable).
    pub fn new(host: &str, port: u16) -> Option<Advertisement> {
        if port == 0 {
            return None;
        }
        let dir = manifest_dir_from_env()?;
        Some(Advertisement::in_dir(&dir, host, port))
    }

    fn in_dir(dir: &Path, host: &str, port: u16) -> Advertisement {
        Advertisement {
            path: dir.join(manifest_file_name()),
            contents: manifest_contents(host, port),
            published: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write the manifest atomically. Readers see the old file or the
    /// whole new one, never a partial write.
    pub fn publish(&mut self) -> std::io::Result<()> {
        let dir = self.path.parent().expect("manifest has a parent");
        create_private_dir(dir)?;
        let name = self.path.file_name().expect("manifest has a name");
        let temp = dir.join(format!(
            ".{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        // The temp file's identity survives the rename.
        let result = write_private(&temp, self.contents.as_bytes())
            .and_then(|()| fs::metadata(&temp))
            .and_then(|meta| {
                fs::rename(&temp, &self.path)?;
                Ok(FileIdentity::of(&meta))
            });
        match result {
            Ok(identity) => {
                self.published = Some(identity);
                Ok(())
            }
            Err(e) => {
                let _ = fs::remove_file(&temp);
                Err(e)
            }
        }
    }

    /// Remove the manifest if it is still the file this process wrote:
    /// same identity and same contents. A newer daemon's publish replaces
    /// the file, so an earlier daemon's shutdown leaves it alone.
    pub fn withdraw(&mut self) {
        let Some(identity) = self.published.take() else {
            return;
        };
        let ours = fs::metadata(&self.path).is_ok_and(|meta| FileIdentity::of(&meta) == identity)
            && fs::read_to_string(&self.path).is_ok_and(|now| now == self.contents);
        if ours {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Drop for Advertisement {
    fn drop(&mut self) {
        self.withdraw();
    }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Probe our own endpoint until it answers, and report whether the
/// answer carries this process's instance header.
async fn answered_by_us(host: &str, port: u16) -> Option<bool> {
    let connect = TcpStream::connect((host.trim_matches(|c| c == '[' || c == ']'), port));
    let mut stream = tokio::time::timeout(PROBE_TIMEOUT, connect)
        .await
        .ok()?
        .ok()?;
    let request = format!(
        "GET {PATH} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.ok()?;
    let mut buf = Vec::new();
    let read = async {
        let mut chunk = [0u8; 2048];
        while buf.len() < MAX_PROBE_BYTES {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
    };
    let _ = tokio::time::timeout(PROBE_TIMEOUT, read).await;
    let text = String::from_utf8_lossy(&buf).to_ascii_lowercase();
    let wanted = format!("{INSTANCE_HEADER}: {}", instance_token());
    Some(text.lines().any(|line| line.trim_end() == wanted))
}

/// Advertise once this process owns the listener at `host:port`, and
/// withdraw on SIGINT/SIGTERM. Runs until a signal arrives; the caller
/// also drops the returned guard on normal return.
///
/// Returns immediately with `None` when the daemon cannot advertise
/// (no data directory, ephemeral port). Failures to write are logged,
/// never fatal: the endpoint keeps working for a manually configured T3.
pub fn spawn_advertiser(
    host: &str,
    port: u16,
) -> Option<std::sync::Arc<std::sync::Mutex<Advertisement>>> {
    let Some(ad) = Advertisement::new(host, port) else {
        eprintln!(
            "bullseye: T3 panel endpoint is not advertised (no data directory or ephemeral port)"
        );
        return None;
    };
    let ad = std::sync::Arc::new(std::sync::Mutex::new(ad));
    let task_ad = std::sync::Arc::clone(&ad);
    let host = host.to_string();
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
        loop {
            match answered_by_us(&host, port).await {
                Some(true) => break,
                // Something else owns the port (or the probe was refused
                // outright): never advertise it.
                Some(false) => return,
                None if tokio::time::Instant::now() >= deadline => return,
                None => tokio::time::sleep(READY_POLL).await,
            }
        }
        let mut ad = task_ad.lock().unwrap_or_else(|e| e.into_inner());
        match ad.publish() {
            Ok(()) => eprintln!("bullseye: advertised T3 panel at {}", ad.path().display()),
            Err(e) => eprintln!("bullseye: could not advertise T3 panel ({e})"),
        }
    });
    let signal_ad = std::sync::Arc::clone(&ad);
    tokio::spawn(async move {
        wait_for_termination().await;
        signal_ad
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .withdraw();
    });
    Some(ad)
}

async fn wait_for_termination() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                term.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_prefers_absolute_xdg_and_falls_back_to_home() {
        assert_eq!(
            manifest_dir(Some("/data"), Some("/home/u")),
            Some(PathBuf::from("/data/t3code/plugins"))
        );
        // A relative XDG_DATA_HOME is invalid and ignored.
        assert_eq!(
            manifest_dir(Some("relative"), Some("/home/u")),
            Some(PathBuf::from("/home/u/.local/share/t3code/plugins"))
        );
        assert_eq!(
            manifest_dir(Some(""), Some("/home/u")),
            Some(PathBuf::from("/home/u/.local/share/t3code/plugins"))
        );
        assert_eq!(manifest_dir(None, None), None);
        assert_eq!(manifest_dir(None, Some("")), None);
    }

    #[test]
    fn file_name_encodes_the_identity_only() {
        assert_eq!(
            manifest_file_name(),
            "github.com+marcelocantos+bullseye.json"
        );
    }

    #[test]
    fn a_restarted_daemon_on_the_same_endpoint_is_not_removed_by_its_predecessor() {
        let dir = std::env::temp_dir().join(format!("bullseye-t3-ad-{}", instance_token()));
        let mut old = Advertisement::in_dir(&dir, "127.0.0.1", 18743);
        let mut new = Advertisement::in_dir(&dir, "127.0.0.1", 18743);
        old.publish().expect("old publishes");
        new.publish().expect("new publishes");
        // Byte-identical contents: only ownership tells the two apart.
        old.withdraw();
        assert!(
            old.path().exists(),
            "predecessor removed its successor's file"
        );
        new.withdraw();
        assert!(!new.path().exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_has_the_agreed_shape() {
        assert_eq!(
            manifest_contents("127.0.0.1", 18743),
            "{\"protocol\":1,\"id\":\"github.com/marcelocantos/bullseye\",\"name\":\"Bullseye\",\"connection\":{\"kind\":\"server\",\"baseUrl\":\"http://127.0.0.1:18743/plugins/t3\"}}\n"
        );
        assert!(manifest_contents("[::1]", 9).contains("http://[::1]:9/plugins/t3"));
        assert!(manifest_contents("LocalHost", 9).contains("http://localhost:9/plugins/t3"));
    }
}
