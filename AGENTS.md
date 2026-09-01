# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Build and Test

```bash
cargo build          # Full product (default features include sqlite)
cargo test           # All tests (requires default features for priorities)
cargo test <name>    # Run a single test by name substring
cargo test --lib     # Unit tests only
cargo clippy         # Lint
cargo fmt --check    # Check formatting

# Slim clean rebuild without compiling bundled SQLite (~6s saved cold):
cargo build --no-default-features
# MCP tools work; sync-priorities needs --features sqlite (or defaults).
```

Rust edition 2024, toolchain 1.94+.

Build-performance notes and tooling thresholds: `docs/build-perf-2026-04-11.md`.
Tokio is feature-trimmed via `vendor/rust-mcp-sdk` + `vendor/rust-mcp-transport`
patches (`[patch.crates-io]` in `Cargo.toml`) — see `vendor/README.md`.

## What This Is

**Bullseye** is an MCP server that manages an **intent ledger**: desired
project states (targets) as testable properties, with dependency tracking,
frontier computation, and claim lifecycle (achieve / defer / reopen).

Targets live in `bullseye.yaml` (YAML source of truth). Discovery walks
up from `cwd` (in-repo) and the external shadow tree.

**Core agent surface** (prefer): `bullseye_open`, `bullseye_query`,
`bullseye_commit`, `bullseye_plan_checks`. Contract: `docs/api-v1-core.md`.
Legacy tool names are shims; portfolio/github/convergence/import/resolve
are extended (L2).

Part of a planned **MCP triad**: targets (plan) + sawmill (code) + mnemo
(history). See `docs/mcp-triad.md`.

## Architecture

```
main.rs        — MCP server + CLI (open/query/commit/plan-checks + L2)
api.rs         — Mutation envelopes and stable error codes (🎯T45)
schema.rs      — TargetsFile, Target, Status, …
store.rs       — YAML I/O, flock + CAS mutations
graph.rs       — Frontier, validation, Mermaid, startup context
handler.rs     — Tool dispatch
tools.rs       — MCP tool definitions
portfolio.rs   — Cross-repo discovery and portfolio summary
github.rs      — `bullseye github sync` issue mirror
```

**Surface parity**: Every capability must be reachable from **both**
surfaces — MCP tools and CLI — sharing one entry point. Core:
`bullseye_open` / `bullseye open`, etc. L2: `bullseye_github_sync` /
`bullseye github sync`, `bullseye_sync_priorities` /
`bullseye sync-priorities`.

**Data flow**: Every MCP tool call receives a `cwd` parameter → `store::discover()` finds `bullseye.yaml` → `store::load()` deserializes → operation applied → `store::save()` writes back.

**Key domain concepts**:
- Every target is structurally uniform — there is no `kind` field and
  no verify/work distinction. The acceptance criteria *are* the
  verification contract; whether the pass signal comes from CI, a
  human review, or a smoke test is free text on the acceptance field.
- `depends_on` edges express hard blocking dependencies. This is the
  single structural edge type. Legacy `gates` edges from older files
  are migrated into `depends_on` at load time
  (see `schema::migrate_gates_to_depends_on`).
- `bullseye_revert` moves an achieved target back to converging when
  a regression or new information shows the achievement was premature.

## Tests

Tests are split between unit tests in modules (`portfolio`, `import`) and integration tests in `tests/core_test.rs` using a fixture at `tests/fixtures/bullseye.yaml`. Tests cover schema parsing, frontier, validation, revert, startup context, portfolio discovery, and YAML roundtrip.

## Transport

Two transports, one handler and one `server_details` — the tool set is
identical by construction, not by convention (🎯T78).

| Transport | Start | Used by |
|---|---|---|
| HTTP (preferred) | `bullseye serve [--addr HOST:PORT]` — `/mcp`, default `127.0.0.1:18743`, env `BULLSEYE_ADDR` | supervisord; agents via an mcpbridge `url` entry |
| stdio | bare `bullseye` | direct spawners; fallback |

Loopback only, deliberately: the ledger is a local artifact and the
server carries no authentication of its own.

Install the daemon with `supervisor/install.sh`, which renders
`supervisor/bullseye.ini` into `supervisor.d`, evicts any other owner of
the port, and starts it. The program runs the **Homebrew** binary, so a
tree build never squats the shared port — every agent on the machine
talks to that one process, and unreleased tool schemas must not reach
them. To run unreleased code, set `BULLSEYE_BIN` and a different
`BULLSEYE_ADDR`.

Verify with `lsof -iTCP:18743 -sTCP:LISTEN`. **Do not probe `/mcp` with
bare `curl`** — MCP answers JSON-RPC POSTs, so a plain GET returns
nothing and reads as "server down".

A single daemon does **not** make bullseye a single writer: the CLI,
other repos' agents, and hand edits still mutate the file concurrently,
so the flock + CAS + `content_hash` machinery stays load-bearing.

## Delivery

Committed to master. Work lands as ordinary commits on `master` — no
pull requests, no merge commits, no CI gate.

`make bullseye` is the gate, and it is the whole definition of green:
`cargo fmt --check`, `cargo clippy --all-targets -D warnings`, and the
test suite. Nothing about "green" is expressible only on a server, so
it can always be reproduced on the machine in front of you. Each step
prints its diagnostic on failure rather than a bare exit code.

## Release

Local, from the dev Mac. There is no release workflow:

```bash
make release-dist   # dist/bullseye-<ver>-{darwin-arm64,linux-amd64,linux-arm64}.tar.gz
make release-tap    # push the formula to marcelocantos/homebrew-tap (release must exist)
make release        # gate + dist + gh release create + tap + brew upgrade + verify
```

The Linux tarballs cross-compile here through `cargo-zigbuild`, which
supplies the cross linker and glibc headers the bundled SQLite needs.
**Cross builds require rustup's toolchain, not Homebrew's rust** —
Homebrew ships only the host target and sits earlier on `PATH`, so a
plain `cargo` reports "target may not be installed" for a target
`rustup target list` says is installed. `scripts/release-common.sh`
resolves `~/.cargo/bin/cargo` explicitly to avoid this.

Version bumps are MINOR-only and go through the `/release` skill.
