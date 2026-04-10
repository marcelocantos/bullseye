# Stability

## Stability commitment

Version 1.0 will represent a backwards-compatibility contract. After
1.0, breaking changes to the MCP tool interface, targets.yaml schema,
or CLI flags will require a new product (not a major version bump).
The pre-1.0 period exists to get these right.

## Interaction surface catalogue

Snapshot as of v0.7.0.

### MCP tools

| Tool | Status | Notes |
|------|--------|-------|
| `bullseye_list(cwd, filter)` | Stable | Filter values (active/achieved/all) are settled |
| `bullseye_get(cwd, id)` | Stable | |
| `bullseye_add(cwd, name, value, cost, acceptance, ...)` | Needs review | Optional fields may expand; missing `depends_on` param |
| `bullseye_update(cwd, id, ...)` | Needs review | Cannot update `depends_on`, `gates`, `verifies`, `rework`, `retry_budget` |
| `bullseye_retire(cwd, id, actual_cost)` | Stable | |
| `bullseye_frontier(cwd)` | Stable | |
| `bullseye_rework(cwd, id, diagnosis)` | Stable | |
| `bullseye_tunnels(cwd, max_depth)` | Stable | |
| `bullseye_validate(cwd)` | Stable | Validation rules will grow but existing ones won't change |
| `bullseye_graph(cwd)` | Stable | |
| `bullseye_render(cwd)` | Stable | |
| `bullseye_import(cwd, path, force)` | Stable | Markdown-to-YAML migration |
| `bullseye_init(cwd, project_name)` | Stable | Refuses to overwrite existing file |
| `bullseye_startup_context(cwd, recent_days)` | Needs review | New in v0.7.0; output format may evolve |
| `bullseye_portfolio(root, max_depth)` | Needs review | New in v0.7.0; output format may evolve |

Planned additions (not yet implemented):
- `bullseye_verify` — execute acceptance checks via sawmill

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
- **`bullseye_add` missing `depends_on`**: Cannot set dependencies at
  creation time — requires a separate manual edit of `targets.yaml`.
- **`bullseye_update` missing edge parameters**: Cannot update
  `depends_on`, `gates`, `verifies`, `rework`, or `retry_budget`.
- **`bullseye_verify`**: Core planned tool not yet implemented. Should
  be present before 1.0.
- **`bullseye_startup_context` and `bullseye_portfolio` stabilisation**:
  Both new in v0.7.0; need real-world usage before locking in.
- **Test coverage for CLI flags**: No tests for --version/--help/--help-agent.

## Out of scope for 1.0

- Protocol app sync — depends on external infrastructure (sqlpipe,
  pigeon).
- MCP resource support — waiting on protocol maturity.
