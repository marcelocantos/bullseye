# Stability

## Stability commitment

Version 1.0 will represent a backwards-compatibility contract. After
1.0, breaking changes to the MCP tool interface, bullseye.yaml schema,
or CLI flags will require a new product (not a major version bump).
The pre-1.0 period exists to get these right.

## Interaction surface catalogue

Snapshot as of v0.27.0. Changes since v0.26.0 affecting the
interaction surface:

- **The `showcase` construct is removed** (🎯T23). Empirically the
  flag did not change agent behaviour — the user-visible
  demonstration step it was meant to force still got skipped,
  while the field carried schema, doc, and tool surface that
  callers had to reason about. Removed: the `Target.showcase`
  field, the `Target.demonstration` field, the `showcase`
  parameter on `bullseye_put`, the `demonstration` parameter on
  `bullseye_retire`, and the showcase-obligation enforcement on
  retire. Schema bumps to `schema_version: 4`. v3 yaml files load
  cleanly via permissive deserialisation; the `showcase`,
  `observable`, and `demonstration` keys are silently dropped on
  next save (one-shot migration, equivalent to how `observable`
  → `showcase` migrated in v0.19.0). Older binaries reading a v4
  file fail loudly via the existing version check. `is_checkpoint`
  is now exactly `kind == Verify`; tunnel-resolution candidate
  output recommends "add a verify target above this candidate"
  full stop (no more "flip showcase: true on a candidate"
  alternative). The output `[showcase]` annotation in
  `bullseye_frontier` / `bullseye_summary` is gone. Settling clock
  hard-resets to v0.27.0 — schema bump + parameters removed from
  two mutating tools.

Earlier v0.26.0 changes still in effect:

- **Advisory validation warnings no longer suppress `## Frontier`**
  in `bullseye_summary` / `bullseye_convergence`. The summary
  renderer now gates the inline frontier section on
  `validate_blocking` (structural errors only), matching the
  documented contract on `validate()` and the existing behaviour
  of `frontier`, `convergence`, and `portfolio`. Previously, any
  cosmetic warning (today, only the non-conforming-target-ID check
  on IDs like `T34a`) collapsed the convergence response into
  `## Active targets by group` + `## Validation errors` and stripped
  the frontier-detail section the `/cv` skill consumes for fan-out.
  Warnings continue to render under their own `## Validation
  warnings` heading via `convergence`. No public Rust API change.
  See `src/graph.rs:983`.

Earlier v0.25.0 changes still in effect:

- **Bullseye self-commits a dirty `bullseye.yaml`** (🎯T22). Every
  mutating tool call (`bullseye_put`, `bullseye_retire`,
  `bullseye_set_aside`, `bullseye_rework`, `bullseye_init`,
  `bullseye_import`) and the start of `bullseye_convergence` now
  fold a dirty `bullseye.yaml` into git automatically. Decision
  rule: if the most recent commit on `HEAD` is unpushed and its set
  of changed files is exactly `{bullseye.yaml}`, fold the new
  state via `git commit --amend --no-edit` (existing message
  preserved); otherwise create a fresh `Update bullseye.yaml`
  commit containing only `bullseye.yaml` (other staged changes
  are left in the index untouched). Best-effort: outside a git
  repo (shadow-tree storage, tempdir tests) and on git failures
  the auto-commit is a silent no-op and the file stays dirty.
  No new public Rust API; the new `git_commit` module is internal.
  Eliminates the prior `/cv` skill workaround for the dirty-tree
  invariants block. See 🎯T22.

Earlier v0.24.0 changes still in effect:

- **`bullseye sync-priorities` CLI subcommand** (🎯T3.1). New
  cron-targetable subcommand that scans the workspace via the
  portfolio engine and upserts each frontier target into a SQLite
  `targets_priorities` table (`id` PRIMARY KEY = `"{repo}/{target_id}"`,
  `repo`, `name`, `priority` REAL = per-target WSJF, `context`,
  `horizon`, `updated_at`). Stale rows whose targets are no longer on
  any repo's frontier are deleted in the same transaction. Flags:
  `--db PATH` (default `$BULLSEYE_DATA_DIR/priorities.db`),
  `--root PATH` (default `~/work`), `--horizon STR` (default `today`),
  `--max-depth N` (default 5). Designed for periodic invocation from
  cron or a daemon hook; downstream sync chain (sqlpipe-over-pigeon →
  Protocol app's `protocol.db`) is tracked separately. Adds a new
  bundled SQLite dependency (`rusqlite = "0.39.0"` with `bundled`
  feature) — no host SQLite required. New public Rust API:
  `priorities::{open, sync, SyncCounts, SyncArgs, run_sync,
  parse_sync_args, default_db_path}`. The `portfolio::PortfolioFrontierTarget`
  struct gains a public `context: Option<String>` field threaded
  through `summarize_repo_raw` so the writer doesn't reload each
  repo's targets file. See `docs/mcp-triad.md` §7 for the full
  Protocol-app sync chain.

- **Tunnel warnings on every mutation** (🎯T21). After every
  successful `bullseye_put` / `bullseye_retire` / `bullseye_set_aside`,
  the post-mutation graph is re-evaluated for tunnels (work targets
  with no checkpoint reachable within 2 hops) and a `## ⚠ Tunnel
  warnings` section is appended to the response listing each
  orphaned target with its distance-to-checkpoint result and ranked
  candidate checkpoint locations (Convergence > Root > Self >
  OnPath). `bullseye_validate` surfaces the same warnings.
  **Mutations are never rejected** — the warning is informational, so
  the agent can add a verify target above a candidate in a follow-up
  `bullseye_put`. Eliminates the prior failure mode where
  `bullseye_convergence` discovered a tunnel three operations after
  the mutation that introduced it, with the original context lost.
  Hard rejection was considered and dismissed: empirically the fix
  is almost always one boolean toggle and forcing a two-step ceremony
  for every such edit produces no benefit. New public Rust API:
  `graph::{format_tunnel_warnings, suggest_checkpoint_candidates,
  CheckpointCandidate, CandidateReason}`. The
  `bullseye_convergence`-side "Blocked: tunnel" recommendation
  remains as a last-resort backstop for the rare case where the
  warning gets ignored.

- **Target-level `strategy` block in schema** (🎯T15.2). A new
  optional `strategy: Strategy` field on `Target` lets a target
  declare how it converges mechanically alongside its acceptance
  criteria. `Strategy { command: String, trigger: String, timeout:
  Option<String>, retry: Option<RetryPolicy> }` and `RetryPolicy {
  max_attempts: Option<u32>, backoff: Option<String> }`. The
  `command` and `trigger` fields are required and validated
  non-empty (after trimming) by `bullseye_validate` whenever a
  strategy is present; `timeout` and `retry` are free-form strings
  that the future executor (🎯T15) parses. Bullseye itself does
  not execute strategies — it only persists them. Targets without a
  `strategy` field are unaffected; serde uses
  `#[serde(skip_serializing_if = "Option::is_none")]` so legacy
  files round-trip unchanged. Schema bump deferred — purely
  additive optional field, schema_version stays at 3. Forward-
  compat for the executor work in 🎯T15 (lives in a separate repo,
  TBD).

- **MCP triad integrations achieved end-to-end** (🎯T1.4, 🎯T1.5,
  🎯T1). No bullseye-side surface change in v0.23.0 — both upstream
  pieces (mnemo summarizer + mnemo_rework_history) shipped on the
  mnemo side (mnemo v0.28.0 / mnemo PR #63) and the bullseye-side
  surface for both was already in place in v0.21.0. v0.23.0 records
  the achievement: the rework-payload composition and target-aware
  compaction now work end-to-end in real sessions. Tracked here
  because the rework-payload `Option<String>` shape (string-encoded
  JSON) was a deliberate workaround for the Anthropic API's
  rejection of `serde_json::Value` and that decision is now load-
  bearing across the live integration.

Earlier changes still in effect from v0.22.0:

- **Tool-call envelope leak guard on every mutating handler**
  (🎯T20). All mutating tools (`bullseye_put`, `bullseye_retire`,
  `bullseye_set_aside`, `bullseye_rework`, `bullseye_import`) now
  reject any caller-controlled string that contains a leaked Claude
  tool-call XML envelope marker — `<invoke `, `</invoke>`,
  `<parameter `, or `</parameter>`. On detection, the tool returns
  an actionable error naming the field and the marker, and the file
  is not mutated. Validation is at the write boundary only —
  `store::load` continues to load corrupted files unchanged so an
  operator can repair them. Generic angle-bracket content (e.g.
  `<context>` substrings, mathematical comparisons, prose mentioning
  closing tags) passes; only the four protocol-specific markers are
  exact-matched. Behavioural addition rather than schema change —
  callers passing valid content see no difference. Filed after
  malformed YAML was observed in another repo where an agent
  serialised a bullseye_put call as XML and the harness's wrapper-
  stripping was incomplete, leaving `</invoke>` tags inside the
  persisted `context:` field. v0.23.0 closes the test acceptance
  gaps (added `import_rejects_envelope_markers_in_parsed_markdown`,
  `store_load_still_loads_corrupted_files`; promoted `handle_import`
  to `pub` for symmetry with the other tested handlers).

Earlier changes still in effect from v0.21.0:

- **Structured rework payloads** (🎯T1.5). `bullseye_rework` gains
  two optional parameters, `sawmill_failure` and `mnemo_history`,
  each a JSON-encoded string. When provided, both are validated as
  JSON, pretty-printed, and persisted into the rework target's
  `context` as fenced ```json blocks under labelled `##` headers
  (`## Sawmill failure` and `## Prior attempts (mnemo)`) alongside
  the existing free-form `diagnosis` prose. The composition keeps
  bullseye decoupled from sawmill's and mnemo's internal payload
  shapes — both are persisted opaquely. The parameters use
  `Option<String>` rather than `Option<serde_json::Value>` because
  the JsonSchema derive in rust-mcp-sdk falls back to
  `type: "unknown"` for arbitrary JSON, which the Anthropic API
  rejects on tool-list submission. Additive change — existing
  callers passing only `diagnosis` see no behavioural difference.
  Sawmill side is end-to-end usable today (sawmill T24 ships
  structured failure payloads); the mnemo half waits on an upstream
  rework-aware query that has not yet been filed.

Earlier changes still in effect from v0.20.0 carry through v0.21.0
unchanged:

- **`set_aside` status with required rationale** (🎯T18). Adds a
  fourth terminal disposition alongside `achieved` for targets the
  user decides not to pursue (parked / deferred / wont_fix). The new
  `set_aside_reason: String` field on `Target` is required and
  non-empty whenever `status == set_aside`; `bullseye_validate`
  flags missing reasons and stale leftover reasons on non-set-aside
  statuses. New tool `bullseye_set_aside(cwd, id, reason)` is the
  canonical transition path; `bullseye_put` rejects
  `status: set_aside` and routes the caller to the dedicated tool.
  Set-aside targets unblock their dependents the same way achieved
  targets do (terminal for graph traversal) but render in a
  separate `## Set aside` group in `bullseye_summary` so they don't
  inflate the achievements count. Schema bumps to
  `schema_version: 3`. Old binaries reading a v3 file fail loudly
  via the existing version check.
- **Lockfile relocated outside the project directory** (🎯T19).
  Bullseye no longer writes `bullseye.yaml.lock` next to
  `bullseye.yaml`; lockfiles now live under
  `std::env::temp_dir()/bullseye/locks/`, named by the parent
  directory's hex `(dev_t, ino_t)`. The keying is robust against
  atomic-rename writes, repo directory renames, and symlinked
  access paths (canonicalised before stat). Auto-clears on reboot
  via temp-dir semantics. Operational change rather than a tool
  surface change, but documented here because third-party writers
  that previously coordinated with bullseye via the sibling
  lockfile need to either replicate the new keying or rely on
  bullseye's CAS-on-`(mtime, len)` conflict detection to catch
  their edit and retry.

Earlier changes still in effect from v0.19.0:

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
| `bullseye_put(cwd, id?, name?, value?, cost?, acceptance?, depends_on?, blocks?, ...)` | Needs review | Unified upsert (create-or-patch). Introduced in v0.8.0 as `bullseye_assert`; renamed to `bullseye_put` in v0.12.0. v0.13.0 added an `observable` parameter (renamed `showcase` in v0.19.0, removed entirely in v0.27.0) and refuses content patches on achieved targets — see 🎯T8. v0.17.0 makes `value`/`cost` optional on create (default `0.0`, the "not set at repo scope" sentinel) — see 🎯T11. v0.20.0 rejects `status: set_aside` and routes callers to `bullseye_set_aside` so the rationale is always recorded — see 🎯T18. v0.27.0 drops the `showcase` parameter — see 🎯T23. |
| `bullseye_retire(cwd, id, actual_cost?)` | Needs review | v0.27.0 drops the v0.19.0 `demonstration` parameter alongside the schema-level showcase removal — `bullseye_retire` no longer enforces a recorded demonstration. See 🎯T23. |
| `bullseye_set_aside(cwd, id, reason)` | Needs review | New in v0.20.0. Sets the target's status to `set_aside` and records the rationale (parked / deferred / wont_fix — the schema deliberately doesn't taxonomise; the free-text reason carries the nuance). Refuses already-achieved targets and is idempotent on already-set-aside targets (original reason wins). Empty / whitespace-only reasons are rejected — the rationale is the load-bearing artefact of the disposition. See 🎯T18. |
| `bullseye_frontier(cwd)` | Stable | v0.13.0 ordering: ascending distance-to-nearest-checkpoint, tiebroken by unblocking fanout, then ID. Per-target value/cost are NOT consumed. |
| `bullseye_rework(cwd, id, diagnosis, sawmill_failure?, mnemo_history?)` | Needs review | v0.21.0 adds two optional JSON-encoded payload parameters that are validated and persisted into the rework target's context as labelled fenced JSON blocks. The string-encoded shape is a workaround for the rust-mcp-sdk JsonSchema derive emitting `type: "unknown"` for `serde_json::Value` (which the Anthropic API rejects); a typed wrapper may emerge once sawmill's and mnemo's payload schemas settle. The mnemo half waits on an upstream rework-aware query — see 🎯T1.5. |
| `bullseye_tunnels(cwd, max_depth)` | Needs review | "No verify-kind target within N hops". v0.13.0 generalised the predicate to "no checkpoint within N hops" (verify-kind plus work-kind with the v0.19.0 `showcase: true` flag); v0.27.0 reverts the membership rule to verify-kind only after the showcase construct was retired (🎯T23). Output format unchanged. |
| `bullseye_verify(cwd, id)` | Fluid | New in v0.13.0. Emits a structured plan (markdown + JSON) mapping each check on the target to a sawmill tool invocation. Bullseye does not execute the plan — the calling agent runs it against sawmill and folds results back into a report. The plan-only (no result-feedback) shape may evolve once we see how `/cv` or similar wrappers consume it. |
| `bullseye_validate(cwd)` | Needs review | v0.24.0 surfaces tunnel warnings (with ranked candidate checkpoint locations) alongside the existing structural-error and stylistic-warning sections — see 🎯T21. Output gains a `## ⚠ Tunnel warnings` block when the active graph has any tunnels. Validation rules will continue to grow; existing rules unchanged. |
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
| `schema_version` | Stable | New in v0.9.0. Required going forward; current value `4` (v0.27.0). Absent on legacy files (treated as v1 on load and stamped on next save). Bullseye refuses to load files whose `schema_version` exceeds the binary's compiled `CURRENT_SCHEMA_VERSION` and prompts for `brew upgrade`. Incremented only on breaking schema changes. v0.19.0 bumped 1→2 for the `observable` → `showcase` rename + retire-demo obligation; v0.20.0 bumped 2→3 for the new `set_aside` status enum value (🎯T18); v0.27.0 bumps 3→4 for the showcase / demonstration field removal (🎯T23). |
| `targets` (map) | Stable | |
| `last_evaluated` | Stable | |
| `name` | Stable | |
| `kind` (work/verify) | Stable | |
| `status` (identified/converging/achieved/set_aside) | Stable | v0.20.0 adds `set_aside` as a fourth terminal disposition for parked / deferred / wont-fix targets. The set-aside transition is performed via `bullseye_set_aside(cwd, id, reason)` rather than `bullseye_put` so the rationale is always recorded. Set-aside targets unblock dependents like achieved targets (terminal for graph traversal) but render in a separate group in summary output. See 🎯T18. |
| `set_aside_reason` | Needs review | New in v0.20.0. Optional string at the schema level; **required and non-empty** at validation time when `status == set_aside`. Validation flags missing or whitespace-only reasons, and flags stale `set_aside_reason` values left on non-set-aside statuses. See 🎯T18. |
| `value`, `cost` | Stable | Fibonacci scale |
| `actual_cost` | Stable | |
| `acceptance` | Stable | |
| `checks` | Fluid | New in v0.13.0. List of executable checks, each one of `convention: name` / `query: {...}` / `invariant: name`. Consumed by `bullseye_verify` which emits a sawmill invocation plan. `invariant` variant is schema-ready for a future sawmill T19; `convention` and `query` are live today. Serde shape is `#[serde(untagged)]` so each entry is a single-key map. |
| `context` | Stable | |
| `depends_on` | Stable | Single edge type (v0.8.0); legacy `gates` edges are migrated into `depends_on` on load |
| `cross_depends`, `cross_enables` | Fluid | New in v0.13.0. Advisory edges (don't block frontier computation) pointing at targets or capabilities in other repos. `CrossEdge { repo, target?, capability?, note? }` — each edge must have a non-empty `repo` and at least one of `target`/`capability`; dangling refs to unscanned repos are silently allowed. Surfaced in `bullseye_portfolio` output today; value propagation is 🎯T2.3. |
| ~~`showcase`~~ | Removed | Introduced as `observable` in v0.13.0, renamed `showcase` in v0.19.0, removed in v0.27.0 (🎯T23). The flag did not change agent behaviour in practice. v3 yaml files load cleanly and the key is silently dropped on next save. |
| ~~`demonstration`~~ | Removed | Introduced in v0.19.0, removed in v0.27.0 (🎯T23). Same migration: v3 files load and the key is dropped. |
| `strategy` | Fluid | New in v0.23.0. Optional `Strategy { command: String, trigger: String, timeout: Option<String>, retry: Option<RetryPolicy> }`; `RetryPolicy { max_attempts: Option<u32>, backoff: Option<String> }`. Targets that declare a strategy will be converged mechanically by the future bullseye-native executor (🎯T15, lives in a separate repo). Bullseye persists and validates (rejects empty `command` / `trigger` after trim) but does not execute. `trigger` is free-form for now (`cron:0 * * * *`, `fswatch:/path`, `on_wake`, `manual`) — the executor parses it. Expected churn as the executor lands and real-world use clarifies which sub-fields need to become structured. |
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
| `sync-priorities [--db PATH] [--root PATH] [--horizon STR] [--max-depth N]` | Fluid | New in v0.24.0. Cron-targetable subcommand that scans the workspace and upserts the portfolio frontier into a SQLite `targets_priorities` table. The flag set may evolve once the Protocol-app sync chain is live and real usage clarifies what's needed (e.g. multi-horizon banding, per-repo include/exclude). See 🎯T3.1. |

### Output formats

| Format | Status | Notes |
|--------|--------|-------|
| Mermaid graph output | Needs review | Node/edge styling may change |
| Tool response text format | Fluid | Not yet formalised; consumers should parse loosely |
| `## ⚠ Tunnel warnings` section in mutating-tool / validate responses | Fluid | New in v0.24.0. Appended to `bullseye_put` / `bullseye_retire` / `bullseye_set_aside` / `bullseye_validate` output when the post-mutation active graph contains tunnels. Format and candidate-ranking heuristic (Convergence > Root > Self > OnPath) may evolve as real usage clarifies what nudges actually drive the right reshape. See 🎯T21. |
| `targets_priorities` SQLite schema | Fluid | New in v0.24.0. Written by `bullseye sync-priorities`, replicated to the Protocol app via sqlpipe-over-pigeon. Columns: `id` PRIMARY KEY (`"{repo}/{target_id}"`), `repo` TEXT NOT NULL, `name` TEXT NOT NULL, `priority` REAL NOT NULL, `context` TEXT, `horizon` TEXT NOT NULL DEFAULT `'today'`, `updated_at` TEXT NOT NULL. Schema is laptop-owned (Protocol is read-only). Column set may evolve as the phone-side rendering settles — particularly whether `priority` should carry richer banding info or whether `horizon` should be auto-assigned by rank. See 🎯T3.1, `docs/mcp-triad.md` §7. |

## Gaps and prerequisites for 1.0

- **Tool response format**: Responses are unstructured text. Consider
  returning structured JSON alongside text for programmatic consumers.
- **`bullseye_verify` feedback loop**: The plan-only shape shipped in
  v0.13.0 has no mechanism to fold sawmill results back into a
  structured pass/fail report on the target. Before 1.0 this should
  either be added (so verification can auto-trigger `bullseye_rework`)
  or the decision to stay plan-only should be documented as deliberate.
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
  reset of the settling clock to v0.19.0. v0.20.0 adds the
  `set_aside` status enum value, the `set_aside_reason` schema
  field, and the new `bullseye_set_aside` mutating tool (🎯T18) —
  schema bumps to v3, the new tool joins the surface, the
  `bullseye_put` status parser rejects `set_aside` (a new constraint
  on an existing tool), and the third-party-writer story changes
  with the lockfile relocation (🎯T19). New mutating tool + schema
  bump + new constraint on existing tool — hard reset of the
  settling clock to v0.20.0. v0.21.0 adds two optional parameters
  to `bullseye_rework` for structured sawmill/mnemo payloads
  (🎯T1.5) — purely additive on a previously stable tool, no
  schema bump, no settling-clock reset. v0.22.0 adds a uniform
  envelope-leak guard on every mutating handler (🎯T20) — rejects
  agent-side serialisation bugs at the write boundary so they don't
  silently corrupt YAML. Behavioural addition only; no schema
  change, no surface change visible to well-behaved callers, no
  settling-clock reset. v0.23.0 adds an optional `strategy` block
  to the target schema (🎯T15.2) for the bullseye-native executor
  to consume; closes 🎯T20's test-acceptance gaps; and retires the
  full MCP triad integration (🎯T1, 🎯T1.4, 🎯T1.5) with live
  end-to-end demonstrations across the rework and compaction
  flows. All purely additive — no schema bump, no settling-clock
  reset. v0.24.0 ships the `bullseye sync-priorities` CLI subcommand
  for the Protocol-app integration (🎯T3.1) and tunnel warnings on
  every mutation (🎯T21). New CLI subcommand surface and a new
  output section on four MCP tools, but no schema bump and no
  breaking change to any existing tool — behavioural addition only,
  no settling-clock reset. v0.27.0 removes the `showcase` schema
  field, the `demonstration` schema field, the `showcase` parameter
  on `bullseye_put`, and the `demonstration` parameter on
  `bullseye_retire` (🎯T23). Schema bumps 3→4; v3 files load via
  permissive deserialisation (showcase/observable/demonstration keys
  silently dropped on next save). Schema bump + parameters removed
  from two mutating tools — hard reset of the settling clock to
  v0.27.0.
- **Test coverage for CLI flags**: No tests for --version/--help/--help-agent.

## Out of scope for 1.0

- MCP resource support — waiting on protocol maturity.
