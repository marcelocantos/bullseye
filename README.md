# Bullseye

An MCP server for managing **targets** — desired project states
expressed as testable properties, with dependency tracking and
frontier computation.

Targets live in `docs/targets.yaml` (source of truth) with an
auto-rendered `docs/targets.md` markdown view. The server discovers
the targets file by walking up from the caller's working directory.

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

## Tools

Bullseye exposes 15 MCP tools. All accept a `cwd` parameter to locate
the nearest `targets.yaml`.

| Tool | Description |
|------|-------------|
| `bullseye_list` | List targets (active/achieved/all) |
| `bullseye_get` | Get a single target by ID with full detail |
| `bullseye_add` | Add a new target (auto-assigns ID) |
| `bullseye_update` | Update fields on an existing target |
| `bullseye_retire` | Mark a target achieved |
| `bullseye_frontier` | Unblocked leaf targets ready for work |
| `bullseye_rework` | Trigger rework from a failed verification |
| `bullseye_tunnels` | Detect work targets far from verification |
| `bullseye_validate` | Check schema conformance |
| `bullseye_graph` | Generate Mermaid dependency graph |
| `bullseye_import` | Import targets from markdown into YAML |
| `bullseye_render` | Re-render docs/targets.md from YAML |
| `bullseye_init` | Create starter targets.yaml with sample target |
| `bullseye_startup_context` | Session startup context (frontier, recent achievements, warnings) |
| `bullseye_portfolio` | Cross-repo portfolio summary with frontier targets |

See [agents-guide.md](docs/agents-guide.md) for detailed tool
parameters, the targets.yaml schema, usage workflows, and a
[copy-pasteable CLAUDE.md snippet](docs/agents-guide.md#agent-integration)
for wiring Bullseye into your project's agent instructions.

## Key concepts

- **Frontier**: The set of unblocked leaf targets that can be worked
  on right now, in parallel.
- **Verification**: Verify-kind targets validate work targets. When
  verification fails, a rework edge re-enters the upstream target
  with a diagnosis and retry budget.
- **Tunnels**: Work targets more than N hops from a verification
  checkpoint — a signal to insert verification earlier.

## Development

```bash
cargo build          # Build
cargo test           # Run all 59 tests
cargo clippy         # Lint
cargo fmt --check    # Check formatting
```

See [CLAUDE.md](CLAUDE.md) for architecture details.

## License

Apache 2.0 — see [LICENSE](LICENSE).
