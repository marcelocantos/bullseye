# Stability

## Stability commitment

Version 1.0 will represent a backwards-compatibility contract. After
1.0, breaking changes to the MCP tool interface, targets.yaml schema,
or CLI flags will require a new product (not a major version bump).
The pre-1.0 period exists to get these right.

## Interaction surface catalogue

Snapshot as of v0.11.0. Two structural changes since v0.10.0:

1. **WSJF ranking purged.** The `## WSJF ranking (top N)` section in
   `bullseye_summary` — which had quietly reappeared in v0.7.0 after
   being removed in v0.4.0 — is gone. Frontier-first scheduling is
   the model. The frontier section itself is now the prioritised
   list, ordered by `focus = value × momentum`. The `top_n` parameter
   on `bullseye_summary` is removed. Momentum stays as an advisory
   reordering signal, not a ranking primitive.
2. **`bullseye_convergence` added.** Single-call convergence
   evaluation that runs `make bullseye` for standing invariants,
   scans git for unreleased fixes, emits the target summary with
   inline frontier details, and computes a deterministic next-action
   recommendation. Replaces the multi-call `/cv` worker pattern.

Both are breaking changes to `bullseye_summary`'s output format (no
more WSJF section, `top_n` parameter gone), acceptable pre-1.0. The
settling clock for 1.0 eligibility continues from v0.8.0.

### MCP tools

| Tool | Status | Notes |
|------|--------|-------|
| `bullseye_list(cwd, filter)` | Stable | Filter values (active/achieved/all) are settled |
| `bullseye_get(cwd, id)` | Stable | |
| `bullseye_assert(cwd, id?, name?, value?, cost?, acceptance?, depends_on?, blocks?, ...)` | Needs review | New in v0.8.0 — unified add/update upsert; optional fields may expand |
| `bullseye_retire(cwd, id, actual_cost)` | Stable | |
| `bullseye_frontier(cwd)` | Stable | |
| `bullseye_rework(cwd, id, diagnosis)` | Stable | |
| `bullseye_tunnels(cwd, max_depth)` | Stable | |
| `bullseye_validate(cwd)` | Stable | Validation rules will grow but existing ones won't change |
| `bullseye_graph(cwd)` | Stable | |
| `bullseye_render(cwd)` | Stable | |
| `bullseye_import(cwd, path, force)` | Stable | Markdown-to-YAML migration |
| `bullseye_init(cwd, project_name)` | Stable | Refuses to overwrite existing file |
| `bullseye_startup_context(cwd, recent_days)` | Needs review | v0.9.0 degrades gracefully on missing / unreadable / unparsable files; still fails loudly on `schema_version` mismatch. |
| `bullseye_portfolio(root, max_depth)` | Needs review | v0.9.0 surfaces load warnings (especially `schema_version` mismatches) under a `## ⚠ Warnings` section instead of silently dropping affected repos. |
| `bullseye_summary(cwd, momentum?, frontier_details?)` | Needs review | `momentum` added in v0.9.0, reshaped in v0.10.0 to a list of `{id, multiplier}` entries. WSJF-ranking section removed in v0.11.0 — frontier-first scheduling is the model, and the frontier section itself is the prioritised list (ordered by `value × momentum`). `frontier_details: true` expands each frontier entry with full acceptance, context, and edges. Composition happens at the skill layer; bullseye never calls mnemo. |
| `bullseye_convergence(cwd, momentum?, skip_invariants?)` | Needs review | New in v0.11.0. Single-call convergence evaluation: invariants via `make bullseye` / `mk bullseye`, git-based unreleased-fix detection, summary with inline frontier details, and a deterministic next-action recommendation. Absorbs most of the old `/cv` worker logic into a stateless tool call. Missing hook degrades gracefully with embedded setup instructions; frontier recommendation still fires. |

**Removed in v0.8.0** (breaking):
- `bullseye_add` — replaced by `bullseye_assert` (upsert)
- `bullseye_update` — replaced by `bullseye_assert` (upsert)

Planned additions (not yet implemented):
- `bullseye_verify` — execute acceptance checks via sawmill

### targets.yaml schema

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
| `context` | Stable | |
| `depends_on` | Stable | Single edge type (v0.8.0); legacy `gates` edges are migrated into `depends_on` on load |
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
- **`bullseye_verify`**: Core planned tool not yet implemented. Should
  be present before 1.0.
- **`bullseye_startup_context`, `bullseye_portfolio`, `bullseye_summary`
  stabilisation**: All three are new and their output formats may evolve
  with real-world usage.
- **`bullseye_assert` stabilisation**: New in v0.8.0 as a unified upsert
  replacing add/update. Needs real-world usage before locking in the
  parameter set — the `blocks` sugar field in particular may see
  iteration (e.g., symmetric `gated_by`, `verified_by` sugars).
- **Settling threshold reset**: The v0.8.0 release removes `bullseye_add`
  and `bullseye_update` and retires the `gates` schema field, both
  breaking changes. The settling clock restarts from v0.8.0.
- **Test coverage for CLI flags**: No tests for --version/--help/--help-agent.

## Out of scope for 1.0

- Protocol app sync — depends on external infrastructure (sqlpipe,
  pigeon).
- MCP resource support — waiting on protocol maturity.
