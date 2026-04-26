// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Priorities sync — write portfolio frontier targets into a SQLite
//! `targets_priorities` table for downstream replication to the
//! Protocol app via sqlpipe.
//!
//! See `docs/mcp-triad.md` §7 for the design. The schema is laptop-
//! owned (Protocol is read-only). A periodic cron job runs the
//! `bullseye sync-priorities` subcommand which scans the workspace
//! for `bullseye.yaml` files (same logic as `bullseye_portfolio`)
//! and upserts each frontier target's name, computed priority, and
//! context into the table. Stale rows whose targets are no longer on
//! a repo's frontier are deleted in the same transaction.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, ToSql, params};

use crate::config;
use crate::portfolio::{self, Momentum, PortfolioScan};

/// Default DB path: `$BULLSEYE_DATA_DIR/priorities.db` (or
/// `$HOME/.local/share/bullseye/priorities.db` if unset).
pub fn default_db_path() -> PathBuf {
    config::external_root().join("priorities.db")
}

const SCHEMA_SQL: &str = "\
CREATE TABLE IF NOT EXISTS targets_priorities (
    id          TEXT PRIMARY KEY,
    repo        TEXT NOT NULL,
    name        TEXT NOT NULL,
    priority    REAL NOT NULL,
    context     TEXT,
    horizon     TEXT NOT NULL DEFAULT 'today',
    updated_at  TEXT NOT NULL
);
";

/// Open (and create if needed) the priorities DB at `db_path`,
/// ensuring the schema exists. Creates parent directories as needed.
pub fn open(db_path: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(db_path)?;
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(conn)
}

/// Result of a sync run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncCounts {
    /// Rows inserted or updated.
    pub upserted: usize,
    /// Rows removed because they were no longer on any repo's frontier.
    pub deleted: usize,
}

/// Upsert all frontier targets from `scan` into the priorities table
/// at `horizon`, then delete any rows whose `id` is not in the current
/// set. Atomic: runs inside a single transaction.
///
/// `now` is the timestamp (RFC3339 string) written into `updated_at`.
/// Caller-supplied so tests can pin it; production code passes
/// `chrono::Utc::now().to_rfc3339()`.
///
/// The `id` column is `"{repo}/{target_id}"` — globally unique across
/// repos and stable across runs, so upsert preserves history.
/// `priority` is the per-target WSJF: `(value/cost) * momentum *
/// enabler_boost`. The Protocol app reads the table read-only and
/// orders by `priority` desc within a horizon bucket.
pub fn sync(
    conn: &mut Connection,
    scan: &PortfolioScan,
    horizon: &str,
    now: &str,
) -> rusqlite::Result<SyncCounts> {
    // Materialise the current entries so the transaction body holds
    // no borrows into `scan` while we run dynamic SQL for the delete.
    let mut entries: Vec<(String, &str, &str, f64, Option<&str>)> = Vec::new();
    for repo in &scan.repos {
        for ft in &repo.frontier_targets {
            let id = format!("{}/{}", repo.repo, ft.id);
            entries.push((
                id,
                repo.repo.as_str(),
                ft.name.as_str(),
                ft.wsjf,
                ft.context.as_deref(),
            ));
        }
    }

    let tx = conn.transaction()?;
    let mut upserted = 0usize;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO targets_priorities (id, repo, name, priority, context, horizon, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 repo = excluded.repo,
                 name = excluded.name,
                 priority = excluded.priority,
                 context = excluded.context,
                 horizon = excluded.horizon,
                 updated_at = excluded.updated_at",
        )?;
        for (id, repo, name, priority, ctx) in &entries {
            stmt.execute(params![id, repo, name, priority, ctx, horizon, now])?;
            upserted += 1;
        }
    }

    let deleted = if entries.is_empty() {
        tx.execute("DELETE FROM targets_priorities", [])?
    } else {
        let placeholders = vec!["?"; entries.len()].join(",");
        let sql = format!("DELETE FROM targets_priorities WHERE id NOT IN ({placeholders})");
        let ids: Vec<&str> = entries.iter().map(|(id, _, _, _, _)| id.as_str()).collect();
        let params_dyn: Vec<&dyn ToSql> = ids.iter().map(|s| s as &dyn ToSql).collect();
        tx.execute(&sql, &params_dyn[..])?
    };

    tx.commit()?;
    Ok(SyncCounts { upserted, deleted })
}

/// Parsed arguments to `bullseye sync-priorities`.
#[derive(Debug, Clone)]
pub struct SyncArgs {
    pub db_path: PathBuf,
    pub root: PathBuf,
    pub max_depth: usize,
    pub horizon: String,
}

impl SyncArgs {
    fn default_root() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/marcelo".to_string());
        PathBuf::from(home).join("work")
    }
}

/// Parse flags for the `sync-priorities` subcommand.
///
/// Recognised flags:
/// - `--db PATH` (default: `$BULLSEYE_DATA_DIR/priorities.db`)
/// - `--root PATH` (default: `~/work`)
/// - `--max-depth N` (default: 5)
/// - `--horizon STR` (default: `today`)
pub fn parse_sync_args(args: &[String]) -> Result<SyncArgs, String> {
    let mut db: Option<PathBuf> = None;
    let mut root: Option<PathBuf> = None;
    let mut horizon = "today".to_string();
    let mut max_depth: usize = 5;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--db" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--db requires a value".to_string())?;
                db = Some(config::expand_tilde(Path::new(v)));
                i += 2;
            }
            "--root" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--root requires a value".to_string())?;
                root = Some(config::expand_tilde(Path::new(v)));
                i += 2;
            }
            "--horizon" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--horizon requires a value".to_string())?;
                horizon = v.clone();
                i += 2;
            }
            "--max-depth" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--max-depth requires a value".to_string())?;
                max_depth = v
                    .parse()
                    .map_err(|_| format!("--max-depth must be a non-negative integer, got {v}"))?;
                i += 2;
            }
            "--help" | "-h" => return Err("HELP".to_string()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(SyncArgs {
        db_path: db.unwrap_or_else(default_db_path),
        root: root.unwrap_or_else(SyncArgs::default_root),
        max_depth,
        horizon,
    })
}

/// Top-level entry point for the `sync-priorities` subcommand. Returns
/// a one-line success message on success, or a string error.
pub fn run_sync(args: &[String]) -> Result<String, String> {
    let parsed = match parse_sync_args(args) {
        Ok(p) => p,
        Err(e) if e == "HELP" => return Ok(help_text()),
        Err(e) => return Err(e),
    };

    if !parsed.root.is_dir() {
        return Err(format!("{} is not a directory", parsed.root.display()));
    }

    let scan = portfolio::discover_repos(&parsed.root, parsed.max_depth, &[] as &[Momentum]);
    let mut conn =
        open(&parsed.db_path).map_err(|e| format!("open {}: {e}", parsed.db_path.display()))?;
    let now = chrono::Utc::now().to_rfc3339();
    let counts = sync(&mut conn, &scan, &parsed.horizon, &now).map_err(|e| format!("sync: {e}"))?;

    Ok(format!(
        "synced priorities → {}: {} upserted, {} deleted (horizon={})",
        parsed.db_path.display(),
        counts.upserted,
        counts.deleted,
        parsed.horizon,
    ))
}

fn help_text() -> String {
    "USAGE:\n    \
     bullseye sync-priorities [--db PATH] [--root PATH] [--max-depth N] [--horizon STR]\n\n\
     Scan the workspace for repos with bullseye.yaml, compute the\n\
     portfolio frontier, and upsert each frontier target into the\n\
     `targets_priorities` SQLite table at --db. Stale rows are\n\
     deleted in the same transaction. Designed for periodic\n\
     invocation from cron or a daemon hook.\n\n\
     FLAGS:\n    \
     --db PATH         SQLite path (default: $BULLSEYE_DATA_DIR/priorities.db)\n    \
     --root PATH       Workspace root to scan (default: ~/work)\n    \
     --max-depth N     Directory walk depth (default: 5)\n    \
     --horizon STR     Value written to `horizon` column (default: today)\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::{PortfolioFrontierTarget, RepoSummary};
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn ft(id: &str, name: &str, wsjf: f64, ctx: Option<&str>) -> PortfolioFrontierTarget {
        PortfolioFrontierTarget {
            id: id.to_string(),
            name: name.to_string(),
            value: 5.0,
            cost: 1.0,
            cross_enables_count: 0,
            momentum: 1.0,
            enabler_boost: 1.0,
            wsjf,
            context: ctx.map(String::from),
        }
    }

    fn repo(name: &str, frontier: Vec<PortfolioFrontierTarget>) -> RepoSummary {
        RepoSummary {
            repo: name.to_string(),
            path: PathBuf::from(format!("/work/{name}")),
            active: frontier.len(),
            frontier: frontier.len(),
            achieved: 0,
            wsjf_score: 0.0,
            frontier_targets: frontier,
            cross_depends: Vec::new(),
            cross_enables: Vec::new(),
        }
    }

    #[test]
    fn open_creates_schema() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("priorities.db");
        let conn = open(&db).unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(tables.contains(&"targets_priorities".to_string()));

        // Schema columns.
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(targets_priorities)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for expected in [
            "id",
            "repo",
            "name",
            "priority",
            "context",
            "horizon",
            "updated_at",
        ] {
            assert!(
                cols.iter().any(|c| c == expected),
                "missing column {expected} in {cols:?}",
            );
        }
    }

    /// Snapshot of one priorities row, used to keep test assertions
    /// readable without tripping clippy's `type_complexity` lint.
    #[derive(Debug)]
    struct Row {
        id: String,
        repo: String,
        name: String,
        priority: f64,
        context: Option<String>,
        horizon: String,
        updated_at: String,
    }

    fn read_all(conn: &Connection) -> Vec<Row> {
        conn.prepare(
            "SELECT id, repo, name, priority, context, horizon, updated_at \
             FROM targets_priorities ORDER BY id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok(Row {
                id: row.get(0)?,
                repo: row.get(1)?,
                name: row.get(2)?,
                priority: row.get(3)?,
                context: row.get(4)?,
                horizon: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    }

    #[test]
    fn sync_inserts_then_updates_then_deletes_stale() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("priorities.db");
        let mut conn = open(&db).unwrap();

        // Initial sync: two targets across two repos.
        let scan1 = PortfolioScan {
            repos: vec![
                repo("org/alpha", vec![ft("T1", "First", 5.0, Some("ctx1"))]),
                repo("org/beta", vec![ft("T2", "Second", 3.5, None)]),
            ],
            warnings: vec![],
        };
        let counts = sync(&mut conn, &scan1, "today", "2026-04-26T00:00:00Z").unwrap();
        assert_eq!(counts.upserted, 2);
        assert_eq!(counts.deleted, 0);

        let rows = read_all(&conn);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "org/alpha/T1");
        assert_eq!(rows[0].repo, "org/alpha");
        assert_eq!(rows[0].name, "First");
        assert!((rows[0].priority - 5.0).abs() < 1e-9);
        assert_eq!(rows[0].context.as_deref(), Some("ctx1"));
        assert_eq!(rows[0].horizon, "today");
        assert_eq!(rows[0].updated_at, "2026-04-26T00:00:00Z");
        assert_eq!(rows[1].id, "org/beta/T2");
        assert_eq!(rows[1].context, None);

        // Second sync: T1 changes priority + context, T2 disappears,
        // T3 appears in alpha. Expect: 2 upserted, 1 deleted.
        let scan2 = PortfolioScan {
            repos: vec![repo(
                "org/alpha",
                vec![
                    ft("T1", "First (renamed)", 7.0, Some("ctx1-updated")),
                    ft("T3", "Third", 4.0, None),
                ],
            )],
            warnings: vec![],
        };
        let counts = sync(&mut conn, &scan2, "today", "2026-04-26T01:00:00Z").unwrap();
        assert_eq!(counts.upserted, 2);
        assert_eq!(counts.deleted, 1);

        let ids: Vec<String> = conn
            .prepare("SELECT id FROM targets_priorities ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(ids, vec!["org/alpha/T1", "org/alpha/T3"]);

        let (name, priority, ctx, ts): (String, f64, Option<String>, String) = conn
            .query_row(
                "SELECT name, priority, context, updated_at FROM targets_priorities WHERE id = ?",
                params!["org/alpha/T1"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(name, "First (renamed)");
        assert!((priority - 7.0).abs() < 1e-9);
        assert_eq!(ctx.as_deref(), Some("ctx1-updated"));
        assert_eq!(ts, "2026-04-26T01:00:00Z");
    }

    #[test]
    fn sync_with_empty_scan_clears_table() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("priorities.db");
        let mut conn = open(&db).unwrap();

        // Seed a row.
        let scan = PortfolioScan {
            repos: vec![repo("org/alpha", vec![ft("T1", "X", 1.0, None)])],
            warnings: vec![],
        };
        sync(&mut conn, &scan, "today", "2026-04-26T00:00:00Z").unwrap();

        // Empty scan deletes everything.
        let counts = sync(
            &mut conn,
            &PortfolioScan::default(),
            "today",
            "2026-04-26T01:00:00Z",
        )
        .unwrap();
        assert_eq!(counts.upserted, 0);
        assert_eq!(counts.deleted, 1);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM targets_priorities", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn sync_id_is_repo_slash_target() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("priorities.db");
        let mut conn = open(&db).unwrap();

        // Two repos can both have a target named "T1" — `id` keeps them
        // distinct.
        let scan = PortfolioScan {
            repos: vec![
                repo("org/alpha", vec![ft("T1", "alpha-T1", 5.0, None)]),
                repo("org/beta", vec![ft("T1", "beta-T1", 3.0, None)]),
            ],
            warnings: vec![],
        };
        let counts = sync(&mut conn, &scan, "today", "2026-04-26T00:00:00Z").unwrap();
        assert_eq!(counts.upserted, 2);

        let ids: Vec<String> = conn
            .prepare("SELECT id FROM targets_priorities ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(ids, vec!["org/alpha/T1", "org/beta/T1"]);
    }

    #[test]
    fn parse_sync_args_defaults() {
        let args = parse_sync_args(&[]).unwrap();
        assert_eq!(args.horizon, "today");
        assert_eq!(args.max_depth, 5);
        assert!(args.db_path.ends_with("priorities.db"));
    }

    #[test]
    fn parse_sync_args_overrides() {
        let args = parse_sync_args(&[
            "--db".into(),
            "/tmp/p.db".into(),
            "--root".into(),
            "/work".into(),
            "--horizon".into(),
            "this_week".into(),
            "--max-depth".into(),
            "3".into(),
        ])
        .unwrap();
        assert_eq!(args.db_path, PathBuf::from("/tmp/p.db"));
        assert_eq!(args.root, PathBuf::from("/work"));
        assert_eq!(args.horizon, "this_week");
        assert_eq!(args.max_depth, 3);
    }

    #[test]
    fn parse_sync_args_unknown() {
        let err = parse_sync_args(&["--bogus".into()]).unwrap_err();
        assert!(err.contains("unknown argument"));
    }

    #[test]
    fn parse_sync_args_missing_value() {
        let err = parse_sync_args(&["--db".into()]).unwrap_err();
        assert!(err.contains("--db requires a value"));
    }
}
