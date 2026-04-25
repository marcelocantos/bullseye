# Stability

## Stability commitment

Version 1.0 will represent a backwards-compatibility contract. After
1.0, breaking changes to the MCP tool interface, bullseye.yaml schema,
or CLI flags will require a new product (not a major version bump).
The pre-1.0 period exists to get these right.

## Interaction surface catalogue

Snapshot as of v0.19.0. Changes since v0.18.0 affecting the
interaction surface:

- **`observable` renamed to `showcase`; retirement of a showcase
  target requires a recorded demonstration** (🎯T14). The work-target
  flag that promotes an opaque target to a checkpoint is now spelled
  `showcase: true` in `bullseye.yaml` and on `bullseye_put`'s
  parameter; the schema bumps to `schema_version: 2`. The legacy
  field name still deserialises via `#[serde(alias = "observable")]`
  on the new field, so older files load cleanly and are rewritten
  under the new name on next save (one-shot migration). On the
  retirement path, `bullseye_retire` now refuses to retire any
  target that carries `showcase: true` unless the caller passes a
  non-empty `demonstration` string describing what was actually
  shown to the user; the string is stored on the retired target as
  permanent evidence. Internal predicate / helper renames (`is_observable`
  → `is_checkpoint`, `observable_distance` → `checkpoint_distance`,
  `OBSERVABLE_REACH_LIMIT` → `CHECKPOINT_REACH_LIMIT`) and
  output-string renames (`[observable]` → `[showcase]`,
  `no observable reachable` → `no checkpoint reachable`,
  `min distance-to-observable` → `min distance-to-checkpoint`) follow
  the same theme — the field is the obligation half (`showcase`),
  the predicate is the ranking concept (`checkpoint`).

Earlier changes still in effect from v0.18.0:
- **Concurrency protocol for `bullseye.yaml`** (🎯T17). Bullseye now
  tolerates concurrent writers. Mutating tools (`bullseye_put`,
  `bullseye_retire`, `bullseye_rework`, `bullseye_import`) acquire an
  exclusive advisory flock on a sibling `<dir>/bullseye.yaml.lock`
  sentinel before reading, re-read the yaml fresh from disk (bypassing
  the parse cache), CAS-check `(mtime, len)` across the
  read-modify-write window to catch non-flock-honouring writers, and
  write back atomically via tempfile + rename. Lock wait is bounded
  (~5s) with a structured timeout error. Third-party tools that want
  to edit `bullseye.yaml` safely alongside bullseye should acquire
  the same `flock(2)` / `LockFileEx` on the sibling lockfile — see
  the README's "Concurrency protocol" section. Visible to callers
  as: (a) new error variants on mutating tools (timeout, conflict),
  (b) a persistent `bullseye.yaml.lock` sentinel file next to every
  yaml, (c) previously-possible lost-update races in multi-session
  use no longer occur.

Earlier changes still in effect from v0.17.0:
- **`bullseye_put` value/cost optional at repo scope** (🎯T11).
  `value` and `cost` are no longer required on create. Both default
  to `0.0`, which the validator now accepts as the "not set at repo
  scope" sentinel (previously required `> 0`). They remain
  portfolio-scope inputs only — never consumed by repo-level
  ordering — so omitting them is appropriate when the target is
  meant for single-repo work. Set them when the target should
  participate in cross-repo WSJF ranking.

Earlier changes still in effect from v0.16.0:
- **Per-repo storage, no machine-wide config** (redesign of 🎯T12).
  The v0.15.0 machine-wide `~/.config/bullseye/config.yaml` and
  `bullseye_configure` tool are **removed**. Location is now a
  per-repo property, encoded by where `bullseye.yaml` lives on disk:
  - `in_repo` — `bullseye.yaml` inside the project, discovered by
    walking up from `cwd`.
  - `external` — `bullseye.yaml` in a shadow tree under
    `~/.local/share/bullseye/` mirroring the absolute `cwd`.
  Every tool calls `discover_anywhere(cwd)`, which checks the
  in-repo walk-up first and then the shadow walk-up. If both exist
  (edge case), **in-repo wins**. There is no config file to sync
  across machines, no global default, and no machine-wide state.

- **`bullseye_init` requires `location`** — `"in_repo"` or
  `"external"`. Called without it, the tool returns the locked
  prompt (`config::LOCATION_PROMPT`) so the agent can ask the user.
  Refuses to create a file if one already exists in either location.

- **`bullseye_import` requires `location`** — same semantics as
  `bullseye_init`. Also refuses to overwrite an existing file in
  either location unless `force: true`.

- **`bullseye_put` no longer auto-creates** — returns the locked
  prompt (via `load_file`'s not-found message) when no targets file
  exists for `cwd`. Forces the location choice to happen once,
  explicitly, at `bullseye_init` time rather than on an arbitrary
  first `put`.

- **`bullseye_configure` removed.** `config::Config`,
  `config::Mode`, `config::Storage`, `config::ConfigError`,
  `config::FIRST_RUN_PROMPT`, and `config::load`/`save` are gone.
  The remaining public surface of the module is
  `config::Location`, `config::external_root`,
  `config::expand_tilde`, `config::LOCATION_PROMPT`, and
  `config::set_external_root_override` (tests only).

- **Invariant retained**: every tool entry terminates in either a
  successful result or an explicit actionable error. Missing-file
  errors embed the locked prompt so the agent can route the user to
  `bullseye_init` in one hop.

Earlier changes still in effect from v0.15.0:
- **Portfolio-level WSJF ranking across repos** (🎯T2.3).
  `bullseye_portfolio` ranks repos by aggregate WSJF with per-target
  `momentum` multipliers and cross-repo enabler-boost propagation via
  `cross_enables` edges. The `momentum` parameter shape is a list of
  `{id, multiplier}` entries (same shape as `bullseye_summary`).

Earlier changes still in effect from v0.14.0:
- **`bullseye_render` removed** (🎯T10). Markdown rendering is deleted
  entirely — the render module, tool, and auto-render-on-save are gone.
  `bullseye_import` no longer auto-discovers markdown files; the `path`
  parameter is now required.
- **File renamed**: `targets.yaml` → `bullseye.yaml`, moved from `docs/`
  to repo root. `store::discover` looks only at root-level
  `bullseye.yaml`. The dual-layout complexity (docs/ vs root, candidate
  lists, precedence rules) is eliminated.

Earlier changes still in effect from v0.13.0:
- **`bullseye_tunnels` semantics generalised** (🎯T7). Previously
  a tunnel was "a work target with no **verify target** reachable
  within `max_depth` hops"; now it is "a work target with no
  **observable target** reachable within `max_depth` hops", where
  observable = `kind: verify` OR the new `observable: true` flag
  on work targets. Existing callers still get warnings — the
  membership predicate just widened. The observable flag itself
  is a new opt-in schema field, so legacy targets files carry no
  observable work targets and every such target is a tunnel until
  the human reshapes the graph. This is the intended signal: the
  repo-level ordering now rewards shortest-path-to-observable
  rather than static value/cost maths.
- **`bullseye_put` refuses content patches on achieved targets**
  (🎯T8). Name/acceptance/context/value/cost/tags/depends_on/
  verifies/observable edits on an achieved target now return an
  explanatory error. The remedy is to re-open the target with
  `status: identified` first (either in a prior call, or atomically
  in the same call alongside the content edits). Status-only
  transitions on achieved targets remain allowed, and `bullseye_retire`
  is unchanged. Callers that previously relied on silent patches
  over historical state will need to insert a reopen step.

Earlier structural changes still in effect:
1. **`bullseye_assert` renamed to `bullseye_put`** (v0.12.0). The old
   name implied "verify a condition, crash if false" but the
   semantics were always REST-style upsert.
2. **WSJF ranking purged** (v0.11.0). Frontier-first scheduling is
   the model; the frontier section itself carries the prioritised
   list. `top_n` parameter removed. `momentum` remains but is no
   longer consumed by repo-level ordering as of v0.13.0 — see the
   phase-boundary note below.
3. **`bullseye_convergence`** (v0.11.0). Single-call convergence
   evaluation; absorbs the old multi-call `/cv` worker pattern.

**Phase-boundary hypothesis** (new in v0.13.0, `docs/mcp-triad.md`
§9): Bullseye now has two prioritisation scopes with different
objective functions. Repo-level (sub-week, human as decision-maker)
uses shortest-path-to-next-checkpoint, tiebroken by unblocking
fanout; per-target `value`/`cost` are **not consumed** by repo-level
ordering. Portfolio-level (weekly-plus, human as
bottleneck allocator) will use WSJF + momentum + cross-repo value
propagation (🎯T2.3, pending). The repo/portfolio split means per-
target `value`/`cost` are now documented as portfolio-scope inputs
only.

The settling clock for 1.0 eligibility restarts from v0.14.0
(removing `bullseye_render` is a tool-surface break; file rename
from `targets.yaml` to `bullseye.yaml` is a file-format break).

### MCP tools

| Tool | Status | Notes |
|------|--------|-------|
| `bullseye_list(cwd, filter)` | Stable | Filter values (active/achieved/all) are settled |
| `bullseye_get(cwd, id)` | Stable | |
| `bullseye_put(cwd, id?, name?, value?, cost?, acceptance?, depends_on?, blocks?, showcase?, ...)` | Needs review | Unified upsert (create-or-patch). Introduced in v0.8.0 as `bullseye_assert`; renamed to `bullseye_put` in v0.12.0. v0.13.0 added an `observable` parameter (now `showcase` as of v0.19.0) and refuses content patches on achieved targets — see 🎯T8. v0.17.0 makes `value`/`cost` optional on create (default `0.0`, the "not set at repo scope" sentinel) — see 🎯T11. v0.19.0 renames the parameter to `showcase` to match the schema rename — see 🎯T14. |
| `bullseye_retire(cwd, id, actual_cost, demonstration?)` | Needs review | v0.19.0 adds a `demonstration` parameter, **required** when the target carries `showcase: true` and ignored otherwise. Refuses retirement of a showcase target without a non-empty demonstration string — see 🎯T14. |
| `bullseye_frontier(cwd)` | Stable | v0.13.0 ordering: ascending distance-to-nearest-checkpoint, tiebroken by unblocking fanout, then ID. Per-target value/cost are NOT consumed. |
| `bullseye_rework(cwd, id, diagnosis)` | Stable | |
| `bullseye_tunnels(cwd, max_depth)` | Needs review | v0.13.0 generalised from "no verify within N hops" to "no checkpoint within N hops" (verify-kind targets, plus work-kind targets with `showcase: true` as of v0.19.0). Membership predicate widened; output format unchanged. Will need another review pass once the `showcase: true` flag sees real-world use. |
| `bullseye_verify(cwd, id)` | Fluid | New in v0.13.0. Emits a structured plan (markdown + JSON) mapping each check on the target to a sawmill tool invocation. Bullseye does not execute the plan — the calling agent runs it against sawmill and folds results back into a report. The plan-only (no result-feedback) shape may evolve once we see how `/cv` or similar wrappers consume it. |
| `bullseye_validate(cwd)` | Stable | Validation rules will grow but existing ones won't change |
| `bullseye_graph(cwd)` | Stable | |
| `bullseye_import(cwd, path, force)` | Stable | Markdown-to-YAML migration. `path` is now required (auto-discovery removed in v0.14.0). |
| `bullseye_init(cwd, project_name)` | Stable | Refuses to overwrite existing file |
| `bullseye_startup_context(cwd, recent_days)` | Needs review | v0.9.0 degrades gracefully on missing / unreadable / unparsable files; still fails loudly on `schema_version` mismatch. |
| `bullseye_portfolio(root, max_depth)` | Needs review | v0.9.0 surfaces load warnings (especially `schema_version` mismatches) under a `## ⚠ Warnings` section instead of silently dropping affected repos. |
| `bullseye_summary(cwd, momentum?, frontier_details?)` | Needs review | `momentum` added in v0.9.0, reshaped in v0.10.0 to a list of `{id, multiplier}` entries. WSJF-ranking section removed in v0.11.0 — frontier-first scheduling is the model, and the frontier section itself is the prioritised list (ordered by `value × momentum`). `frontier_details: true` expands each frontier entry with full acceptance, context, and edges. Composition happens at the skill layer; bullseye never calls mnemo. |
| `bullseye_convergence(cwd, momentum?, skip_invariants?)` | Needs review | New in v0.11.0. Single-call convergence evaluation: invariants via `make bullseye` / `mk bullseye`, git-based unreleased-fix detection, summary with inline frontier details, and a deterministic next-action recommendation. Absorbs most of the old `/cv` worker logic into a stateless tool call. Missing hook degrades gracefully with embedded setup instructions; frontier recommendation still fires. |

**Removed in v0.8.0** (breaking):
- `bullseye_add` — replaced by the upsert tool (`bullseye_put` as of v0.12.0; was `bullseye_assert` in v0.8.0–v0.11.0)
- `bullseye_update` — replaced by the upsert tool (same)

**Renamed in v0.12.0** (breaking):
- `bullseye_assert` → `bullseye_put`

**Removed in v0.14.0** (breaking):
- `bullseye_render` — markdown rendering deleted entirely (🎯T10)

**Behavioural changes in v0.14.0**:
- `bullseye_import` `path` parameter is now required (auto-discovery
  of markdown files removed with render module)
- File discovered as `bullseye.yaml` at repo root (was
  `docs/targets.yaml` or `targets.yaml`)

**Behavioural changes in v0.13.0**:
- `bullseye_tunnels` membership predicate generalised from verify-
  reachability to observable-reachability (🎯T7).
- `bullseye_put` rejects content patches on achieved targets (🎯T8).

Planned additions (not yet implemented):
- Cross-repo value propagation in portfolio ranking (🎯T2.3) —
  upgrades the binary `cross_enables` flag from v0.13.0 to
  weighted propagation, and adds per-repo WSJF scoring at the
  portfolio boundary.

### bullseye.yaml schema

| Field | Status | Notes |
|-------|--------|-------|
| `schema_version` | Stable | New in v0.9.0. Required going forward; current value `2` (v0.19.0). Absent on legacy files (treated as v1 on load and stamped on next save). Bullseye refuses to load files whose `schema_version` exceeds the binary's compiled `CURRENT_SCHEMA_VERSION` and prompts for `brew upgrade`. Incremented only on breaking schema changes. |
| `targets` (map) | Stable | |
| `last_evaluated` | Stable | |
| `name` | Stable | |
| `kind` (work/verify) | Stable | |
| `status` (identified/converging/achieved) | Stable | |
| `value`, `cost` | Stable | Fibonacci scale |
| `actual_cost` | Stable | |
| `acceptance` | Stable | |
| `checks` | Fluid | New in v0.13.0. List of executable checks, each one of `convention: name` / `query: {...}` / `invariant: name`. Consumed by `bullseye_verify` which emits a sawmill invocation plan. `invariant` variant is schema-ready for a future sawmill T19; `convention` and `query` are live today. Serde shape is `#[serde(untagged)]` so each entry is a single-key map. |
| `context` | Stable | |
| `depends_on` | Stable | Single edge type (v0.8.0); legacy `gates` edges are migrated into `depends_on` on load |
| `cross_depends`, `cross_enables` | Fluid | New in v0.13.0. Advisory edges (don't block frontier computation) pointing at targets or capabilities in other repos. `CrossEdge { repo, target?, capability?, note? }` — each edge must have a non-empty `repo` and at least one of `target`/`capability`; dangling refs to unscanned repos are silently allowed. Surfaced in `bullseye_portfolio` output today; value propagation is 🎯T2.3. |
| `showcase` | Fluid | Renamed from `observable` in v0.19.0; legacy YAML still loads via a serde alias on the new field. Boolean flag (default false, omitted from YAML when false). Marks a work target whose retirement requires a user-visible demonstration recorded via `bullseye_retire`'s `demonstration` parameter. Verify-kind targets are checkpoints by definition; this flag promotes a work target to checkpoint status (driving distance-to-checkpoint frontier ordering and `bullseye_tunnels` membership) AND obliges the demo on retire. Expected to see churn as the convention for marking showcase work targets settles in practice. |
| `demonstration` | Fluid | New in v0.19.0. Optional string, populated by `bullseye_retire` when the retired target carries `showcase: true`; never serialised when absent. The recorded note is the permanent evidence that the showcase obligation was discharged with a real user-visible step rather than a "tests pass" stand-in. |
| `verifies` | Stable | |
| `rework` | Stable | |
| `retry_budget`, `retries` | Stable | |
| `tags` | Stable | |
| `origin` | Stable | |
| `discovered`, `achieved` | Stable | |

Planned additions:
- Cross-repo value propagation (🎯T2.3) — upgrades `cross_enables`
  from a binary flag to weighted value propagation via portfolio
  scan lookup.

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
| Mermaid graph output | Needs review | Node/edge styling may change |
| Tool response text format | Fluid | Not yet formalised; consumers should parse loosely |

## Gaps and prerequisites for 1.0

- **Tool response format**: Responses are unstructured text. Consider
  returning structured JSON alongside text for programmatic consumers.
- **`bullseye_verify` feedback loop**: The plan-only shape shipped in
  v0.13.0 has no mechanism to fold sawmill results back into a
  structured pass/fail report on the target. Before 1.0 this should
  either be added (so verification can auto-trigger `bullseye_rework`)
  or the decision to stay plan-only should be documented as deliberate.
- **`showcase` flag convention**: Renamed from `observable` in
  v0.19.0 and given an explicit retirement obligation; the convention
  for when to mark a work target `showcase: true` will settle with
  practice. Gotchas and anti-patterns should be documented as they
  emerge.
- **`bullseye_startup_context`, `bullseye_portfolio`, `bullseye_summary`
  stabilisation**: All three are new and their output formats may evolve
  with real-world usage.
- **`bullseye_put` stabilisation**: Unified upsert replacing add/update.
  Needs real-world usage before locking in the parameter set — the
  `blocks` sugar field in particular may see iteration (e.g., symmetric
  `gated_by`, `verified_by` sugars).
- **Portfolio-level WSJF (🎯T2.3)**: Cross-repo enabler propagation is
  currently a binary flag; the value-weighted upgrade is pending.
  Should land before 1.0 so the portfolio engine is stable.
- **Settling threshold reset**: v0.16.0 redesigned storage discovery
  (per-repo location, `bullseye_configure` removed, `location`
  required on `bullseye_init`/`bullseye_import`, no auto-create on
  `bullseye_put`). The settling clock restarts from v0.16.0.
  v0.17.0 relaxes the `bullseye_put` input contract (`value`/`cost`
  optional on create) — additive, no reset. v0.18.0 introduces the
  lockfile protocol (🎯T17) — additive for bullseye's own tools
  but imposes a new expectation on third-party editors, so note the
  change here even though it's not a hard reset. v0.19.0 renames
  the `observable` field to `showcase` and adds the
  `demonstration`-on-retire obligation (🎯T14) — schema bumps to v2,
  the legacy field name still loads via a serde alias, the
  `bullseye_put` parameter renames in lockstep, and `bullseye_retire`
  gains the new required-when-showcase `demonstration` parameter.
  Field rename + new required parameter on a mutating tool — hard
  reset of the settling clock to v0.19.0.
- **Test coverage for CLI flags**: No tests for --version/--help/--help-agent.

## Out of scope for 1.0

- Protocol app sync — depends on external infrastructure (sqlpipe,
  pigeon).
- MCP resource support — waiting on protocol maturity.
