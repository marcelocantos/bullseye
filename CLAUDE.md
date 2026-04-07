# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Test

```bash
cargo build          # Build the project
cargo test           # Run all 44 tests
cargo test <name>    # Run a single test by name substring
cargo test --lib     # Unit tests only
cargo clippy         # Lint
cargo fmt --check    # Check formatting
```

Rust edition 2024, toolchain 1.94+.

## What This Is

**Bullseye** is an MCP (Model Context Protocol) server that manages **targets** — desired project states expressed as testable properties, ranked by weighted shortest job first (WSJF = value/cost).

Targets live in `docs/targets.yaml` (YAML source of truth) with an auto-rendered `docs/targets.md` markdown view. The server discovers the targets file by walking up from the caller's `cwd`.

Part of a planned **MCP triad**: targets (plan) + sawmill (code) + mnemo (history). See `docs/mcp-triad.md` for the integration design.

## Architecture

```
main.rs        — MCP server entry point (tokio + rust-mcp-sdk stdio transport)
schema.rs      — Core types: TargetsFile, Target, Status, Kind, GateEdge
store.rs       — YAML file I/O: discover, load, save
graph.rs       — Ranking (WSJF), frontier (unblocked leaves), tunnel detection,
                 validation, Mermaid graph rendering
ops.rs         — Rework cycle logic (verify failure → re-enter work target)
handler.rs     — MCP tool request dispatch (routes tool name → implementation)
tools.rs       — 12 MCP tool definitions via #[mcp_tool] macro
render.rs      — Markdown rendering of targets for docs/targets.md
```

**Data flow**: Every MCP tool call receives a `cwd` parameter → `store::discover()` finds `targets.yaml` → `store::load()` deserializes → operation applied → `store::save()` writes back + `render::render()` updates markdown.

**Key domain concepts**:
- Targets are hierarchical (T1 → T1.1, T1.2) via `parent` field
- `gates` edges express criticality-weighted blocking relationships
- `depends_on` edges express hard blocking dependencies
- Verify-kind targets (`kind: verify`) validate work targets via `verifies` edges
- Rework loops: when verification fails, `rework` edge re-enters a work target with retry budget

## Tests

All tests are integration tests in `tests/core_test.rs` using a fixture at `tests/fixtures/docs/targets.yaml`. Tests cover schema parsing, ranking, frontier, rework cycles, tunnel detection, validation, rendering, and YAML roundtrip.

## Delivery

Merged to master.
