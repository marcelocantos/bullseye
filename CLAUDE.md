# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Test

```bash
cargo build          # Build the project
cargo test           # Run all tests
cargo test <name>    # Run a single test by name substring
cargo test --lib     # Unit tests only
cargo clippy         # Lint
cargo fmt --check    # Check formatting
```

Rust edition 2024, toolchain 1.94+.

## What This Is

**Bullseye** is an MCP (Model Context Protocol) server that manages **targets** — desired project states expressed as testable properties, with dependency tracking and frontier computation.

Targets live in `bullseye.yaml` (YAML source of truth). The server discovers the targets file by walking up from the caller's `cwd`.

Part of a planned **MCP triad**: targets (plan) + sawmill (code) + mnemo (history). See `docs/mcp-triad.md` for the integration design.

## Architecture

```
main.rs        — MCP server entry point (tokio + rust-mcp-sdk stdio transport)
schema.rs      — Core types: TargetsFile, Target, Status, GateEdge
store.rs       — YAML file I/O: discover, load, save
graph.rs       — Frontier (unblocked leaves), validation,
                 Mermaid graph rendering, startup context
handler.rs     — MCP tool request dispatch (routes tool name → implementation)
tools.rs       — MCP tool definitions via #[mcp_tool] macro
portfolio.rs   — Cross-repo discovery, scanning, and portfolio summary
github.rs      — `bullseye github sync` CLI: gh-based GitHub issue mirror
                 (🎯T34). Two-way: mirrors issues → GHI-<n> targets, and
                 reflects target lifecycle back to issues via the gh CLI.
```

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

## Delivery

Merged to master.
