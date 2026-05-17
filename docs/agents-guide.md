# Bullseye agents guide

Reference for AI agents using Bullseye as an MCP server.

## What Bullseye does

Bullseye manages **targets** — desired project states expressed as
testable properties. It stores targets in `bullseye.yaml`, computes
which are unblocked (the frontier), and helps agents fan out across
parallel work.

Every target-operating tool accepts a `cwd` parameter. Where the
targets file is discovered depends on the configured storage mode.

## Storage locations and first-use flow

Each repo chooses **once** where its `bullseye.yaml` lives:

- `in_repo` — inside the project. Discovery walks up from `cwd`.
- `external` — in a shadow tree under `~/.local/share/bullseye/`
  mirroring the absolute `cwd`, so
  `/Users/alice/work/acme/api` maps to
  `~/.local/share/bullseye/Users/alice/work/acme/api/bullseye.yaml`.
  Path-driven — no git-remote or layout assumptions.

After `bullseye_init`, every other tool calls
`discover_anywhere(cwd)` which checks both locations and returns the
first match. If both exist (edge case), **in-repo wins**.

There is no machine-wide config file. The location is encoded by
where `bullseye.yaml` lives on disk.

### First-use flow for agents

If the repo has no `bullseye.yaml` in either location, every
target-operating tool returns an error ending with this prompt —
pass it to the user verbatim:

> **Create bullseye.yaml for this repo where?**
> - **in_repo** — commit `bullseye.yaml` into the repo (you own it, team uses bullseye).
> - **external** — shadow tree under `~/.local/share/bullseye/` (read-only repo, or personal use of bullseye).
>
> Call `bullseye_init` with `location: in_repo` or `location: external`.

After the user answers, call `bullseye_init` with the chosen `location`.
Subsequent tool calls proceed normally. Do **not** retry the original
tool until init succeeds.

The same prompt is returned when `bullseye_init` itself is called
without a valid `location` — so the agent can route straight to `init`
without first triggering an error elsewhere.

## Tools

### bullseye_list

List targets with optional filtering.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `filter` | string | `"active"` | `"active"`, `"achieved"`, `"set_aside"`, or `"all"` |

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

On create, `name` and `acceptance` are required. `value` and `cost`
default to `0` (the "not set at repo scope" sentinel) when omitted —
they're portfolio-scope inputs only, never consumed by repo-level
ordering, so leaving them unset is appropriate when you're working
inside a single repo. Provide them when adding a target intended to
participate in cross-repo WSJF ranking. On patch, all fields are
optional; only the ones provided are changed.

**Achieved targets are immutable.** `bullseye_put` rejects content
edits (name/acceptance/context/value/cost/tags/depends_on) on a
target whose current status is `achieved`. Achieved targets are
historical artifacts. To modify one, re-open it first by patching
`status: identified` — either in a prior call, or atomically in the
same call alongside the content edits (the reopen applies first,
then the content lands on the now-identified target). Status-only
transitions on achieved targets remain allowed.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | null | Target ID (omit to auto-assign a new top-level ID) |
| `name` | string | null | Desired state assertion (required on create) |
| `value` | number | `0` on create | Fibonacci scale: 1, 2, 3, 5, 8, 13, 20. **Portfolio-scope input only** — not consumed by repo-level ordering, so optional at repo scope. `0` means "not set". |
| `cost` | number | `0` on create | Fibonacci scale: 1, 2, 3, 5, 8, 13, 20. **Portfolio-scope input only** — not consumed by repo-level ordering, so optional at repo scope. `0` means "not set". |
| `acceptance` | string[] | null | How to verify the target is achieved (required on create). This is the verification contract — whether the pass signal comes from CI, a human review, a smoke test, or a design walkthrough is described here in free text. |
| `context` | string | null | Why this target matters |
| `status` | string | `"identified"` on create | `"identified"`, `"converging"`, `"achieved"`. The `set_aside` value is **not** settable here — call `bullseye_set_aside(id, reason)` instead so the rationale is always recorded. |
| `depends_on` | string[] | null | IDs of targets this one depends on (must be achieved first) |
| `blocks` | string[] | null | Sugar: append this target's ID to each listed target's `depends_on` — useful when creating a new prerequisite above existing work. Refuses to inject into achieved targets (same rule as content patches). |
| `origin` | string | `"manual"` on create | How the target was created |
| `tags` | string[] | null | Freeform tags |

### bullseye_retire

Mark a target achieved.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | required | Target ID |
| `actual_cost` | number | null | Actual cost for calibration |

### bullseye_set_aside

Set a target aside with a documented rationale (🎯T18). Use this
when the target was **not** delivered — parked indefinitely,
deferred to a later milestone, or actively rejected as won't-fix —
but should no longer surface on the frontier. Distinct from
`bullseye_retire`, which is achievement-only.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | required | Target ID to set aside |
| `reason` | string | required | Rationale for the disposition. Free text — carries the parked / deferred / wont_fix nuance. Must be non-empty after trimming. Examples: `"deferred to v2.0 — UX needs more thought"`, `"won't fix — superseded by 🎯T57's redesign"`, `"parked pending design discussion in 🎯T42"`. |

**Behaviour:**
- Status flips to `set_aside`; `set_aside_reason` is recorded on the target.
- Set-aside targets unblock their dependents the same way achieved targets do (terminal for graph traversal).
- They render in a separate `## Set aside` group in `bullseye_summary`, distinct from achievements.
- Refuses already-achieved targets (would falsify the achievement record).
- Idempotent on already-set-aside targets — original reason wins, no error.
- Rejects empty / whitespace-only reasons — the rationale is the load-bearing artefact.

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

### bullseye_revert

Move a target from achieved back to converging — use this when a
regression or new information shows that the achievement was premature.
Clears the achieved date and appends `Reverted YYYY-MM-DD: <reason>`
to the target's context. Achievement-only: to resume a set-aside
target use `bullseye_put` with `status: identified`.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |
| `id` | string | required | The achieved target to revert |
| `reason` | string | required | Why the achievement is being reversed |

### bullseye_frontier

Compute unblocked leaf targets ready for work. Validates the target
graph first and returns errors if invalid.

The frontier is the *parallelisable set* — agents are expected to fan
out across it rather than pick a single item. Frontier targets are
ordered by **descending unblocking fanout** (count of active targets
that depend on this one), tiebroken by ascending ID. Per-target
`value`/`cost` and `momentum` are **not consumed** — those are
portfolio-scope inputs.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |

### bullseye_validate

Validate the targets file: ID format, dependency references, and
cycle detection.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |

### bullseye_graph

Generate a Mermaid dependency graph of active targets.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Working directory |

### bullseye_init

Create a starter `bullseye.yaml` with a sample target. **Location is
required** — `"in_repo"` (committed into the repo) or `"external"`
(shadow tree under `~/.local/share/bullseye/`). See
[Storage locations and first-use flow](#storage-locations-and-first-use-flow).
Refuses to overwrite an existing file at either location.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Project root directory |
| `location` | string | required | `"in_repo"` or `"external"` — ask the user on first use; it's per-repo, not machine-wide |
| `project_name` | string | directory name | Project name for sample target context |

### bullseye_import

Import targets from a markdown file into `bullseye.yaml`. Validates the
parsed result before writing. Refuses to overwrite an existing
`bullseye.yaml` in either location unless `force: true`.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cwd` | string | required | Project root directory |
| `path` | string | required | Explicit path to the markdown source |
| `location` | string | required | `"in_repo"` or `"external"` (same semantics as `bullseye_init`) |
| `force` | bool | `false` | Overwrite existing `bullseye.yaml` |

### bullseye_startup_context

Return a concise startup context for the current project: active target count,
frontier targets ready for work, recently achieved targets, and any validation
warnings. Designed for agent consumption at session start — pair with
mnemo_recent_activity for full session context.

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
ordered by descending unblocking fanout, blocked targets with
blockers, and stale targets with inconsistent graph state. Replaces
separate calls to `bullseye_list`, `bullseye_frontier`, and
`bullseye_validate` when you want a single snapshot.

Each frontier entry is annotated with `fanout=M` showing the sort
key, so the ordering is always visible and debuggable.

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

**Parallelised variant** (optional): projects with *heterogeneous*
toolchains can restructure the hook so independent checks run
concurrently. The idea is to decompose `bullseye` into per-tool
sub-rules and let make schedule them across cores:

```make
NPROC := $(shell getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
MAKEFLAGS += -j$(NPROC)

.PHONY: bullseye check-fmt check-clippy check-tests check-clean

bullseye: check-fmt check-clippy check-tests check-clean

check-fmt:
\t@cargo fmt --check >/dev/null && echo "✓ fmt"

check-clippy:
\t@cargo clippy --quiet --all-targets -- -D warnings >/dev/null 2>&1 && echo "✓ clippy"

check-tests:
\t@cargo test --quiet >/dev/null 2>&1 && echo "✓ tests"

check-clean:
\t@test -z "$$(git status --porcelain)" && echo "✓ clean tree" || \\
\t (echo "✗ dirty tree:"; git status --short; exit 1)
```

The `NPROC` fallback chain works on Linux (`nproc`), macOS/BSD
(`sysctl -n hw.ncpu`), and anywhere else with `getconf`; the final
`echo 4` guards against exotic environments where none resolve.
Setting `MAKEFLAGS` inside the Makefile is the convention — callers
should never pass `-j` on the command line, since not every project
is safe to parallelise.

**When this is worth it**: parallelism only meaningfully helps
projects with *mixed toolchains* — e.g. Go + Python + shellcheck +
docs, where each tool uses a different working set and doesn't
contend with the others. Single-ecosystem projects (pure cargo, pure
go test) see little benefit because internal build parallelism
already saturates cores and contenders like `clippy` and `test`
fight over the same `target/` directory, serialising themselves via
filesystem locks. The bullseye repo itself keeps the simple form
above for that reason. Reach for the decomposed layout only when
profiling shows `make bullseye` is genuinely CPU-bound across
disjoint toolchains.

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

## bullseye.yaml schema

```yaml
schema_version: 5          # required going forward; legacy files
                           # without it are accepted and stamped on
                           # next save. Bullseye refuses to load a
                           # file whose schema_version is higher than
                           # the binary supports (prompts for upgrade).
last_evaluated: <git-sha>  # optional, last /cv evaluation point

targets:
  T1:
    name: "All tests pass on CI"          # desired state assertion (required)
    status: converging                    # identified, converging, achieved,
                                          # set_aside (the latter is set via
                                          # bullseye_set_aside, not bullseye_put).
    value: 8                              # Fibonacci (portfolio-scope input only)
    cost: 3                               # Fibonacci (portfolio-scope input only)
    actual_cost: 5                        # recorded on retirement (optional)
    acceptance:                           # verification contract (required):
      - CI green on all platforms         # prose describing what "done" looks like.
      - No test skips without documented reason  # whether pass signal comes from
                                          # CI, human review, smoke test, or design
                                          # walkthrough is described here in free text.
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
    tags: [ci, infrastructure]            # freeform (optional)
    origin: manual                        # how created (default "manual")
    discovered: 2026-03-01                # date discovered (required)
    achieved: 2026-03-15                  # date achieved (optional)
```

### Phase-boundary hypothesis

Bullseye uses different prioritisation engines at repo and portfolio
scopes (`docs/mcp-triad.md` §9):

- **Repo-level** (sub-week horizon, human as decision-maker): ordering
  rewards maximum unblocking fanout (targets that unblock the most
  downstream work move more of the graph per unit effort), tiebroken
  by ascending target ID. Per-target `value`/`cost` and the `momentum`
  input are **not consumed** at this layer. The frontier is the
  parallelisable set — agents are expected to fan out across it.
- **Portfolio-level** (weekly-plus horizon, human as bottleneck
  allocator): WSJF with momentum and cross-repo enabler propagation
  earns its keep. This is where `value`/`cost`/`momentum`/
  `cross_enables` are consumed. 🎯T2.3 is the portfolio engine;
  the schema fields are in place but weighted propagation is still
  pending.

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

To undo an achievement (e.g. a regression shows it was premature),
use `bullseye_revert` — it moves the target back to `converging` and
appends a timestamped revert note to its context.

### Edges

Bullseye has a single structural edge type: `depends_on`. Legacy
`gates` edges from older targets files are migrated into `depends_on`
on load (the owning target absorbs its gates as blockers).

- **depends_on**: Hard blocking. A target cannot be worked until all
  of its dependencies are achieved. This is the only structural edge.

## Typical workflows

### Assess what to work on

```
bullseye_frontier(cwd) → unblocked targets ready for work
```

### Add and track a target

```
bullseye_put(cwd, name, acceptance)
  → creates target with auto-assigned ID; value/cost optional
    at repo scope (set them only for portfolio-scope ranking)
bullseye_put(cwd, id, status: "converging")
  → mark as in progress (patch by ID)
bullseye_retire(cwd, id, actual_cost)
  → mark as achieved
```

### Add a new prerequisite above existing work

```
bullseye_put(cwd, name, acceptance, blocks: [T5, T7])
  → creates a new target and injects it into T5 and T7's depends_on,
    so both become blocked on the new prerequisite in one call
```

### Revert a mistaken retirement

```
bullseye_revert(cwd, id, reason)
  → move an achieved target back to converging, clearing the
    achieved date and appending a timestamped revert note
```

### Health checks

```
bullseye_validate(cwd)  → schema conformance and cycle detection
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
testable properties, stored in `bullseye.yaml`.

### Getting started

If this project doesn't have `bullseye.yaml` yet, call
`bullseye_init` to create one with a starter template.

### Assessing work

- `bullseye_frontier` — unblocked targets ready for work right now.
- `bullseye_list` — browse all targets (active, achieved, set_aside, or all).

Before starting work, call `bullseye_frontier` to see what's
available.

### Managing targets

- `bullseye_put` — upsert a target. Omit `id` to create a new
  target with an auto-assigned ID (provide `name` and `acceptance`;
  `value`/`cost` are optional at repo scope). Provide `id` to create at a specific ID (sub-targets
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
