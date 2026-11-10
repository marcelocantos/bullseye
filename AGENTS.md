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
make gate            # fmt + clippy + tests + declared checks; attests the tree for pre-push
make tla             # TLC on formal/BullseyeConvergence (~13–22 min)
                     # JDK: $TLC_JAVA, else macOS java_home -v 11+, else java on PATH
make hooks           # core.hooksPath=scripts/hooks

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
up from `cwd` (in-repo), then the external tree keyed by the `origin`
remote (`repo_identity.rs`), then the path-keyed shadow tree.

**Core agent surface** (prefer): `bullseye_open`, `bullseye_query`,
`bullseye_commit`, `bullseye_plan_checks`. Contract: `docs/api-v1-core.md`.
Legacy tool names are shims; portfolio/github/convergence/import/resolve
are extended (L2).

Part of a planned **MCP triad**: targets (plan) + sawmill (code) + mnemo
(history). See `docs/mcp-triad.md`.

## Architecture

```
main.rs / cli.rs       — MCP server entry point; CLI routes and parity
api.rs / apply.rs      — Mutation envelopes, partial desired-state apply
schema.rs / id_alloc.rs — Ledger types, migration and target ID allocation
store.rs / config.rs   — YAML discovery, locking, CAS and configuration
graph/                 — Frontier, validation, Mermaid and rendering
ops.rs / handler.rs    — Domain operations and MCP dispatch
tools.rs               — MCP tool definitions
portfolio.rs / resolve.rs / priorities.rs — Workspace discovery and priorities
convergence.rs         — Convergence checks and report
gatecheck.rs           — Cited gate/attestation validation at achieve time
github.rs / github_issues.rs — GitHub issue integrations
bounded.rs / cache.rs / repo_guard.rs — Bounds, caching and repo guardrails
repo_identity.rs       — Remote-derived repo identity for external ledgers
t3_plugin.rs           — Opt-in T3 panel WebSocket + probe at /plugins/t3 (🎯T114)
t3_discovery.rs        — Advertises that endpoint to T3 Code's plugin discovery
import.rs / version.rs — Markdown import and version support
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

HTTP is the only MCP transport (🎯T81): `bullseye serve [--addr HOST:PORT]`
serves `/mcp`, default `127.0.0.1:18743`, env `BULLSEYE_ADDR`. Agents reach
it through an mcpbridge `url` entry; supervisord owns the process.

A bare `bullseye` invocation is a stale stdio registration and exits
non-zero with migration instructions, writing nothing to stdout — usage
text there would be read as protocol noise and hang the client rather
than fail it.

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

**Process-global caches carry a TTL** (`cache::TTL`, 5 min, overridable
with `BULLSEYE_CACHE_TTL_SECS`). They were written for a process that
lived as long as one agent session; a daemon removed that ceiling. The
workspace scan in `resolve` has no other validation, so the TTL is its
correctness mechanism — without it a repo cloned after startup stays
invisible. The id-history and parse caches are already exact (a git ref
fingerprint and an mtime), so there the TTL only bounds memory.
Deliberately keyed on elapsed time rather than on client lifecycle:
MCP2 is stateless, so there will be no session whose end could carry an
invalidation.

**T3 panel endpoint (🎯T114, opt-in).** `bullseye serve --t3-plugin` or
`BULLSEYE_T3_PLUGIN=1` mounts a WebSocket at `/plugins/t3` on the same
listener (via `HyperServer::with_route`; no second port, no subprocess).
It speaks T3 Code's panel-plugin protocol v1 (`docs/operations/panel-plugins.md`
in the t3code repo): `subscribe`/`unsubscribe` in, complete `panels`
replacements out — one immediately, then one per observed change. Rows are
built from the graph (`graph::frontier_tolerant`/`rank_frontier`), not CLI
text; `frontier` is the default view, `active` and `all` are selectable.
Changes are found by a 1 s poll per connection because the CLI, other
agents and hand edits write ledgers outside this process. Read-only: it
uses `store::discover_anywhere_readonly`, which finds the same ledger as
MCP discovery but never performs the path-keyed → repo-keyed move.
Local-only by construction: startup refuses a non-loopback `--addr`, and
any request with an `Origin` header gets 403. Bounds: 1 MiB frames, 2000
rows, 16 connections, 128 subscriptions per connection; malformed frames
close the socket with 1008 (the protocol has no error frame). A plain GET
of the route (no `Upgrade`) is a context-free probe returning
`{"protocol":1,"id":"github.com/marcelocantos/bullseye","name":"Bullseye"}` plus an
`x-bullseye-instance` header. Discovery (`src/t3_discovery.rs`): once the
probe shows this process owns the listener, the daemon atomically writes
`$XDG_DATA_HOME/t3code/plugins/github.com+marcelocantos+bullseye.json` (home fallback
`~/.local/share`, relative XDG ignored; file 0600, dir 0700; no instance
suffix, so one slot where the latest publisher wins and the port lives
only in `baseUrl`; T3 would also accept `+<instance>`) and removes
it on SIGINT/SIGTERM and return, only if the file is still the one it
wrote (same device/inode/mtime and contents). The T3 side consents per user; T3 owns that UI and persistence,
Bullseye owns the manifest and probe. Supervised daemon:
`BULLSEYE_T3_PLUGIN=1 supervisor/install.sh` renders the variable into
`supervisor.d/bullseye.ini` (omit it and a re-render turns the endpoint
off). Tests: `tests/t3_plugin_test.rs` boots the binary with
`BULLSEYE_DATA_DIR`/`HOME` in a tempdir on a free port, mutates
ledgers with the real CLI and checks manifest publication, probe,
withdrawal and single-slot takeover; it runs under `make gate` via `cargo test`.

`--default-location` is process-wide, so under the daemon it is a
machine-wide default rather than a per-invocation choice.

A single daemon does **not** make bullseye a single writer: the CLI,
other repos' agents, and hand edits still mutate the file concurrently,
so the flock + CAS + `content_hash` machinery stays load-bearing.

## Delivery

Committed to master. Work lands as ordinary commits on `master` — no
pull requests, no merge commits, no CI gate.

`make gate` is the push oracle: fmt, clippy, tests, and declared ledger
checks. On success it writes `.git/gate-attestation` naming the tree it
measured (`scripts/gate-attest.sh`). `scripts/hooks/pre-push` does not
re-run the gate; it runs `cargo fmt --check` and refuses a push unless a
clean attestation names the tree of each pushed tip. So: commit, run
`make gate`, push. `make bullseye` is that plus the
in-repo TLA+ Convergence check (`./formal/check`, ~13–22 min) and the
advisory dirty-tree report. Nothing about "green" is expressible only
on a server. Each step prints its diagnostic on failure rather than a
bare exit code. `make hooks` sets `core.hooksPath=scripts/hooks`.

## Release

Local, from the dev Mac. There is no release workflow:

```bash
make release-dist   # dist/bullseye-<ver>-{darwin-arm64,linux-amd64,linux-arm64}.tar.gz
make release-tap    # push the formula to marcelocantos/homebrew-tap (release must exist)
make release        # gate + dist + gh release create + tap + brew upgrade + supervisord restart + verify
```

The Linux tarballs cross-compile here through `cargo-zigbuild`, which
supplies the cross linker and glibc headers the bundled SQLite needs.
**Cross builds require rustup's toolchain, not Homebrew's rust** —
Homebrew ships only the host target and sits earlier on `PATH`, so a
plain `cargo` reports "target may not be installed" for a target
`rustup target list` says is installed. `scripts/release-common.sh`
resolves `~/.cargo/bin/cargo` explicitly to avoid this.

Version bumps are MINOR-only and go through the `/release` skill.
