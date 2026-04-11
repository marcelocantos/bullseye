# Bullseye agents guide

Reference for AI agents using Bullseye as an MCP server.

## What Bullseye does

Bullseye manages **targets** — desired project states expressed as
testable properties. It stores targets in
`docs/targets.yaml`, computes which are unblocked (the frontier),
and detects gaps in verification coverage.

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

### bullseye_assert

Upsert a target: create if the ID doesn't exist, patch if it does.
Omit `id` to create a new target with an auto-assigned top-level ID
(`T1`, `T2`, ...). Provide `id` to create at a specific ID (useful
for sub-targets like `T1.2`) or to patch an existing target — the
handler decides create-vs-patch based on whether the ID exists.

On create, `name`, `value`, `cost`, and `acceptance` are required.
On patch, all fields are optional; only the ones provided are changed.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | null | Target ID (omit to auto-assign a new top-level ID) |
| `name` | string | null | Desired state assertion (required on create) |
| `value` | number | null | Fibonacci scale: 1, 2, 3, 5, 8, 13, 20 (required on create) |
| `cost` | number | null | Fibonacci scale: 1, 2, 3, 5, 8, 13, 20 (required on create) |
| `acceptance` | string[] | null | How to verify the target is achieved (required on create) |
| `context` | string | null | Why this target matters |
| `kind` | string | `"work"` on create | `"work"` or `"verify"`; settable only on create |
| `status` | string | `"identified"` on create | `"identified"`, `"converging"`, `"achieved"` |
| `depends_on` | string[] | null | IDs of targets this one depends on (must be achieved first) |
| `blocks` | string[] | null | Sugar: append this target's ID to each listed target's `depends_on` — useful when creating a new prerequisite above existing work |
| `verifies` | string[] | null | For verify targets: IDs of targets this verifies |
| `origin` | string | `"manual"` on create | How the target was created |
| `tags` | string[] | null | Freeform tags |

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

Validate the targets file: ID format, dependency references,
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
overwrite an existing file — use `bullseye_assert` for repos that
already have targets.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Project root directory |
| `project_name` | string | directory name | Project name for sample target context |

### bullseye_import

Import targets from a markdown `docs/targets.md` file into
`docs/targets.yaml`. Parses the markdown format produced by
`render.rs` and other repos' `/cv` skills. Tolerant of minor
formatting variations. Validates the parsed result before writing.

Refuses to overwrite an existing `targets.yaml` unless `force` is set.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Project root directory |
| `path` | string | auto-discover | Explicit path to markdown file |
| `force` | bool | `false` | Overwrite existing `targets.yaml` |

### bullseye_startup_context

Return a concise startup context for the current project: active target count,
frontier targets ready for work, recently achieved targets, and any warnings
(tunnels, validation errors). Designed for agent consumption at session start —
pair with mnemo_recent_activity for full session context.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `recent_days` | number | `14` | Days to look back for recently achieved targets |

### bullseye_portfolio

Discover all repos with targets under a workspace root and return a portfolio
summary: per-repo active/frontier/achieved counts and frontier target names.
Use this for cross-project prioritisation and global convergence assessment.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | `~/work/` | Workspace root to scan |
| `max_depth` | number | `5` | Maximum directory depth to scan |

### bullseye_summary

Return a consolidated status overview in one call: active targets grouped
by parent with rollup counts, frontier (unblocked) targets, blocked
targets with blockers, stale targets, and a top-N WSJF ranking. Replaces
separate calls to `bullseye_list`, `bullseye_frontier`, and
`bullseye_validate` when you want a single snapshot (e.g. at session
start, or inside `/cv`).

The optional `momentum` parameter folds external recency/frequency
signals into the WSJF ranking. When provided, each target's WSJF score
is multiplied by its momentum value before sorting; targets missing from
the list default to 1.0 (no-op). Bullseye never calls other MCP servers,
so the caller (typically `/cv`) is responsible for computing momentum
from e.g. `mnemo_recent_activity` and passing it in — composition
happens at the skill layer, the formula is external, and tuning the
momentum factor doesn't require touching bullseye.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `top_n` | number | `5` | Maximum targets in the WSJF ranking section |
| `momentum` | array | null | Optional per-target multipliers, as a list of `{id, multiplier}` objects (e.g. `[{"id": "T1", "multiplier": 1.6}, {"id": "T3", "multiplier": 0.8}]`). Values > 1.0 boost, < 1.0 suppress, 1.0 is identity. Targets not listed default to 1.0. Duplicate ids use the last multiplier seen. |

When `momentum` is present, the ranking section heading is
`## WSJF ranking, momentum-adjusted (top N)` and each boosted or
suppressed entry is annotated with `(WSJF X × momentum Y, ...)` so the
original score and the applied multiplier are both visible.

A reasonable caller-side formula (from `docs/mcp-triad.md` §2):

```
momentum = 1.0 + 0.3 * log(1 + recent_sessions) * exp(-days_since_last / 7)
```

— or anything else the caller wants. Bullseye just multiplies.

## targets.yaml schema

```yaml
schema_version: 1          # required going forward; legacy files
                           # without it are accepted and stamped on
                           # next save. Bullseye refuses to load a
                           # file whose schema_version is higher than
                           # the binary supports (prompts for upgrade).
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

Targets use `T<N>` (e.g., `T1`, `T2`). IDs are assigned
automatically by `bullseye_assert` when `id` is omitted.
Sub-target IDs (`T1.2`) can be created by passing an explicit
`id` — the assert tool creates-if-missing or patches-if-present.

### Status lifecycle

`identified` → `converging` → `achieved`

Rework resets a verify target to `identified` and its rework
destination to `converging`.

### Edges

Bullseye has a single structural edge type: `depends_on`. Legacy
`gates` edges from older targets files are migrated into `depends_on`
on load (the owning target absorbs its gates as blockers).

- **depends_on**: Hard blocking. Target cannot start until all
  dependencies are achieved. The only structural edge.
- **verifies**: Verify targets validate work targets. Only valid on
  `kind: verify`.
- **rework**: On verify failure, re-enter this target. Must be one of
  the `verifies` targets. Increments `retries` on the destination.

## Typical workflows

### Assess what to work on

```
bullseye_frontier(cwd) → unblocked targets ready for work
```

### Add and track a target

```
bullseye_assert(cwd, name, value, cost, acceptance)
  → creates target with auto-assigned ID
bullseye_assert(cwd, id, status: "converging")
  → mark as in progress (patch by ID)
bullseye_retire(cwd, id, actual_cost)
  → mark as achieved
```

### Add a new prerequisite above existing work

```
bullseye_assert(cwd, name, value, cost, acceptance, blocks: [T5, T7])
  → creates a new target and injects it into T5 and T7's depends_on,
    so both become blocked on the new prerequisite in one call
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

### Session context

```
bullseye_startup_context(cwd) → project context at session start
bullseye_portfolio()          → cross-repo portfolio summary
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
- `bullseye_list` — browse all targets (active, achieved, or all).

Before starting work, call `bullseye_frontier` to see what's
available.

### Managing targets

- `bullseye_assert` — upsert a target. Omit `id` to create a new
  target with an auto-assigned ID (provide `name`, `value`, `cost`,
  `acceptance`). Provide `id` to create at a specific ID (sub-targets
  like `T1.2`) or to patch an existing target (only the provided
  fields change). Supports `depends_on` and `blocks` (sugar for
  injecting this target into other targets' `depends_on`).
- `bullseye_retire` — mark a target as achieved.

When you discover something that should be tracked — a bug, a quality
gap, a missing capability — add it as a target with `bullseye_assert`
rather than leaving a bare TODO.

When you complete work that achieves a target, call `bullseye_retire`
to mark it done.

All tools accept a `cwd` parameter — pass the project root directory.
````
