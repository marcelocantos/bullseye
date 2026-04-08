# Bullseye agents guide

Reference for AI agents using Bullseye as an MCP server.

## What Bullseye does

Bullseye manages **targets** — desired project states expressed as
testable properties. It stores targets in
`docs/targets.yaml`, ranks them by WSJF (value/cost), computes which
are unblocked, and detects gaps in verification coverage.

Every tool accepts a `cwd` parameter. The server walks up from `cwd`
to find the nearest `docs/targets.yaml` or `targets.yaml`.

## Tools

### bullseye_list

List targets with optional filtering.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `filter` | string | `"active"` | `"active"`, `"achieved"`, or `"all"` |

### bullseye_get

Get a single target by ID.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | required | Target ID (e.g., `"T1"`, `"T1.2"`) |

### bullseye_add

Add a new target. The server assigns the next available ID.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `name` | string | required | Desired state assertion |
| `value` | number | required | Fibonacci scale: 1, 2, 3, 5, 8, 13, 20 |
| `cost` | number | required | Fibonacci scale: 1, 2, 3, 5, 8, 13, 20 |
| `acceptance` | string[] | required | How to verify the target is achieved |
| `context` | string | `""` | Why this target matters |
| `parent` | string | null | Parent target ID for sub-targets |
| `kind` | string | `"work"` | `"work"` or `"verify"` |
| `verifies` | string[] | `[]` | For verify targets: IDs of targets this verifies |
| `origin` | string | `"manual"` | How the target was created |
| `tags` | string[] | `[]` | Freeform tags |

### bullseye_update

Update fields on an existing target. Only provided fields are changed.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | required | Target ID |
| `status` | string | null | `"identified"`, `"converging"`, `"achieved"` |
| `value` | number | null | New value score |
| `cost` | number | null | New cost estimate |
| `name` | string | null | New assertion |
| `acceptance` | string[] | null | Replace acceptance criteria |
| `context` | string | null | Replace context |
| `tags` | string[] | null | Replace tags |

### bullseye_retire

Mark a target achieved.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | required | Target ID |
| `actual_cost` | number | null | Actual cost for calibration |

### bullseye_frontier

Compute unblocked leaf targets ready for work. Validates the target
graph first and returns errors if invalid.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |

### bullseye_rank

WSJF ranking of active targets, split into unblocked and blocked.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |

### bullseye_rework

Trigger rework from a failed verification. Resets the rework
destination to converging, increments its retry count, and resets the
verify target to identified.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | required | The verify target that failed |
| `diagnosis` | string | `""` | What went wrong (appended to rework target context) |

### bullseye_tunnels

Detect work targets that have no verification checkpoint within N
hops.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `max_depth` | number | `2` | Maximum hops before flagging |

### bullseye_validate

Validate the targets file: ID format, parent/dependency references,
cycle detection, verify/rework constraints.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |

### bullseye_graph

Generate a Mermaid dependency graph of active targets.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |

### bullseye_render

Re-render `docs/targets.md` from `docs/targets.yaml`. Mutation tools
(add, update, retire, rework) auto-render, so this is only needed for
manual re-rendering.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |

### bullseye_init

Create a starter `docs/targets.yaml` with a sample target. Refuses to
overwrite an existing file — use `bullseye_add` for repos that already
have targets.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Project root directory |
| `project_name` | string | directory name | Project name for sample target context |

## targets.yaml schema

```yaml
last_evaluated: <git-sha>  # optional, last /cv evaluation point

targets:
  T1:
    name: "All tests pass on CI"          # desired state assertion (required)
    kind: work                            # work (default) or verify
    status: converging                    # identified, converging, achieved
    value: 8                              # Fibonacci: 1, 2, 3, 5, 8, 13, 20
    cost: 3                               # Fibonacci: 1, 2, 3, 5, 8, 13, 20
    actual_cost: 5                        # recorded on retirement (optional)
    acceptance:                           # how to verify achievement (required)
      - CI green on all platforms
      - No test skips without documented reason
    context: "Cross-platform CI is a project goal."  # optional
    parent: T0                            # parent target ID (optional)
    gates:                                # gating relationships (optional)
      - target: T2
        criticality: 0.8                  # fraction of gated value (default 1.0)
    depends_on: [T3]                      # hard blockers (optional)
    verifies: [T4, T5]                    # verify targets only (optional)
    rework: T4                            # re-entry on verify failure (optional)
    retry_budget: 3                       # max rework cycles (optional)
    retries: 0                            # current retry count (auto-managed)
    tags: [ci, infrastructure]            # freeform (optional)
    origin: manual                        # how created (default "manual")
    discovered: 2026-03-01                # date discovered (required)
    achieved: 2026-03-15                  # date achieved (optional)
```

### Target IDs

Top-level targets use `T<N>` (e.g., `T1`, `T2`). Sub-targets append
a dot-separated suffix: `T1.1`, `T1.2`. IDs are assigned
automatically by `bullseye_add`.

### Status lifecycle

`identified` → `converging` → `achieved`

Rework resets a verify target to `identified` and its rework
destination to `converging`.

### Edges

- **parent/child**: Decomposition. Parents are excluded from the
  frontier while they have active children.
- **depends_on**: Hard blocking. Target cannot start until all
  dependencies are achieved.
- **gates**: Soft blocking with criticality weight. A gate at 0.8
  means 80% of the gated target's value depends on this gate.
- **verifies**: Verify targets validate work targets. Only valid on
  `kind: verify`.
- **rework**: On verify failure, re-enter this target. Must be one of
  the `verifies` targets. Increments `retries` on the destination.

## Typical workflows

### Assess what to work on

```
bullseye_frontier(cwd) → unblocked targets ready for work
bullseye_rank(cwd)     → full priority ordering with blocking info
```

### Add and track a target

```
bullseye_add(cwd, name, value, cost, acceptance)
  → creates target, returns assigned ID
bullseye_update(cwd, id, status: "converging")
  → mark as in progress
bullseye_retire(cwd, id, actual_cost)
  → mark as achieved
```

### Verify and rework

```
bullseye_frontier(cwd)
  → includes verify targets when their verifies are achieved
bullseye_rework(cwd, id, diagnosis)
  → failed verification triggers rework cycle
```

### Health checks

```
bullseye_validate(cwd)  → schema conformance
bullseye_tunnels(cwd)   → unverified work chains
bullseye_graph(cwd)     → visual dependency map
```

## Agent integration

Add the following snippet to your project's `CLAUDE.md` (or
equivalent agent instructions file) to enable target-driven workflow
management via Bullseye. The only prerequisite is having the Bullseye
MCP server registered (see [README.md](../README.md#mcp-client-configuration)).

````markdown
## Target management

This project uses [Bullseye](https://github.com/marcelocantos/bullseye)
for target management. Targets are desired project states expressed as
testable properties, stored in `docs/targets.yaml`.

### Getting started

If this project doesn't have `docs/targets.yaml` yet, call
`bullseye_init` to create one with a starter template.

### Assessing work

- `bullseye_frontier` — unblocked targets ready for work right now.
- `bullseye_rank` — full WSJF priority ordering with blocking info.
- `bullseye_list` — browse all targets (active, achieved, or all).

Before starting work, call `bullseye_frontier` to see what's
available. Use `bullseye_rank` when you need the full priority picture.

### Managing targets

- `bullseye_add` — create a new target (provide name, value, cost,
  and acceptance criteria; ID is auto-assigned).
- `bullseye_update` — change status, value, cost, or other fields on
  an existing target.
- `bullseye_retire` — mark a target as achieved.

When you discover something that should be tracked — a bug, a quality
gap, a missing capability — add it as a target with `bullseye_add`
rather than leaving a bare TODO.

When you complete work that achieves a target, call `bullseye_retire`
to mark it done.

All tools accept a `cwd` parameter — pass the project root directory.
````
