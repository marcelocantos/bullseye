// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Hot-path benchmark and perf ratchet.
//!
//! ```text
//! cargo bench --bench hot_paths                  # full run, prints the baseline table
//! cargo bench --bench hot_paths -- --ratchet     # quick run, checked against docs/perf/baseline.md
//! cargo bench --bench hot_paths -- --filter frontier
//! ```
//!
//! Hand-rolled rather than criterion: the crate's build latency is a
//! product feature (`~/.claude/rust.md`), criterion drags in thirty-odd
//! crates, and the number we ratchet on is min-of-N wall time, which a
//! few lines of `Instant` give us with no statistics library.
//!
//! **Why min, not mean.** The gate runs on a developer Mac that may be
//! carrying other agents at the same time. Under load the mean drifts
//! with whatever else is running; the minimum is the best the code did
//! when it briefly had the machine to itself, and that is a property of
//! the code. It is also what a threshold can be tight about.
//!
//! **Ratchet, not floor.** `--ratchet` fails when a row is slower than
//! the recorded baseline by more than [`RATCHET_TOLERANCE`] *and* when
//! it is faster by more than that. A speed-up is welcome, but it has to
//! land as a deliberate edit to `docs/perf/baseline.md` in the same
//! commit, so the number never drifts silently in either direction.

#[path = "support/synth.rs"]
mod synth;

use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use bullseye::{graph, portfolio, schema::TargetsFile, store};

/// A row fails the ratchet when `measured / baseline` leaves
/// `[1 / (1 + tol), 1 + tol]`. Symmetric in log space so "twice as
/// fast" and "twice as slow" are the same distance from the baseline.
///
/// 0.5 is the observed noise envelope for min-of-N on the dev Mac with
/// a full agent fan-out running alongside (see `docs/perf/baseline.md`
/// for the measurement); a tighter bound flaps under load, a looser one
/// would let a 2× regression through.
const RATCHET_TOLERANCE: f64 = 0.5;

/// Where the locked numbers live, relative to the crate root.
const BASELINE_MD: &str = "docs/perf/baseline.md";

/// Fixed seed so every run generates byte-identical ledgers.
const SEED: u64 = 0x5EED_B011_5E1E;

/// Directory walk depth for the portfolio scan, matching the tool default.
const PORTFOLIO_MAX_DEPTH: usize = 5;

/// Synthetic ledger sizes for the single-file paths.
const SYNTH_SIZES: &[(&str, usize)] = &[
    ("synth-1k", 1_000),
    ("synth-5k", 5_000),
    ("synth-20k", 20_000),
];

/// Repo sizes for the workspace-shaped portfolio fixture: one big
/// ledger, a handful of mid-sized ones, and a long tail of small repos —
/// the distribution `~/work` actually has (83 repos, ~6.7k targets).
fn workspace_like_sizes() -> Vec<usize> {
    let mut sizes = vec![800, 300, 300, 280, 250, 200, 200, 150, 150, 120];
    sizes.extend(std::iter::repeat_n(100, 10));
    sizes.extend(std::iter::repeat_n(50, 20));
    sizes.extend(std::iter::repeat_n(20, 40));
    sizes
}

/// Ten repos of 2k targets each: the "every repo grew up" case.
fn big_workspace_sizes() -> Vec<usize> {
    vec![2_000; 10]
}

/// How long to keep sampling one row.
#[derive(Clone, Copy)]
struct Budget {
    min_iters: usize,
    max_iters: usize,
    wall: Duration,
}

impl Budget {
    const FULL: Budget = Budget {
        min_iters: 10,
        max_iters: 300,
        wall: Duration::from_secs(2),
    };
    /// Enough iterations for a stable minimum, few enough that the gate
    /// stays under a handful of seconds end to end.
    const QUICK: Budget = Budget {
        min_iters: 5,
        max_iters: 40,
        wall: Duration::from_millis(400),
    };
}

struct Sample {
    min: Duration,
    median: Duration,
    iters: usize,
}

/// Run `prepare` (untimed) then `f` (timed) until the budget is spent.
fn measure(budget: Budget, mut prepare: impl FnMut(), mut f: impl FnMut()) -> Sample {
    // One untimed pass so lazy statics and page cache are warm.
    prepare();
    f();
    let started = Instant::now();
    let mut times = Vec::with_capacity(budget.max_iters);
    while times.len() < budget.max_iters
        && (times.len() < budget.min_iters || started.elapsed() < budget.wall)
    {
        prepare();
        let t0 = Instant::now();
        f();
        times.push(t0.elapsed());
    }
    times.sort();
    Sample {
        min: times[0],
        median: times[times.len() / 2],
        iters: times.len(),
    }
}

/// One ledger on disk plus its parsed form.
struct Fixture {
    name: &'static str,
    path: PathBuf,
    file: TargetsFile,
}

/// One workspace root for the portfolio scan.
struct Workspace {
    name: &'static str,
    root: PathBuf,
    ledgers: Vec<PathBuf>,
    targets: usize,
}

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn build_fixtures(tmp: &Path) -> Vec<Fixture> {
    let mut out = Vec::new();
    for (name, n) in SYNTH_SIZES {
        let dir = tmp.join(name);
        std::fs::create_dir_all(&dir).expect("fixture dir");
        let path = dir.join("bullseye.yaml");
        let ledger = synth::ledger(
            SEED,
            &synth::Shape {
                targets: *n,
                other_repos: vec!["org/other".to_string()],
            },
        );
        store::save(&path, &ledger).expect("write synthetic ledger");
        // Reload so the in-memory form is exactly what the shipped path
        // sees after load-time migrations, not the generator's struct.
        let file = store::load(&path).expect("parse synthetic ledger");
        out.push(Fixture { name, path, file });
    }
    let jevons = crate_root().join("benches/fixtures/jevons/bullseye.yaml");
    let file = store::load(&jevons).expect("parse jevons fixture");
    out.push(Fixture {
        name: "jevons",
        path: jevons,
        file,
    });
    out
}

fn build_workspace(tmp: &Path, name: &'static str, sizes: &[usize]) -> Workspace {
    let root = tmp.join(name);
    let repo_names: Vec<String> = (0..sizes.len()).map(|i| format!("org/repo-{i}")).collect();
    let mut ledgers = Vec::new();
    let mut targets = 0;
    for (i, n) in sizes.iter().enumerate() {
        let dir = root.join("github.com").join(&repo_names[i]);
        std::fs::create_dir_all(&dir).expect("repo dir");
        let others: Vec<String> = repo_names
            .iter()
            .filter(|r| **r != repo_names[i])
            .cloned()
            .collect();
        let ledger = synth::ledger(
            SEED.wrapping_add(i as u64),
            &synth::Shape {
                targets: *n,
                other_repos: others,
            },
        );
        let path = dir.join("bullseye.yaml");
        store::save(&path, &ledger).expect("write workspace ledger");
        targets += ledger.targets.len();
        ledgers.push(path);
    }
    Workspace {
        name,
        root,
        ledgers,
        targets,
    }
}

/// Give every ledger a fresh, strictly increasing mtime so the parse
/// cache misses on the next scan. Set explicitly rather than touched
/// with "now": two iterations inside one clock tick would otherwise
/// share an mtime and the second would be served warm.
fn bump_mtimes(ws: &Workspace, tick: &mut u64) {
    *tick += 1;
    let stamp = UNIX_EPOCH + Duration::from_secs(1_700_000_000 + *tick);
    for p in &ws.ledgers {
        std::fs::File::options()
            .write(true)
            .open(p)
            .and_then(|f| f.set_modified(stamp))
            .expect("bump mtime");
    }
}

/// A measured row: `path/fixture`.
struct Row {
    path: &'static str,
    fixture: &'static str,
    targets: usize,
    sample: Sample,
}

fn run_rows(budget: Budget, filter: Option<&str>) -> Vec<Row> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let fixtures = build_fixtures(tmp.path());
    let workspaces = [
        build_workspace(tmp.path(), "ws-80", &workspace_like_sizes()),
        build_workspace(tmp.path(), "ws-20k", &big_workspace_sizes()),
    ];

    let wanted =
        |path: &str, fixture: &str| filter.is_none_or(|f| path.contains(f) || fixture.contains(f));
    let mut rows = Vec::new();

    for fx in &fixtures {
        let n = fx.file.targets.len();
        let mut push = |path: &'static str, sample: Sample| {
            rows.push(Row {
                path,
                fixture: fx.name,
                targets: n,
                sample,
            });
        };
        if wanted("parse", fx.name) {
            push(
                "parse",
                measure(
                    budget,
                    || {},
                    || {
                        black_box(store::load(&fx.path).expect("parse"));
                    },
                ),
            );
        }
        if wanted("render", fx.name) {
            push(
                "render",
                measure(
                    budget,
                    || {},
                    || {
                        black_box(store::render_file_text(&fx.file).expect("render"));
                    },
                ),
            );
        }
        if wanted("frontier", fx.name) {
            push(
                "frontier",
                measure(
                    budget,
                    || {},
                    || {
                        let tolerant = graph::frontier_tolerant(&fx.file);
                        black_box(graph::rank_frontier(&fx.file, &tolerant.targets));
                    },
                ),
            );
        }
        if wanted("validate", fx.name) {
            push(
                "validate",
                measure(
                    budget,
                    || {},
                    || {
                        black_box(graph::validate(&fx.file));
                    },
                ),
            );
        }
        if wanted("summary", fx.name) {
            push(
                "summary",
                measure(
                    budget,
                    || {},
                    || {
                        black_box(graph::summary(&fx.file, "bullseye.yaml", None, false));
                    },
                ),
            );
        }
        if wanted("context", fx.name) {
            push(
                "context",
                measure(
                    budget,
                    || {},
                    || {
                        black_box(graph::startup_context(&fx.file, "bullseye.yaml", 7));
                    },
                ),
            );
        }
        if wanted("mermaid", fx.name) {
            push(
                "mermaid",
                measure(
                    budget,
                    || {},
                    || {
                        black_box(graph::mermaid(&fx.file));
                    },
                ),
            );
        }
    }

    for ws in &workspaces {
        let mut tick = 0u64;
        if wanted("portfolio-cold", ws.name) {
            let sample = measure(
                budget,
                || bump_mtimes(ws, &mut tick),
                || {
                    black_box(portfolio::discover_repos(
                        &ws.root,
                        PORTFOLIO_MAX_DEPTH,
                        &[],
                    ));
                },
            );
            rows.push(Row {
                path: "portfolio-cold",
                fixture: ws.name,
                targets: ws.targets,
                sample,
            });
        }
        if wanted("portfolio-warm", ws.name) {
            // First scan after the last bump fills the cache; `measure`'s
            // untimed warm-up pass does that.
            let sample = measure(
                budget,
                || {},
                || {
                    black_box(portfolio::discover_repos(
                        &ws.root,
                        PORTFOLIO_MAX_DEPTH,
                        &[],
                    ));
                },
            );
            rows.push(Row {
                path: "portfolio-warm",
                fixture: ws.name,
                targets: ws.targets,
                sample,
            });
        }
    }
    rows
}

fn micros(d: Duration) -> u64 {
    d.as_micros() as u64
}

fn print_table(rows: &[Row]) {
    println!("| path | fixture | targets | min µs | median µs | iters |");
    println!("|---|---|---:|---:|---:|---:|");
    for r in rows {
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            r.path,
            r.fixture,
            r.targets,
            micros(r.sample.min),
            micros(r.sample.median),
            r.sample.iters,
        );
    }
}

/// Parse the `| path | fixture | targets | min µs | ... |` table out of
/// the baseline document. Anything that is not a table row is prose and
/// skipped, so the document can carry as much explanation as it likes.
fn read_baseline(path: &Path) -> BTreeMap<(String, String), u64> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let cells: Vec<&str> = line
            .trim()
            .strip_prefix('|')
            .and_then(|l| l.strip_suffix('|'))
            .map(|l| l.split('|').map(str::trim).collect())
            .unwrap_or_default();
        if cells.len() < 4 {
            continue;
        }
        let Ok(min) = cells[3].parse::<u64>() else {
            continue;
        };
        out.insert((cells[0].to_string(), cells[1].to_string()), min);
    }
    out
}

fn ratchet(rows: &[Row]) -> bool {
    let baseline_path = crate_root().join(BASELINE_MD);
    let baseline = read_baseline(&baseline_path);
    let lo = 1.0 / (1.0 + RATCHET_TOLERANCE);
    let hi = 1.0 + RATCHET_TOLERANCE;
    let mut ok = true;
    println!("| path | fixture | baseline µs | now µs | ratio | verdict |");
    println!("|---|---|---:|---:|---:|---|");
    for r in rows {
        let key = (r.path.to_string(), r.fixture.to_string());
        let now = micros(r.sample.min);
        let Some(&base) = baseline.get(&key) else {
            ok = false;
            println!(
                "| {} | {} | — | {now} | — | ✗ no baseline row |",
                r.path, r.fixture
            );
            continue;
        };
        let ratio = now as f64 / base.max(1) as f64;
        let verdict = if ratio > hi {
            ok = false;
            "✗ slower than baseline"
        } else if ratio < lo {
            ok = false;
            "✗ faster than baseline — update docs/perf/baseline.md"
        } else {
            "✓"
        };
        println!(
            "| {} | {} | {base} | {now} | {ratio:.2} | {verdict} |",
            r.path, r.fixture
        );
    }
    for (path, fixture) in baseline.keys() {
        if !rows.iter().any(|r| r.path == path && r.fixture == fixture) {
            println!("| {path} | {fixture} | — | — | — | ⚠ baseline row not measured |");
        }
    }
    ok
}

fn main() {
    // `cargo bench` passes `--bench` through; everything else is ours.
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--bench")
        .collect();
    let mut do_ratchet = false;
    let mut filter: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ratchet" => do_ratchet = true,
            "--filter" => {
                i += 1;
                filter = args.get(i).cloned();
            }
            other => {
                eprintln!("unknown argument {other}");
                eprintln!("usage: cargo bench --bench hot_paths -- [--ratchet] [--filter SUBSTR]");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let budget = if do_ratchet {
        Budget::QUICK
    } else {
        Budget::FULL
    };
    let started = Instant::now();
    let rows = run_rows(budget, filter.as_deref());
    if do_ratchet {
        let ok = ratchet(&rows);
        eprintln!(
            "perf ratchet: {} rows in {:.1}s",
            rows.len(),
            started.elapsed().as_secs_f64()
        );
        if !ok {
            std::process::exit(1);
        }
    } else {
        print_table(&rows);
        eprintln!(
            "full bench: {} rows in {:.1}s",
            rows.len(),
            started.elapsed().as_secs_f64()
        );
    }
}
