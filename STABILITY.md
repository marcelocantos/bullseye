# Stability

## Stability commitment

Version 1.0 will represent a backwards-compatibility contract. After
1.0, breaking changes to the MCP tool interface, targets.yaml schema,
or CLI flags will require a new product (not a major version bump).
The pre-1.0 period exists to get these right.

## Interaction surface catalogue

Snapshot as of v0.6.0.

### MCP tools

| Tool | Status | Notes |
|------|--------|-------|
| `bullseye_list(cwd, filter)` | Needs review | Filter values (active/achieved/all) are settled |
| `bullseye_get(cwd, id)` | Stable | |
| `bullseye_add(cwd, name, value, cost, acceptance, ...)` | Needs review | Optional fields may expand |
| `bullseye_update(cwd, id, ...)` | Needs review | Field set may expand |
| `bullseye_retire(cwd, id, actual_cost)` | Stable | |
| `bullseye_frontier(cwd)` | Stable | |
| `bullseye_rework(cwd, id, diagnosis)` | Stable | |
| `bullseye_tunnels(cwd, max_depth)` | Stable | |
| `bullseye_validate(cwd)` | Stable | Validation rules will grow but existing ones won't change |
| `bullseye_graph(cwd)` | Stable | |
| `bullseye_render(cwd)` | Stable | |
| `bullseye_import(cwd, path, force)` | Stable | Markdown-to-YAML migration |
| `bullseye_init(cwd, project_name)` | Stable | Refuses to overwrite existing file |

Planned additions (not yet implemented):
- `bullseye_verify` — execute acceptance checks via sawmill
- `bullseye_portfolio` — cross-repo ranking

### targets.yaml schema

| Field | Status | Notes |
|-------|--------|-------|
| `targets` (map) | Stable | |
| `last_evaluated` | Stable | |
| `name` | Stable | |
| `kind` (work/verify) | Stable | |
| `status` (identified/converging/achieved) | Stable | |
| `value`, `cost` | Stable | Fibonacci scale |
| `actual_cost` | Stable | |
| `acceptance` | Stable | |
| `context` | Stable | |
| `gates` (target, criticality) | Stable | |
| `depends_on` | Stable | |
| `verifies` | Stable | |
| `rework` | Stable | |
| `retry_budget`, `retries` | Stable | |
| `tags` | Stable | |
| `origin` | Stable | |
| `discovered`, `achieved` | Stable | |

Planned additions:
- `checks` — executable acceptance criteria (sawmill integration)
- `cross_depends`, `cross_enables` — cross-repo edges

### CLI flags

| Flag | Status | Notes |
|------|--------|-------|
| `--version` | Stable | |
| `--help` | Stable | |
| `--help-agent` | Stable | |
| (no args) | Stable | Starts MCP stdio server |

### Output formats

| Format | Status | Notes |
|--------|--------|-------|
| targets.md rendering | Needs review | Section structure and field display may evolve |
| Mermaid graph output | Needs review | Node/edge styling may change |
| Tool response text format | Fluid | Not yet formalised; consumers should parse loosely |

## Gaps and prerequisites for 1.0

- **Tool response format**: Responses are unstructured text. Consider
  returning structured JSON alongside text for programmatic consumers.
- ~~**Error types**: All errors use `CallToolError::unknown_tool`.~~ Fixed in v0.2.0 — now uses `from_message`.
- **`bullseye_verify`**: Core planned tool not yet implemented. Should
  be present before 1.0.
- **Test coverage for CLI flags**: No tests for --version/--help/--help-agent.

## Out of scope for 1.0

- Portfolio view (`bullseye_portfolio`) — cross-repo features need
  real-world validation before stabilising.
- Protocol app sync — depends on external infrastructure (sqlpipe,
  pigeon).
- MCP resource support — waiting on protocol maturity.
