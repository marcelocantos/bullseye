# Stability

## Stability commitment

Version 1.0 will represent a backwards-compatibility contract. After
1.0, breaking changes to the MCP tool interface, bullseye.yaml schema,
or CLI flags will require a new product (not a major version bump).
The pre-1.0 period exists to get these right.

## Interaction surface catalogue

Snapshot as of v0.15.0. Changes since v0.14.0 affecting the
interaction surface:

- **External storage mode added** (🎯T12). Bullseye now requires a
  one-time machine-wide choice between two storage modes, recorded
  at `~/.config/bullseye/config.yaml`:
  - `mode: in_repo` — `bullseye.yaml` lives in the repo, discovered
    by walking up from `cwd` (original behaviour).
  - `mode: external` — `bullseye.yaml` lives in a shadow tree under
    `storage.root` (default `~/.local/share/bullseye/`) mirroring
    the absolute `cwd`. Discovery walks up the shadow tree the same
    way in-repo mode walks up the real tree. Path-driven; no
    git-remote, host/org/repo, or layout assumptions.

  A new `bullseye_configure` MCP tool records the choice. Until it
  is called, every other tool returns a structured error containing
  the first-run prompt for the agent to relay to the user. The
  prompt wording is locked in `config::FIRST_RUN_PROMPT`:
  "Store targets where? in_repo — commit bullseye.yaml into the
  repo (you own it, team uses bullseye). external — shadow tree
  under ~/.local/share/bullseye/ (read-only repo, or personal use
  of bullseye). Answer: in_repo or external. Machine-wide; edit
  ~/.config/bullseye/config.yaml to change."

  Invariant: every tool entry point terminates in success-with-config
  or an explicit actionable error; malformed config, filesystem
  errors, and unknown modes surface as hard errors with no silent
  fallback.

- **Portfolio-level WSJF ranking** (🎯T2.3). `bullseye_portfolio`
  now ranks repos by aggregate WSJF, with per-target momentum
  multipliers and cross-repo enabler-boost propagation. The
  `momentum` parameter accepts a list of `{id, multiplier}` entries.
  Cross-repo edges (`cross_enables`) are resolved during the scan.

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
uses shortest-path-to-next-observable-checkpoint, tiebroken by
unblocking fanout; per-target `value`/`cost` are **not consumed**
by repo-level ordering. Portfolio-level (weekly-plus, human as
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
| `bullseye_put(cwd, id?, name?, value?, cost?, acceptance?, depends_on?, blocks?, observable?, ...)` | Needs review | Unified upsert (create-or-patch). Introduced in v0.8.0 as `bullseye_assert`; renamed to `bullseye_put` in v0.12.0. v0.13.0 adds an `observable` parameter (for the new schema field) and refuses content patches on achieved targets — see 🎯T8. |
| `bullseye_retire(cwd, id, actual_cost)` | Stable | |
| `bullseye_frontier(cwd)` | Stable | v0.13.0 ordering: ascending distance-to-nearest-observable-target, tiebroken by unblocking fanout, then ID. Per-target value/cost are NOT consumed. |
| `bullseye_rework(cwd, id, diagnosis)` | Stable | |
| `bullseye_tunnels(cwd, max_depth)` | Needs review | v0.13.0 generalised from "no verify within N hops" to "no observable within N hops". Membership predicate widened; output format unchanged. Will need another review pass once the `observable: true` flag sees real-world use. |
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
| `schema_version` | Stable | New in v0.9.0. Required going forward; current value `1`. Absent on legacy files (treated as v1 on load and stamped on next save). Bullseye refuses to load files whose `schema_version` exceeds the binary's compiled `CURRENT_SCHEMA_VERSION` and prompts for `brew upgrade`. Incremented only on breaking schema changes. |
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
| `observable` | Fluid | New in v0.13.0 boolean flag (default false, omitted from YAML when false). Marks a work target whose completion produces reviewable output. Verify-kind targets are observable by definition; this flag only matters for work-kind targets. Drives the distance-to-nearest-observable signal that orders the repo-level frontier and the `bullseye_tunnels` membership predicate. Expected to see churn as the convention for marking observable work targets settles in practice. |
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
- **`observable` flag convention**: Newly introduced; the convention
  for when to mark a work target `observable: true` will settle with
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
- **Settling threshold reset**: v0.14.0 removed `bullseye_render` and
  renamed the targets file from `targets.yaml` to `bullseye.yaml`.
  Both are interaction-surface breaks. The settling clock restarts
  from v0.14.0.
- **Test coverage for CLI flags**: No tests for --version/--help/--help-agent.

## Out of scope for 1.0

- Protocol app sync — depends on external infrastructure (sqlpipe,
  pigeon).
- MCP resource support — waiting on protocol maturity.
