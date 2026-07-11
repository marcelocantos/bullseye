# Build performance audit — 2026-04-11

## Summary

| Metric | Before | After | Delta |
|---|---|---|---|
| CI `check` job (cold) | ~53s | ~53s | unchanged (populates cache) |
| CI `check` job (warm) | ~53s | **~22s** | **-31s (58% faster)** |
| Local clean debug build | 8.3–11.0s | 8.3–11.0s | unchanged (no local intervention) |
| Local no-op incremental | 0.04s | 0.04s | unchanged |
| Local one-file change | 0.46s | 0.46s | unchanged |

The only meaningful finding was that neither `ci.yml` nor `release.yml`
had any cargo build cache, so every run rebuilt the full dependency
graph (tokio, rust-mcp-schema, regex, chrono, the serde/proc-macro
stack) from scratch. Adding `Swatinem/rust-cache@v2` drops the warm-
cache path by ~31 seconds per run — more than half of the previous CI
wall time.

Local dev builds were already excellent — no intervention was needed
or made.

## Baseline

Machine: Apple M4 Max (16 cores), macOS, `cargo 1.94.1` / `rustc 1.94.1`.
Cold debug build via `cargo build --timings`.

### Critical path (clean debug, 8.32s wall)

```
rust-mcp-schema  (2.86s) ──┐
                           ├─ rust-mcp-transport (0.81s) ─ rust-mcp-sdk (0.94s) ─ bullseye lib (0.93s) ─ bullseye bin link (0.54s)
tokio            (2.14s) ──┘

wall = 8.32s    avg CPU = 59.9%    total CPU-seconds = 35.4
```

### Top 10 units by wall duration

| # | Crate | Duration | Target |
|---|---|---|---|
| 1 | `rust-mcp-schema v0.10.0` | 2.86s | lib |
| 2 | `tokio v1.50.0` | 2.14s | lib |
| 3 | `syn v2.0.117` | 1.29s | lib |
| 4 | `futures-util v0.3.32` | 1.10s | lib |
| 5 | `regex-automata v0.4.14` | 1.05s | lib |
| 6 | `num-traits` build script | 1.05s | run-custom-build |
| 7 | `thiserror` build script | 1.01s | run-custom-build |
| 8 | `rust-mcp-sdk v0.9.0` | 0.94s | lib |
| 9 | `bullseye v0.8.0` | 0.93s | lib |
| 10 | `libc` build script | 0.91s | run-custom-build |

Build scripts (11 total) account for ~6s cumulative CPU, but most run
in parallel. `syn v2.0.117` dominates proc-macro cost but isn't a
bottleneck.

### Measurement modes

| Mode | Wall | user CPU | sys CPU | Notes |
|---|---|---|---|---|
| `cargo clean && cargo build` | 8.3–11.0s | 31.8s | 3.7s | First run includes network; steady state is ~8.3s |
| `cargo clean && cargo test` | 10.4s | 35.3s | 4.5s | 57 unit + 11 portfolio tests; full compile + run |
| No-op `cargo build` (second run) | 0.04s | 0.02s | 0.01s | Incremental cache hit |
| Touch `src/schema.rs` + `cargo build` | 0.46s | 0.40s | 1.21s | bullseye lib + bin relink |

All measurements taken twice; incremental measurements report the
second (cache-warm) run per skill convention.

## Findings

### 1 — No cargo build cache in CI (High severity, Low risk) — **APPLIED**

**Location**: `.github/workflows/ci.yml:13-25`, `.github/workflows/release.yml:27-40`

Both workflows rebuilt the full dependency graph from scratch on every
run. Expected impact: 8–11s of compile time saved per warm run on the
`ci.yml` check job; ~5–15s saved per platform on release builds
(though release runs are rare).

**Fix**: add `Swatinem/rust-cache@v2` as a step after toolchain install
but before the first cargo invocation. For `release.yml`, pass
`key: ${{ matrix.target }}` so the three platform builds don't clobber
each other's caches.

**Risk**: Low. `Swatinem/rust-cache` keys on `Cargo.lock` + toolchain
version and is the de-facto standard Rust caching action for GitHub
Actions. Dependency changes auto-invalidate; stale caches fail closed
(forcing a rebuild) rather than producing stale artifacts.

### Nothing else

The following were checked against `patterns/cargo.md` and
`patterns/common.md` and found clean:

- ✅ No `[profile.dev]` misconfiguration (no `codegen-units = 1`, no
  LTO, no `opt-level = 3`; all defaults)
- ✅ No `cargo clean` in CI or scripts
- ✅ No `--test-threads=1` or serial test runners
- ✅ No `cargo incremental=0` override
- ✅ Incremental builds near-instantaneous (0.04s no-op, 0.46s edit)
- ℹ️ Single-crate project, but small enough that splitting into
  workspace crates wouldn't help (incremental is already fast)
- ℹ️ `sccache` not installed — not worth adding given that rust-cache
  on CI covers the shared-machine case and local incremental is
  already fast
- ℹ️ `syn`/`serde_derive` are on the critical path but not dominant;
  swapping to non-derived serde would hurt ergonomics without a clear
  win

## Applied

Commit `1058196` (PR #9) adds `Swatinem/rust-cache@v2` to both
workflows:

- `ci.yml`: single cache shared across fmt/clippy/test in the one job
- `release.yml`: per-target cache keyed on the matrix target triple
  (darwin-arm64, linux-amd64, linux-arm64)

### Before/after on `ci.yml`

Measured on GitHub Actions runners via two back-to-back PR runs on
branch `ci-rust-cache`:

| Stage | Run 1 (cold) | Run 2 (warm) | Delta |
|---|---|---|---|
| Total wall (job duration) | 53s | **22s** | **-31s (58% faster)** |
| Clippy | 17.31s | 3.83s | -13.48s (78% faster) |
| Test compile | 19.13s | 4.16s | -14.97s (78% faster) |

Warm clippy and test compile each drop to ~4s, which is the bullseye-
only incremental compile time — exactly matching the prediction. All
dependency builds are restored from the cache (~155 MB compressed,
saved at end of Run 1).

### Correctness verification

- Run 1 (cold cache populate): full test suite passes.
- Run 2 (cache restore): full test suite passes.
- Cache restore preserves correct artifacts — no stale-object or
  incremental-bug symptoms.

### `release.yml`

Not directly measured — release runs are infrequent, and the next
release will be the first validation. The change is mechanical and
structurally identical to `ci.yml` (with a per-target `key:` override),
so risk is identical.

## Deferred

None at the time of the 2026-04-11 pass.

## Follow-up — 2026-07-11

Re-measured cold debug build at **~13.7s** (up from ~8–11s). New
dominant unit: **libsqlite3-sys** (~6.2s) from `rusqlite` + `bundled`.
`tokio` with `features = ["full"]` remains on the critical path via
**rust-mcp-sdk**, not bullseye.

**Applied now:** bullseye's own `tokio` dep lists only
`macros`, `rt-multi-thread`, `sync`, `time`, `io-util` (no `full`).
Feature unification still enables `full` until the SDK stops declaring
it — tracked as 🎯T47.

**Filed for aggressive tuning (not applied):**

| Target | Idea |
|--------|------|
| 🎯T47 | SDK / patch so `tokio` `full` is not forced |
| 🎯T48 | Optional feature-gate bundled SQLite for slim local clean builds |
| 🎯T49 | Adopt sccache / mold / nextest / workspace split only past measured thresholds |

CI `Swatinem/rust-cache@v2` and local incremental (~0.05s no-op, ~1s
edit) remain the right defaults; no change there.

## Method

- **Build system**: single-crate cargo project. No workspace, no
  custom build.rs in bullseye itself (build scripts come from
  dependencies).
- **Profiling**: `cargo build --timings` (HTML output parsed via
  Python for ranking). `/usr/bin/time -p` for wall-clock measurement.
- **Pattern files consulted**: `~/.claude/skills/build-perf-audit/patterns/common.md`
  and `~/.claude/skills/build-perf-audit/patterns/cargo.md`.
- **CI validation**: PR #9 with two back-to-back runs (cold populate,
  warm restore) on a linux-amd64 ubuntu-latest runner.
- **Measurement notes**: each local incremental measurement was run
  twice; first run warms filesystem caches and is discarded. Clean
  builds measured once.
