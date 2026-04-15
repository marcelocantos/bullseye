# Bullseye

An MCP server for managing **targets** — desired project states
expressed as testable properties, with dependency tracking and
frontier computation.

Targets live in `bullseye.yaml` (source of truth). Storage is
machine-wide configurable — bullseye either reads `bullseye.yaml`
in-repo (walking up from the caller's working directory) or stores it
in an external shadow tree keyed to the cwd's absolute path. See
[Storage modes](#storage-modes) below.

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
# Binary at target/release/bullseye
```

## MCP client configuration

Add to `.mcp.json` (project scope) or `~/.claude.json` (user scope):

```json
{
  "mcpServers": {
    "bullseye": {
      "command": "bullseye",
      "args": []
    }
  }
}
```

Or via the CLI:

```bash
claude mcp add --scope user bullseye -- bullseye
```

The server communicates over stdio using the MCP protocol.

## Storage modes

On first use, bullseye requires a one-time choice recorded at
`~/.config/bullseye/config.yaml`:

| Mode | Where `bullseye.yaml` lives | Use when |
|------|-----------------------------|----------|
| `in_repo` | Inside the repo, discovered by walking up from `cwd`. | You own the repo and the team has adopted bullseye. |
| `external` | Shadow tree under `~/.local/share/bullseye/` mirroring the cwd's absolute path. Discovery walks up the shadow tree the same way `in_repo` walks up the real tree. | Repo is read-only to you (corporate repos where bullseye isn't adopted), or targets are personal to you. |

External mode is purely path-driven — no assumptions about git
remotes or `host/org/repo` layouts — so monorepos, non-git
directories, and unconventional workspaces all resolve identically.

Until configured, every tool call returns an actionable error
instructing the agent to ask the user and then call
`bullseye_configure`. Configure explicitly:

```
# Team-adopted repo (commits bullseye.yaml alongside code):
bullseye_configure mode=in_repo

# Personal or read-only use (shadow tree under ~/.local/share/bullseye/):
bullseye_configure mode=external
# or with a custom root:
bullseye_configure mode=external root=/path/to/data
```

The config file is small, hand-editable YAML, and can be version-
controlled (`yadm add`, `stow`, `chezmoi`, etc.) if you want the
mode to follow you across machines.

## Tools

Bullseye exposes 18 MCP tools. All target-operating tools accept a
`cwd` parameter to locate the nearest `bullseye.yaml` under the
configured storage mode.

| Tool | Description |
|------|-------------|
| `bullseye_list` | List targets (active/achieved/all) |
| `bullseye_get` | Get a single target by ID with full detail |
| `bullseye_put` | Upsert a target — create (auto- or explicit ID) or patch in one call. Rejects content patches on achieved targets unless re-opened in the same call. |
| `bullseye_retire` | Mark a target achieved |
| `bullseye_verify` | Emit a structured plan that maps each of a target's `checks` to a sawmill tool invocation. Plan-only — the calling agent runs the checks and folds results back. |
| `bullseye_frontier` | Unblocked leaf targets ready for work, ordered by distance-to-nearest-observable-checkpoint then unblocking fanout |
| `bullseye_rework` | Trigger rework from a failed verification |
| `bullseye_tunnels` | Detect work targets with no observable checkpoint reachable within N hops |
| `bullseye_validate` | Check schema conformance |
| `bullseye_graph` | Generate Mermaid dependency graph |
| `bullseye_import` | Import targets from markdown into YAML |
| `bullseye_init` | Create starter bullseye.yaml with sample target |
| `bullseye_startup_context` | Session startup context (frontier, recent achievements, warnings) |
| `bullseye_portfolio` | Cross-repo portfolio summary with frontier targets, including cross-repo edges |
| `bullseye_summary` | Consolidated status overview: groups, frontier ordered by distance + fanout, blocked, stale |
| `bullseye_convergence` | End-to-end convergence evaluation: runs `make bullseye` for invariants, scans git for unreleased fixes, emits summary with frontier detail inline, and computes a deterministic next-action recommendation. Single call, replaces the old multi-tool `/cv` worker. |
| `bullseye_configure` | Record the one-time storage-mode choice (`in_repo` or `external`, with optional external `root`). Writes `~/.config/bullseye/config.yaml`. |

See [agents-guide.md](docs/agents-guide.md) for detailed tool
parameters, the bullseye.yaml schema, usage workflows, and a
[copy-pasteable CLAUDE.md snippet](docs/agents-guide.md#agent-integration)
for wiring Bullseye into your project's agent instructions.

## Key concepts

- **Frontier**: The set of unblocked leaf targets that can be worked
  on right now, in parallel.
- **Observable targets**: Work targets that produce something the
  human decision-maker can look at and react to — a checkpoint.
  Marked with `observable: true`. Verify-kind targets are
  observable by definition. Repo-level frontier ordering rewards
  shortest-path-to-nearest-observable-target, so chains of opaque
  work targets get flagged rather than ordered.
- **Tunnels**: Work targets with no observable checkpoint reachable
  within N hops — a signal to reshape the graph by adding an
  intermediate observable target or promoting an existing work
  target with the `observable` flag.
- **Verification**: Verify-kind targets validate work targets. When
  verification fails, a rework edge re-enters the upstream target
  with a diagnosis and retry budget.
- **Phase boundary**: Bullseye uses different prioritisation engines
  at repo and portfolio scopes. Inside a repo (sub-week horizon,
  human as decision-maker), ordering is driven by distance-to-
  observable-checkpoint — `value`/`cost` are not consumed. Across
  repos (weekly-plus horizon, human as bottleneck allocator), WSJF
  with momentum and cross-repo enabler propagation earns its keep.
  See `docs/mcp-triad.md` §9.

## Development

```bash
cargo build          # Build
cargo test           # Run unit + integration tests
cargo clippy         # Lint
cargo fmt --check    # Check formatting
```

See [CLAUDE.md](CLAUDE.md) for architecture details.

## License

Apache 2.0 — see [LICENSE](LICENSE).
