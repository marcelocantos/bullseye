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

### bullseye_put

Upsert a target: create if the ID doesn't exist, patch if it does.
Omit `id` to create a new target with an auto-assigned top-level ID
(`T1`, `T2`, ...). Provide `id` to create at a specific ID (useful
for sub-targets like `T1.2`) or to patch an existing target — the
handler decides create-vs-patch based on whether the ID exists.

On create, `name`, `value`, `cost`, and `acceptance` are required.
On patch, all fields are optional; only the ones provided are changed.

**Achieved targets are immutable.** As of v0.13.0 `bullseye_put`
rejects content edits (name/acceptance/context/value/cost/tags/
depends_on/verifies/observable) on a target whose current status
is `achieved`. Achieved targets are historical artifacts. To
modify one, re-open it first by patching `status: identified`
— either in a prior call, or atomically in the same call
alongside the content edits (the reopen applies first, then
the content lands on the now-identified target). Status-only
transitions on achieved targets remain allowed, and
`bullseye_retire` is unchanged.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | null | Target ID (omit to auto-assign a new top-level ID) |
| `name` | string | null | Desired state assertion (required on create) |
| `value` | number | null | Fibonacci scale: 1, 2, 3, 5, 8, 13, 20 (required on create). **Portfolio-scope input only** — not consumed by repo-level ordering. |
| `cost` | number | null | Fibonacci scale: 1, 2, 3, 5, 8, 13, 20 (required on create). **Portfolio-scope input only** — not consumed by repo-level ordering. |
| `acceptance` | string[] | null | How to verify the target is achieved (required on create) |
| `context` | string | null | Why this target matters |
| `kind` | string | `"work"` on create | `"work"` or `"verify"`; settable only on create |
| `status` | string | `"identified"` on create | `"identified"`, `"converging"`, `"achieved"` |
| `observable` | bool | `false` on create, unchanged on patch | Mark a work target as producing a human-observable checkpoint. Verify-kind targets are observable by definition; this flag only matters for work-kind targets. Drives repo-level ordering and `bullseye_tunnels` membership. |
| `depends_on` | string[] | null | IDs of targets this one depends on (must be achieved first) |
| `blocks` | string[] | null | Sugar: append this target's ID to each listed target's `depends_on` — useful when creating a new prerequisite above existing work. Refuses to inject into achieved targets (same rule as content patches). |
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

### bullseye_verify

Emit a structured execution plan that maps each of a target's
`checks` entries to a sawmill tool invocation. **Bullseye does
not execute the plan** — the calling agent runs each check
against sawmill and folds the results back into a report. This
preserves the MCP cross-server constraint (servers don't call
each other; the agent composes).

The response is a markdown document with a human-readable plan
plus a JSON block containing the structured plan and a pending
report template. Checks come in three shapes, matching the
`checks` field of the target:

- `convention: <name>` → maps to sawmill `check_conventions`
- `query: {kind, pattern?, exclude_path?, expect}` → maps to sawmill `query`
- `invariant: <name>` → maps to sawmill `check_invariants` (phase 2, requires sawmill T19)

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | required | Target ID whose `checks` field should be planned |

### bullseye_frontier

Compute unblocked leaf targets ready for work. Validates the target
graph first and returns errors if invalid.

As of v0.13.0, the frontier is ordered by the **repo-level signal**:
ascending distance to the nearest observable target, tiebroken by
descending unblocking fanout (count of active targets that depend
on this one), then ascending ID. Per-target `value`/`cost` and
`momentum` are **not consumed** — those are portfolio-scope inputs.
When every frontier target has no observable reachable at all,
`bullseye_convergence`'s next-action emits a `**Blocked**: … reshape`
recommendation instead of auto-selecting, prompting the human to
add an intermediate observable target or promote an existing
downstream work target with `observable: true`.

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

Detect work targets that have no **observable checkpoint** reachable
within N hops along the forward dependency graph. As of v0.13.0 a
target is observable iff `kind: verify` OR the new `observable: true`
flag is set; previous releases defined observability strictly as
verification reachability. Legacy targets files carry no observable
work targets until the human opts in, so on a freshly-upgraded repo
most work targets will be flagged — the reshape signal is the point.

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
overwrite an existing file — use `bullseye_put` for repos that
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

Return a consolidated status overview in one call: active targets
grouped by parent with rollup counts, frontier (unblocked) targets
ordered by the repo-level signal (distance-to-observable then
unblocking fanout), blocked targets with blockers, and stale targets
with inconsistent graph state. Replaces separate calls to
`bullseye_list`, `bullseye_frontier`, and `bullseye_validate` when you
want a single snapshot.

Each frontier entry is annotated with `dist=N, fanout=M` showing the
two sort keys, so the ordering is always visible and debuggable.

The optional `momentum` parameter is **retained for wire
compatibility but not consumed at repo scope** as of v0.13.0.
Repo-level ordering no longer uses `value`/`cost`/`momentum` — those
are portfolio-scope inputs. A future `bullseye_portfolio` WSJF
ranking (🎯T2.3) will consume momentum at the portfolio layer.
Callers that still pass `momentum` at repo scope get no error, just
a no-op.

`frontier_details: true` expands each frontier entry with its full
acceptance criteria, context, and edges — useful when you would
otherwise round-trip `bullseye_get` on every frontier target.
`bullseye_convergence` uses this internally.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `momentum` | array | null | Optional per-target multipliers (wire-compat only at repo scope; see above). List of `{id, multiplier}` objects. |
| `frontier_details` | bool | `false` | Expand each frontier entry with full acceptance, context, and edges. |

### bullseye_convergence

Answer "what's the next most-valuable thing to work on?" in a single
tool call. Consolidates the old `/cv` worker's many round-trips
(standing-invariants check, unreleased-fix scan, target summary,
per-target details, frontier ranking, recommendation) into one.

The response has the shape:

```
# Convergence
File: …
Total: N target(s) — X active, Y achieved

## Invariants
<stdout of `make bullseye` or `mk bullseye`, verbatim>
Status: ✓ all green / ✗ failed (exit N) / ⚠ unknown (hook not configured) / skipped

## Unreleased fixes
<commits since the last tag whose subjects contain fix markers, or "(none)">

## Active targets by group
## Frontier (unblocked, ready for work)
  <each frontier target with full acceptance, context, tags, edges inline>
## Blocked targets
## Stale targets

## Next action
**Execute now**: Work on 🎯T… <name>         ← or …
**Execute now**: Run `/release` to ship N unreleased fix(es)  ← or …
**Blocked**: invariants failing (exit N). See above.
```

The `## Next action` line is deterministic: bullseye picks the
recommendation based on invariants state, unreleased-fix count, and
the focus-ordered frontier. The calling skill (`/cv`) relays the
output verbatim and executes the instruction if it starts with
`**Execute now**`; anything else (`**Blocked**`, `**Parallel**`) is
presented to the user for decision.

**Standing-invariants hook**: bullseye_convergence requires a
`bullseye` rule in the project's `Makefile` or `mkfile`. The rule
runs whatever checks the project considers "green" — tests, lints,
clean tree, anything. Exit code 0 = all green, non-zero = at least
one violation. Stdout is relayed verbatim into the Invariants section.
Example for a Rust project:

```make
bullseye:
\t@cargo fmt --check >/dev/null && echo "✓ fmt"
\t@cargo clippy --quiet --all-targets -- -D warnings >/dev/null 2>&1 && echo "✓ clippy"
\t@cargo test --quiet >/dev/null 2>&1 && echo "✓ tests"
\t@test -z "$(git status --porcelain)" && echo "✓ clean tree" || \\
\t (echo "✗ dirty tree:"; git status --short; exit 1)
```

If the hook is missing, convergence still runs to completion — the
Invariants section carries a setup warning with an example rule, the
frontier recommendation still fires, and the Next action ends with a
"standing invariants are unknown" warning so the agent proceeds with
appropriate caution.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `momentum` | array | null | Same shape as `bullseye_summary` — optional per-target multipliers. |
| `skip_invariants` | bool | `false` | Skip the `make bullseye` / `mk bullseye` invocation. Lightweight scan mode that omits the hook but still runs the summary and frontier recommendation. |

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
    value: 8                              # Fibonacci (portfolio-scope input only)
    cost: 3                               # Fibonacci (portfolio-scope input only)
    actual_cost: 5                        # recorded on retirement (optional)
    observable: true                      # v0.13.0: mark work target as producing
                                          # a human-visible checkpoint (optional,
                                          # default false, omitted when false)
    acceptance:                           # how to verify achievement (required)
      - CI green on all platforms
      - No test skips without documented reason
    checks:                               # v0.13.0: executable checks (optional)
      - convention: no-skipped-tests      # sawmill convention name
      - query:                            # sawmill query tool invocation
          kind: attribute
          pattern: "#\\[ignore\\]"
          exclude_path: tests/fixtures/
          expect: 0
      - invariant: ci-green               # sawmill invariant name (phase 2)
    context: "Cross-platform CI is a project goal."  # optional
    depends_on: [T3]                      # hard blockers (optional)
    cross_depends:                        # v0.13.0: advisory cross-repo deps
      - repo: marcelocantos/sawmill       # (optional, doesn't block frontier)
        target: T19
        note: "Needs structural invariants"
    cross_enables:                        # v0.13.0: advisory cross-repo enablers
      - repo: marcelocantos/mnemo         # (optional, feeds portfolio ranking)
        capability: "Session startup context"
    verifies: [T4, T5]                    # verify targets only (optional)
    rework: T4                            # re-entry on verify failure (optional)
    retry_budget: 3                       # max rework cycles (optional)
    retries: 0                            # current retry count (auto-managed)
    tags: [ci, infrastructure]            # freeform (optional)
    origin: manual                        # how created (default "manual")
    discovered: 2026-03-01                # date discovered (required)
    achieved: 2026-03-15                  # date achieved (optional)
```

### Phase-boundary hypothesis (v0.13.0)

Bullseye uses different prioritisation engines at repo and portfolio
scopes (`docs/mcp-triad.md` §9):

- **Repo-level** (sub-week horizon, human as decision-maker): ordering
  rewards shortest path to the next **observable checkpoint**,
  tiebroken by unblocking fanout. Per-target `value`/`cost` and the
  `momentum` input are **not consumed** at this layer. The point is
  to drive work toward the next human decision point as quickly as
  possible, and to flag opaque tunnels where the graph has no
  checkpoint to head toward.
- **Portfolio-level** (weekly-plus horizon, human as bottleneck
  allocator): WSJF with momentum and cross-repo enabler propagation
  earns its keep. This is where `value`/`cost`/`momentum`/
  `cross_enables` are consumed. 🎯T2.3 is the portfolio engine;
  v0.13.0 has the schema fields in place but weighted propagation
  is still pending.

The phase boundary means per-target `value` and `cost` should be
thought of as portfolio-scope inputs — they don't drive repo-level
ordering even though they continue to round-trip cleanly through
the schema.

### Achieved targets are immutable

As of v0.13.0, `bullseye_put` refuses content patches on achieved
targets. Re-open with `status: identified` first (either in a
prior call or atomically in the same call alongside content edits).
`bullseye_retire` is unchanged. The `blocks: [T]` sugar into an
achieved target is also rejected (it would mutate T's `depends_on`,
which is a content edit in disguise). See 🎯T8.

### Target IDs

Targets use `T<N>` (e.g., `T1`, `T2`). IDs are assigned
automatically by `bullseye_put` when `id` is omitted.
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
bullseye_put(cwd, name, value, cost, acceptance)
  → creates target with auto-assigned ID
bullseye_put(cwd, id, status: "converging")
  → mark as in progress (patch by ID)
bullseye_retire(cwd, id, actual_cost)
  → mark as achieved
```

### Add a new prerequisite above existing work

```
bullseye_put(cwd, name, value, cost, acceptance, blocks: [T5, T7])
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
bullseye_tunnels(cwd)   → work chains with no observable checkpoint
bullseye_graph(cwd)     → visual dependency map
```

### Execute acceptance checks

```
bullseye_verify(cwd, id) → structured plan mapping the target's
                           `checks` entries to sawmill tool calls.
                           The agent runs the plan against sawmill
                           and folds results into a pass/fail report.
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

- `bullseye_put` — upsert a target. Omit `id` to create a new
  target with an auto-assigned ID (provide `name`, `value`, `cost`,
  `acceptance`). Provide `id` to create at a specific ID (sub-targets
  like `T1.2`) or to patch an existing target (only the provided
  fields change). Supports `depends_on` and `blocks` (sugar for
  injecting this target into other targets' `depends_on`).
- `bullseye_retire` — mark a target as achieved.

When you discover something that should be tracked — a bug, a quality
gap, a missing capability — add it as a target with `bullseye_put`
rather than leaving a bare TODO.

When you complete work that achieves a target, call `bullseye_retire`
to mark it done.

All tools accept a `cwd` parameter — pass the project root directory.
````
