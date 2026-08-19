# Audit Log

Chronological record of audits, releases, documentation passes, and other
maintenance activities. Append-only -- newest entries at the bottom.

## 2026-04-07 -- /audit

- **Commit**: `5bb64bf`
- **Outcome**: 32 findings (2 critical, 5 high, 6 medium, 4 low, 15 info). Report: docs/audit-2026-04-07.md. Pre-open-source audit. Primary blockers: incomplete LICENSE file, deprecated serde_yaml, no CI, no README, UTF-8 panic in truncate().
- **Deferred**:
  - serde_yaml migration (critical -- requires evaluating replacement libraries)
  - agents-guide.md and README.md creation (high -- docs pass needed)
  - CI pipeline setup (high -- depends on GitHub repo creation)
  - NOTICES/THIRD_PARTY attribution file (medium)

## 2026-04-07 -- /release v0.2.0

- **Commit**: `fbcd66c`
- **Outcome**: Released v0.2.0 (darwin-arm64, linux-amd64, linux-arm64). Auto-create on first add, error message fix, convergence terminology removed from user-facing surfaces. Homebrew formula updated.

## 2026-04-08 -- /release v0.3.0

- **Commit**: `bb9225b`
- **Outcome**: Released v0.3.0 (darwin-arm64, linux-amd64, linux-arm64). New bullseye_init tool, agent integration CLAUDE.md snippet, CI fix. 🎯T6 (new-user adoption) fully achieved. Homebrew formula updated.

## 2026-04-08 -- /release v0.4.0

- **Commit**: `9eebcc3`
- **Outcome**: Released v0.4.0 (darwin-arm64, linux-amd64, linux-arm64). Removed WSJF ranking, collapsed parent/child into depends_on, added bullseye_import tool. Homebrew formula updated.

## 2026-04-09 -- /release v0.5.0

- **Commit**: `055d997`
- **Outcome**: Released v0.5.0 (darwin-arm64, linux-amd64, linux-arm64). Fixed parent-to-depends_on migration direction — edges were inverted in v0.4.0. Homebrew formula updated.

## 2026-04-09 -- /release v0.6.0

- **Commit**: `3c19656`
- **Outcome**: Released v0.6.0 (darwin-arm64, linux-amd64, linux-arm64). Fixed bullseye_import parent-to-depends_on direction inversion (same bug as v0.5.0 data migration, but in the import tool). Homebrew formula updated.

## 2026-04-10 -- /release v0.7.0

- **Commit**: `f63af1d`
- **Outcome**: Released v0.7.0 (darwin-arm64, linux-amd64, linux-arm64). Two new tools: bullseye_startup_context (session start enrichment) and bullseye_portfolio (cross-repo target discovery). Import parser fix for code fences and em dashes. Targets achieved: 🎯T4 (dynamic session startup context), 🎯T2.1 (cross-repo discovery). Homebrew formula updated.

## 2026-04-11 -- /release v0.8.0

- **Commit**: `afd2ae5`
- **Outcome**: Released v0.8.0 (darwin-arm64, linux-amd64, linux-arm64). Breaking: retired the `gates` schema field in favour of single-edge-type `depends_on` (legacy files self-migrate on load); merged `bullseye_add` and `bullseye_update` into a unified `bullseye_assert` upsert tool with `blocks` sugar for upstream-declared dependencies and support for explicit sub-target IDs at creation. Settling clock reset for 1.0 eligibility. Homebrew formula updated.

## 2026-04-11 -- /build-perf-audit

- **Commit**: `1058196` (PR #9)
- **Outcome**: Audited the cargo build. One actionable finding: neither `ci.yml` nor `release.yml` had a cargo build cache, so every CI run rebuilt the full dependency graph. Added `Swatinem/rust-cache@v2` to both workflows. Measured on back-to-back PR runs: total CI wall time 53s → 22s (-31s, 58% faster); clippy and test compile each drop from ~17-19s to ~4s. Local dev builds were already excellent (8.3s clean / 0.04s no-op / 0.46s single-file edit) and untouched. Report: docs/build-perf-2026-04-11.md.

## 2026-04-11 -- /release v0.9.0

- **Commit**: `4006efd`
- **Outcome**: Released v0.9.0 (darwin-arm64, linux-amd64, linux-arm64). Three user-visible additions: `bullseye_summary` gets an optional `momentum` parameter for caller-provided recency/frequency scaling of WSJF ranking (🎯T1.2, #11); `schema_version` field in `targets.yaml` with upgrade-prompt enforcement (#13); `bullseye_startup_context` and `bullseye_portfolio` no longer silently drop repos on broken targets files (#12, #14) — parse/IO errors degrade gracefully, schema-version mismatches are surfaced prominently. Internal: `store::load` returns a typed `LoadError` enum; `portfolio::discover_repos` returns `PortfolioScan { repos, warnings }`. No MCP tool surface removals. Settling clock unchanged from v0.8.0. Homebrew formula updated.
- **Known broken**: `bullseye_summary.momentum` used `Option<BTreeMap<String, f64>>`, which the rust-mcp-sdk `JsonSchema` derive emits as `"type": "unknown"` — rejected by the Anthropic API on every tool-list submission. v0.9.0 is unusable; fixed in v0.10.0.

## 2026-04-11 -- /release v0.10.0

- **Commit**: `f6575d9`
- **Outcome**: Released v0.10.0 (darwin-arm64, linux-amd64, linux-arm64). Hotfix for the broken v0.9.0 `bullseye_summary.momentum` schema. Reshaped the parameter from a keyed map to a list of `{id, multiplier}` entries — JSON arrays of scalar-field objects always emit valid Draft 2020-12 schema. Added `every_tool_emits_valid_json_schema` regression test that scans all tool schemas for forbidden `type` values; would have caught v0.9.0 before release. No other functional changes. Users on v0.9.0 must upgrade. Homebrew formula updated.

## 2026-04-11 -- /release v0.11.0

- **Commit**: `7c7cef1`
- **Outcome**: Released v0.11.0 (darwin-arm64, linux-amd64, linux-arm64). Two structural changes via #19: WSJF ranking purged from `bullseye_summary` (it had quietly crept back in v0.7.0 after being officially removed in v0.4.0); new `bullseye_convergence` tool that collapses the old `/cv` worker's multi-round-trip pattern into a single stateless tool call — runs `make bullseye` / `mk bullseye` for standing invariants, scans git for unreleased fixes, emits the target summary with inline frontier details, and computes a deterministic next-action recommendation. Missing-hook case degrades gracefully with embedded setup instructions rather than erroring. Frontier is now ordered by `focus = value × momentum` directly; `top_n` parameter dropped. Project dogfoods its own convergence via a new `Makefile` with a `bullseye:` rule. 92 tests passing (+8). Homebrew formula updated.

## 2026-04-11 -- /release v0.12.0

- **Commit**: `de7d7cb`
- **Outcome**: Released v0.12.0 (darwin-arm64, linux-amd64, linux-arm64). Single-focus release via #22: renamed `bullseye_assert` → `bullseye_put`. The tool's semantics haven't changed (still an upsert), but "assert" carried the wrong connotation in programming contexts — "put" matches the REST verb and the actual behaviour. Breaking at the MCP tool surface; parameters and response shape unchanged. Skills sweep in `marcelocantos/skills` commit `266c2d0` updates `/target`, `/wrap`, `/stash`, `/cv`, and the global CLAUDE.md convergence directives in parallel. Also adds `.claude/settings.local.json` to `.gitignore`. 92 tests unchanged. Homebrew formula updated.

## 2026-04-12 -- /release v0.13.0

- **Commit**: `88d6645`
- **Outcome**: Released v0.13.0 (darwin-arm64, linux-amd64, linux-arm64). Four-target release: 🎯T1.1 adds executable acceptance checks via sawmill (new `checks` schema field, new `bullseye_verify` tool emitting a plan-only sawmill invocation map — preserves cross-server constraint, #25); 🎯T2.2 adds `cross_depends`/`cross_enables` advisory edges surfaced in `bullseye_portfolio` with a binary enabler boost (#26); 🎯T7 formalises the phase-boundary hypothesis — repo-level frontier ordering replaced with distance-to-nearest-observable-checkpoint + unblocking fanout, `bullseye_tunnels` generalised from verify-reachability to observable-reachability, new `observable: true` schema flag, per-target `value`/`cost` documented as portfolio-scope inputs only, `bullseye_convergence` emits a tunnel reshape recommendation when the top frontier candidate has no observable reachable (#27); 🎯T8 makes `bullseye_put` refuse content patches on achieved targets (name/acceptance/context/value/cost/tags/depends_on/verifies/observable edits now require an explicit re-open, `blocks: [T]` into achieved T also rejected, atomic reopen+edit in a single call allowed, retirement path unchanged, #28). Settling clock for 1.0 eligibility restarts from v0.13.0 — the tunnels semantics shift and the `bullseye_put` input-surface narrowing are behavioural breaks. 116 tests (28 unit + 88 integration), up from 92 at v0.12.0. Documentation sweep: README tool table + key concepts, agents-guide full schema example + tool docs + phase-boundary section, STABILITY.md interaction surface catalogue updated in a single pass. Homebrew formula updated.

## 2026-04-12 -- /release v0.14.0

- **Commit**: `184eda7`
- **Outcome**: Released v0.14.0 (darwin-arm64, linux-amd64, linux-arm64). 🎯T10: renamed `targets.yaml` → `bullseye.yaml`, moved from `docs/` to repo root, deleted markdown rendering entirely (`render.rs`, `bullseye_render` tool, `discover_markdown`, auto-render-on-save). Net -618 lines. Fixed mk stderr pattern — was matching "no recipe to make" (plan9 guess); actual marcelocantos/mk phrasing is "no rule to build". Added handler-level regression test for root-level convergence. 85 tests (35 unit + 50 integration). Settling clock restarts — tool surface break (`bullseye_render` removed) and file format break (`targets.yaml` → `bullseye.yaml`). Homebrew formula updated.

## 2026-04-15 -- /release v0.15.0

- **Commit**: `cc7920f`
- **Outcome**: Released v0.15.0 (darwin-arm64, linux-amd64, linux-arm64). Headline: 🎯T12 adds external storage mode for corporate adoption where `bullseye.yaml` can't be committed into managed repos. Machine-wide config at `~/.config/bullseye/config.yaml` selects between `in_repo` (original walk-up-from-cwd behaviour) and `external` (shadow tree under `~/.local/share/bullseye/` mirroring absolute cwd paths, purely path-driven — no git-remote or host/org/repo assumptions). New `bullseye_configure` MCP tool records the one-time choice; until it is called, every other tool returns a structured actionable error containing the locked first-run prompt (wording pinned in `config::FIRST_RUN_PROMPT` as a single source of truth). Invariant enforced across all tool entry points: success-with-config or explicit actionable error — no silent fallback. Elicitation-based UX fast path forked into 🎯T12.1 follow-up (client support across opencode / Roo Code / Cline / VS Code extension / Claude Code is uneven; error-message path is the cross-client contract). Also ships 🎯T2.3 portfolio-level WSJF ranking with cross-repo enabler-boost propagation (a4dd906) and 🎯T9 parallelised Makefile convention documented in the agents guide. 130 tests (40 unit + 90 integration), up from 116 at v0.14.0. Settling clock restarts — tool surface addition (`bullseye_configure`) plus a mandatory first-run step every caller now encounters. Homebrew formula updated.

## 2026-04-15 -- /release v0.16.0

- **Commit**: `a337ae6`
- **Outcome**: Released v0.16.0 (darwin-arm64, linux-amd64, linux-arm64). Redesign of 🎯T12: the v0.15.0 machine-wide `~/.config/bullseye/config.yaml` and `bullseye_configure` tool are **gone**. Storage location is now a per-repo property, encoded by where `bullseye.yaml` lives on disk. Discovery uses a new `discover_anywhere(cwd)` that probes in-repo walk-up first and the shadow-tree walk-up under `~/.local/share/bullseye/` second — in-repo wins on collision. `bullseye_init` and `bullseye_import` grow a required `location` parameter (`in_repo` or `external`); called without it they return the locked `config::LOCATION_PROMPT` so agents can relay the question. `bullseye_put` loses its auto-create fallback and redirects to `bullseye_init` on a bare repo. `config` module shrinks from 300+ lines to <120 — just `Location`, `external_root`, `expand_tilde`, `LOCATION_PROMPT`, and the shadow-root test override. Why the reversal: v0.15.0 traded one machine-wide setting for cross-machine sync burden; per-repo resolution is purely filesystem-driven, nothing to sync, nothing to configure, one prompt per repo instead of one per machine. 127 tests (36 unit + 91 integration). Users upgrading from v0.15.0 can delete `~/.config/bullseye/config.yaml` — the file is ignored. Settling clock restarts — tool surface change (`bullseye_configure` removed, `location` required on init and import) plus behavioural change (no auto-create on `put`). Homebrew formula updated.

## 2026-04-18 -- /release v0.17.0

- **Commit**: `04e698b`
- **Outcome**: Released v0.17.0 (darwin-arm64, linux-amd64, linux-arm64). Headline: 🎯T11 makes `bullseye_put` `value`/`cost` optional on create. Both default to `0.0` (the "not set at repo scope" sentinel) and the validator accepts `>= 0` instead of `> 0`. They remain portfolio-scope inputs only — never consumed by repo-level ordering — so omitting them is appropriate inside a single repo. Eliminates a consistent miscalibration source where agents invented Fibonacci numbers that were immediately ignored. Also includes 🎯T13 perf work (mtime-keyed parse cache in `store::load()` to avoid redundant YAML parses on hot paths) and a clippy regression fix for Rust 1.95 (`recent_achieved` sort uses `sort_by_key` with `std::cmp::Reverse`; test helper uses `writeln!`). Doc sweep updates the agent guide and STABILITY.md to reflect the new `bullseye_put` contract. STABILITY.md settling-threshold note corrected — was stale at v0.14.0; reset to v0.16.0 for the storage redesign, with v0.17.0 noted as additive. Homebrew formula updated.

## 2026-04-20 -- /release v0.18.0

- **Commit**: `008230b`
- **Outcome**: Released v0.18.0 (darwin-arm64, linux-amd64, linux-arm64). Headline: 🎯T17 makes bullseye tolerate concurrent writers on `bullseye.yaml`. Every mutation (`bullseye_put`, `bullseye_retire`, `bullseye_rework`, `bullseye_import`) now acquires an exclusive advisory flock (`flock(2)` on POSIX, `LockFileEx` on Windows, via `fs2`) on a sibling `bullseye.yaml.lock` sentinel — a stable anchor that doesn't move when the yaml is atomically renamed on write. Inside the lock, mutations re-read from disk bypassing the parse cache, apply the mutation, CAS-check `(mtime, len)` bracketing the read-modify-write window to catch non-flock-honouring writers (editors, quick scripts), and write back atomically via tempfile + fsync + rename. Lock wait bounded at 5s with a structured timeout error naming the contended lockfile; conflict errors likewise structured. Discovered 2026-04-20 in spyder/bullseye.yaml — two concurrent Claude Code sessions clobbered each other's target mutations. Regression test (`concurrent_mutations_do_not_lose_updates`) spawns 4 threads × 10 iterations (40 concurrent writes per run); fails deterministically without the lock, passes with it. README gains a "Concurrency protocol" section documenting the lockfile convention for third-party tools. New dependencies: `fs2` (0.4.3) for cross-platform advisory locks, `tempfile` (3) promoted from dev-dep for atomic writes. Settling clock noted but not reset — the lockfile protocol is additive for bullseye's own tools and only tightens expectations on third-party editors. Homebrew formula updated.

## 2026-04-24 -- /release v0.19.0

- **Commit**: `3caf784`
- **Outcome**: Released v0.19.0 (darwin-arm64, linux-amd64, linux-arm64). Headline: 🎯T14 renames the work-target `observable: true` flag to `showcase: true` and adds an enforced retirement obligation. `bullseye_retire` now refuses to retire any target carrying `showcase: true` unless the caller passes a non-empty `demonstration` argument describing what was actually shown to the user; whitespace-only strings are rejected; the demonstration is persisted on the target as a new `demonstration: Option<String>` field. Schema bumps to `schema_version: 2`; legacy `observable:` keys still deserialise via `#[serde(alias = "observable")]` on the new field, so older `bullseye.yaml` files load cleanly and are rewritten under the new name on the next save (one-shot migration). `bullseye_put`'s parameter is renamed in lockstep. Output strings updated: repo-scope banner reads `min distance-to-checkpoint, then max unblocking fanout`; legend distinguishes `[showcase]` (work target with the demo obligation) from `checkpoint` (any target — verify-kind or showcase — that produces a signal); per-entry annotations `[observable]` → `[showcase]`, `"observable"` distance label → `"checkpoint"`, `"no observable reachable"` → `"no checkpoint reachable"`. Internal predicate and helper renames follow the same theme: `is_observable` → `is_checkpoint`, `observable_distance` → `checkpoint_distance`, `OBSERVABLE_REACH_LIMIT` → `CHECKPOINT_REACH_LIMIT`. Discovered while working on 🎯T10 in squz/ge — tiltbuggy connected to ged but the user couldn't see anything until the player was actually launched, which required a follow-up prompt; the `observable` intent was there but the obligation wasn't codified. Two new regression tests pin the behaviour: `legacy_observable_field_still_deserialises` exercises the alias migration end-to-end; `retire_showcase_target_requires_demonstration` exercises the rejection path (missing/whitespace), the acceptance path (real demo string), and the persistence of the demonstration on the retired target. README, agents-guide, mcp-triad, and STABILITY all updated. Settling clock hard-resets to v0.19.0 — field rename + new required parameter on a mutating tool. Homebrew formula updated.


## 2026-04-25 -- /release v0.20.0

- **Commit**: `db50ad3`
- **Outcome**: Released v0.20.0 (darwin-arm64, linux-amd64, linux-arm64). Two headline features. **🎯T18: `set_aside` status with required rationale.** New terminal disposition for targets the user decides not to pursue (parked / deferred / wont_fix) — distinct from `achieved`: the target is removed from the active set and unblocks its dependents the same way an achieved target does, but renders in a separate `## Set aside` group in `bullseye_summary` so decisions-not-to-do don't inflate the achievement record. New tool `bullseye_set_aside(cwd, id, reason)` is the canonical transition path; the free-text `reason` is required and non-empty; `bullseye_validate` flags missing reasons and stale leftover reasons on non-set-aside statuses; `bullseye_put` rejects `status: set_aside` and routes callers to the dedicated tool so the rationale is always recorded. Schema bumps to `schema_version: 3`. Surfaced during convergence work where stale `cross_depends` notes and decisions-not-to-pursue both polluted the `context:` field — the status model needed a terminal-but-unachieved disposition to stop that pattern from compounding. **🎯T19: lockfile relocated outside the project directory.** Bullseye no longer writes `bullseye.yaml.lock` next to the YAML; lockfiles now live under `std::env::temp_dir()/bullseye/locks/`, named by the parent directory's hex `(dev_t, ino_t)`. Robust against atomic-rename writes (which change the YAML's inode but leave the parent dir's inode intact), repo directory renames (which keep the dir's inode), and symlinked access paths (canonicalised before stat). Auto-clears on reboot via temp-dir semantics — no cleanup machinery to write. Surfaced during 🎯T18 work as a cross-cutting hygiene issue: every repo using bullseye was getting polluted with `bullseye.yaml.lock` artefacts. 10 new regression tests (5 set-aside, 5 lock-keying); existing 139 tests unchanged. README "Concurrent edits & locking" section rewritten for the new model; agents-guide gains a `bullseye_set_aside` reference; STABILITY catalogue updated for both features. Settling clock hard-resets to v0.20.0 — new mutating tool, schema bump 2→3, new constraint on existing tool. Homebrew formula updated.

## 2026-04-26 -- /release v0.21.0

- **Commit**: `07f5e98`
- **Outcome**: Released v0.21.0 (darwin-arm64, linux-amd64, linux-arm64). Single headline feature. **🎯T1.5: structured rework payloads.** `bullseye_rework` accepts two optional JSON-encoded payload parameters — `sawmill_failure` (structured failure output from sawmill's `check_conventions` / `check_invariants` / `query`, sawmill T24's deliverable) and `mnemo_history` (prior rework attempts from mnemo for the same target ID). Both are validated as JSON, pretty-printed, and persisted into the rework target's `context` as labelled fenced ```json blocks under `## Sawmill failure` and `## Prior attempts (mnemo)` headers, alongside the existing free-form `diagnosis` prose. Composition stays at the agent layer (MCP servers can't call each other directly); bullseye persists opaquely. Implementation note: parameters are `Option<String>` rather than `Option<serde_json::Value>` because the rust-mcp-sdk JsonSchema derive falls back to `type: "unknown"` for arbitrary JSON, which the Anthropic API rejects on tool-list submission. Sawmill side is end-to-end usable today; the mnemo half is forward-compatible — mnemo currently lacks a rework-aware query, which is the remaining external gap. Additive change on a previously stable tool — existing callers passing only `diagnosis` see no behavioural difference. 9 new tests cover composition logic and end-to-end persistence + invalid-payload rejection. STABILITY catalogue updated; `bullseye_rework` row drops from `Stable` to `Needs review` while the JSON-string shape settles. Settling clock unchanged at v0.20.0 — purely additive, no schema bump. Homebrew formula updated.

## 2026-04-26 -- /release v0.22.0

- **Commit**: `6375d7c`
- **Outcome**: Released v0.22.0 (darwin-arm64, linux-amd64, linux-arm64). Single headline feature. **🎯T20: tool-call envelope leak guard on every mutating handler.** All mutating tools (`bullseye_put`, `bullseye_retire`, `bullseye_set_aside`, `bullseye_rework`, `bullseye_import`) now reject any caller-controlled string containing a leaked Claude tool-call XML envelope marker — `<invoke `, `</invoke>`, `<parameter `, or `</parameter>`. On detection the tool returns an actionable error naming the field and the marker (e.g. `context contains tool-call envelope marker '</invoke>' — looks like XML tool-call syntax leaked into the parameter value`), and the file is not mutated. Validation fires at the write boundary only — `store::load` continues to load corrupted files unchanged so an operator can repair them. Generic angle-bracket prose passes; only the four protocol-specific markers are exact-matched. Filed after observing malformed YAML in another repo (the user's stock car racing project) where an agent serialised a `bullseye_put` call as XML, the harness's wrapper-stripping was incomplete, and `</invoke>` plus surrounding closing tags ended up persisted into a target's `context:` field as literal YAML content — bullseye stored it faithfully, leaving a corrupted file an unrelated future agent had to debug. The same failure mode hit me earlier in this session when I formatted `acceptance` as an XML envelope rather than a JSON array, but bullseye correctly rejected it because acceptance is typed as a sequence; the corruption only lands silently when leakage hits a free-text string field. Root cause is upstream of bullseye (older Claude `<invoke name="..."><parameter name="...">…</parameter></invoke>` tool-call syntax bleeds through under multi-line content pressure), but bullseye should not be a faithful storer of agent serialisation bugs. Implementation: a `check_no_envelope_leak(field, value)` helper plus a `check_target_no_envelope_leaks(id, target)` walker for the bulk-import path, called from every mutating handler before the locked read-modify-write block. 8 new tests cover marker detection per field, false-positive avoidance (`<context>`, `</context>`, `a < b`, `a > b` all pass), per-handler rejection (put context, put acceptance index, set_aside reason, rework diagnosis, rework sawmill payload), file-unchanged-on-rejection, and the cross_depends.note walker path. STABILITY catalogue updated; no tool-table edits needed since the validator is uniform and doesn't change any signature. Settling clock unchanged at v0.20.0 — behavioural addition, no schema change, no surface change visible to well-behaved callers. Homebrew formula updated.

## 2026-04-26 -- /release v0.23.0

- **Commit**: `ce39301`
- **Outcome**: Released v0.23.0 (darwin-arm64, linux-amd64, linux-arm64). Three threads converge in this release. **🎯T15.2: target-level `strategy` block in schema.** A new optional `strategy: Strategy { command, trigger, timeout?, retry? }` field on `Target` (with `RetryPolicy { max_attempts?, backoff? }`) lets a target declare how it converges mechanically alongside its acceptance criteria. `bullseye_validate` rejects empty `command` or `trigger` after trim; bullseye persists and validates but does not execute. The forthcoming bullseye-native executor (🎯T15) consumes it. Targets without a strategy round-trip unchanged; schema_version stays at 3 (purely additive optional field). 6 new tests cover parse, round-trip, both rejection cases, valid pass, and fixture forward-compat. **🎯T20 test-acceptance closure.** Added two integration tests (`import_rejects_envelope_markers_in_parsed_markdown` exercises `handle_import` end-to-end with corrupted markdown and pins no-write-on-rejection; `store_load_still_loads_corrupted_files` pins the write-boundary invariant so a pre-corrupted file remains repairable in place) and promoted `handle_import` to `pub` for symmetry with the other tested handlers. **MCP triad integration achieved end-to-end (🎯T1, 🎯T1.4, 🎯T1.5).** No bullseye-side surface change in v0.23.0, but the contract is load-bearing across the upstream pieces that landed this release window: mnemo v0.28.0's compactor reads `bullseye.yaml` directly and surfaces structured `targets_active` / `targets_progressed` / `targets_next` fields in every compaction payload (🎯T1.4 demo via the round-trip test that pins the produced shape); mnemo PR #63 ships the `mnemo_rework_history` query producing the JSON payload `bullseye_rework`'s `mnemo_history` parameter accepts (🎯T1.5 demo via a live rework cycle in an isolated repo carrying both `## Sawmill failure` and `## Prior attempts (mnemo)` fenced JSON blocks alongside the diagnosis prose). The string-encoded JSON parameter shape for `bullseye_rework` (deliberate workaround for the Anthropic API rejecting `serde_json::Value`) is now load-bearing across the live integration. Also includes graph: split `validate()` into `validate_blocking` + `validate_warnings` (#71) — internal refactor separating ID-format and similar stylistic warnings from structural blocking errors; the combined `validate()` remains for callers (e.g., `bullseye_validate`) that want the union. Five additional administrative retirements: 🎯T14, 🎯T18, 🎯T19 (already-shipped features whose targets just hadn't been retired), and the parent 🎯T1 once all sub-targets achieved. Active target count down 12 → 4 over the session. STABILITY catalogue updated for `strategy`; settling clock unchanged at v0.20.0 — purely additive, no schema bump. Homebrew formula updated.

## 2026-04-28 -- /release v0.25.0

- **Commit**: `81f48aa`
- **Outcome**: Released v0.25.0. Single headline feature. **🎯T22: bullseye self-commits a dirty `bullseye.yaml`.** Every mutating tool call (`bullseye_put`, `bullseye_retire`, `bullseye_set_aside`, `bullseye_rework`, `bullseye_init`, `bullseye_import`) and the start of `bullseye_convergence` now fold a dirty `bullseye.yaml` into git automatically. Decision rule: if the most recent commit on `HEAD` is unpushed and its set of changed files is exactly `{bullseye.yaml}`, fold the new state via `git commit --amend --no-edit -- bullseye.yaml` (existing message preserved); otherwise create a fresh `Update bullseye.yaml` commit containing only `bullseye.yaml` (other staged changes left in the index untouched). Best-effort: outside a git repo (shadow-tree storage, tempdir tests) and on git failures the auto-commit is a silent no-op and the file stays dirty for manual resolution. Implementation lives in a new internal `git_commit` module with six unit tests covering empty repo, clean yaml, amend-eligible (unpushed yaml-only HEAD), amend-ineligible (mixed-files HEAD), unrelated staged changes preserved, and pushed-HEAD fallback. Symlink-safe — canonicalises the input path before stripping the repo top-level prefix so macOS `/var/folders/...` → `/private/var/...` resolution works in tempdirs. Filed after observing the `/cv` skill's special-case dirty-`bullseye.yaml` workaround: every mutation already knew it just dirtied the file, so bullseye is the right layer to fold the change into git. Moving the policy server-side keeps the skill thin and makes the behaviour consistent across any caller (not just /cv). The companion `/cv` skill change (removing the special case) lands in the same release window. STABILITY catalogue updated; no public API additions (the `git_commit` module is internal). Settling clock unchanged at v0.20.0 — purely additive behavioural change, no schema bump, no surface change visible to existing callers. Homebrew formula updated.

## 2026-04-27 -- /release v0.24.0

- **Commit**: `3152820`
- **Outcome**: Released v0.24.0 (darwin-arm64, linux-amd64, linux-arm64). Two headline features. **🎯T3.1: `bullseye sync-priorities` CLI subcommand.** New cron-targetable subcommand scans the workspace via the existing portfolio engine and upserts each frontier target into a SQLite `targets_priorities` table (`id` PRIMARY KEY = `"{repo}/{target_id}"`, `repo`, `name`, `priority` REAL = per-target WSJF, `context`, `horizon`, `updated_at`); stale rows whose targets are no longer on any repo's frontier are deleted in the same transaction so the table is a clean projection of the current frontier. Flags `--db PATH` (default `$BULLSEYE_DATA_DIR/priorities.db`), `--root PATH` (default `~/work`), `--horizon STR` (default `today`), `--max-depth N` (default 5). Designed for periodic invocation from cron or a daemon hook; the downstream sync chain (sqlpipe-over-pigeon → Protocol app's `protocol.db` → Today-page Focus section) is tracked separately under 🎯T3.2. Adds `rusqlite = "0.39.0"` with the `bundled` feature so SQLite compiles from source — no host library required, matches the global C/C++ no-Homebrew-linkage policy. New public Rust API: `priorities::{open, sync, SyncCounts, SyncArgs, run_sync, parse_sync_args, default_db_path}`. The `portfolio::PortfolioFrontierTarget` struct gains a public `context: Option<String>` field threaded through `summarize_repo_raw` so the writer doesn't reload each repo's targets file. 8 new unit tests cover schema init, transactional upsert + delete-stale, empty-scan clearing, repo-scoped IDs, and arg parsing. Smoke-tested against the live workspace: 323 frontier targets synced cleanly, idempotent on second run. **🎯T21: tunnel warnings on every mutation.** After every successful `bullseye_put` / `bullseye_retire` / `bullseye_set_aside` the post-mutation graph is re-run through tunnel detection (existing `graph::tunnels` predicate at max_depth=2) and a `## ⚠ Tunnel warnings` section is appended to the response listing each orphaned work target with its distance-to-checkpoint result (`no checkpoint reachable` or `nearest checkpoint at N hops`) and up to four ranked candidate checkpoint locations (Convergence → Root → Self → OnPath). `bullseye_validate` surfaces the same warnings. Mutations are never rejected — the warning is informational, so the agent reads it and flips `showcase: true` on a candidate in a follow-up `bullseye_put`. Filed after recurring sessions where every chain in the live graph turned out to be a tunnel and `bullseye_convergence` refused to recommend any frontier target, with the original context for which node should have been the showcase already lost. Hard rejection was considered and dismissed — empirically the fix is almost always one boolean toggle, and forcing a two-step ceremony for every such edit produces no benefit; the `bullseye_convergence`-side "Blocked: tunnel" recommendation remains as a last-resort backstop for the rare case where the warning gets ignored. New public Rust API: `graph::{format_tunnel_warnings, suggest_checkpoint_candidates, CheckpointCandidate, CandidateReason}`. 5 new tests cover the candidate-ranking heuristic and the rendered warning output. STABILITY catalogue updated for both features (new fluid CLI subcommand row, new fluid output-format entries for the warning section and the SQLite schema, `bullseye_validate` drops to "needs review"). "Out of scope for 1.0 — Protocol app sync" line removed from STABILITY.md, since 🎯T3.1 ships and 🎯T3.2 is on the active frontier. Settling clock unchanged at v0.20.0 — purely additive surface, no schema bump, no breaking change to any existing tool. Homebrew formula updated.

## 2026-05-02 -- /release v0.26.0

- **Commit**: `c2cf85e`
- **Outcome**: Released v0.26.0. Single fix. **`bullseye_summary` / `bullseye_convergence` no longer suppress the inline `## Frontier` section on advisory-warning-only validate runs.** The summary renderer now gates the frontier block on `validate_blocking` (structural errors) instead of the warning-inclusive `validate`, matching the existing behaviour of `frontier`, `convergence`, and `portfolio` — and matching `validate()`'s own doc-comment guidance that downstream tools needing a hard gate should call `validate_blocking`. Cosmetic warnings (today, the non-conforming-target-ID check on IDs like `T34a`) used to collapse the convergence response into `## Active targets by group` + `## Validation errors`, hiding the per-frontier-target acceptance/context that the `/cv` skill consumes for fan-out and forcing the agent to round-trip `bullseye_get` on every tied frontier target. Surfaced when running `/cv` on pigeon, which carries four advisory ID-format warnings on `T34a`/`T34b.1`/`T34b.2`/`T34c.1`. Warnings continue to render separately via `convergence.rs`'s dedicated `## Validation warnings` section. STABILITY catalogue updated. New regression test `summary_with_only_warnings_still_renders_frontier`. No public Rust API change; settling clock unchanged at v0.20.0. Homebrew formula updated.

## 2026-05-13 -- /release v0.27.0

- **Commit**: `2135d58`
- **Outcome**: Released v0.27.0 (darwin-arm64, linux-amd64, linux-arm64). Two headline changes. **🎯T23: the `showcase` construct is removed** end-to-end (see the per-target entry below) — schema bumps `3 → 4`, `bullseye_put` loses `showcase`, `bullseye_retire` loses `demonstration`, `is_checkpoint` is verify-kind-only. Pre-v4 yaml files load unchanged; the retired keys are silently dropped on parse and stripped on next save. **🎯T24: mutating operations refuse submodule replicas and detached HEAD.** All mutating handlers (`bullseye_put`, `bullseye_retire`, `bullseye_set_aside`, `bullseye_rework`, `bullseye_init`, `bullseye_import`) run `git rev-parse --show-superproject-working-tree` and `git symbolic-ref -q HEAD` against the repo containing the discovered `bullseye.yaml` before any read or write; submodule replicas and detached HEAD checkouts are refused with an actionable error (naming the superproject path / suggesting the canonical clone location for the submodule case; explaining that auto-committing would land on a dangling local branch for detached HEAD). Read-only operations (`list`, `frontier`, `get`, `summary`, `graph`, `validate`, etc.) are unaffected. Filed after the multimaze2 incident on 2026-05-09 where a target filed inside a `multimaze2/ge` submodule auto-committed to a local-only branch one commit past `v0.9.0`; the canonical clone showed nothing on grep, and the auto-commit hid the modification from the parent's `git status` until a future agent went looking. New `repo_guard` module exposes typed `Submodule { repo_root, superproject, suggested_path }` and `DetachedHead { repo_root, current_sha }` variants; canonical-path heuristic parses `git config --get remote.origin.url` (SSH and HTTPS forms) and recommends `~/work/<host>/<owner>/<repo>/`. Three new integration tests (submodule replica via `file://` URL, detached HEAD, read-only-still-works-in-submodule) plus six unit tests in `repo_guard::tests`. STABILITY catalogue records both — settling clock reset to v0.27.0 by the schema bump alone; the 🎯T24 gate is a new runtime error class only (tool surface unchanged). Homebrew formula updated.

## 2026-05-10 -- 🎯T23: showcase construct removed

- **Outcome**: The `showcase` construct is removed from bullseye end-to-end. Schema bumps to `schema_version: 4`. The boolean `showcase` field on `Target` (and its legacy `observable` alias), the `demonstration` field on `Target`, the `showcase` parameter on `bullseye_put`, and the `demonstration` parameter on `bullseye_retire` are all gone. `is_checkpoint` returns true only for verify-kind targets now; tunnel-candidate suggestions and the convergence reshape recommendation read "add a verify target above this candidate" rather than "flip showcase: true". Pre-v4 yaml files load unchanged — `showcase`, `demonstration`, and the legacy `observable` alias are silently dropped by serde on parse and stripped on the next save (one-shot migration; equivalent to how `observable` → `showcase` migrated in v0.19.0). Repo-side bullseye.yaml stripped of all `showcase: true` / `demonstration: ...` lines on active and achieved targets. Six tests retired (`legacy_observable_field_still_deserialises`, `retire_showcase_target_requires_demonstration`, `retire_non_showcase_target_does_not_require_demonstration`, `showcase_field_yaml_roundtrip`, `showcase_flag_changes_frontier_order`, `tunnels_treats_showcase_work_as_checkpoint`); replaced with `legacy_showcase_demonstration_keys_load_and_strip_on_save` to pin the migration behaviour. README, STABILITY, agents-guide, and mcp-triad updated to describe the new model; STABILITY records the v4 schema bump and the removed parameters. Filed because the obligation half of the flag never carried its weight in practice — agents skipped the demonstration step the field was supposed to force, while the field added schema, doc, and tool-parameter surface that callers had to reason about. Hard reset of the settling clock to the next release tag — schema bump, removed parameters on two mutating tools.

## 2026-05-17 -- 🎯T25: schema v5 uniform-node model

- **Outcome**: The verify-kind / work-kind distinction is removed from bullseye end-to-end. Schema bumps to `schema_version: 5`. The `kind` field on `Target` (and its `work` / `verify` enum values), `verifies` edges, `rework` edges, `retry_budget`, and `retries` are all gone. Pre-v5 yaml files load unchanged — `kind`, `verifies`, `rework`, `retry_budget`, and `retries` keys are silently dropped by serde on parse and stripped on the next save (one-shot migration, same shape as the v3→v4 showcase removal). Every target is now structurally uniform: a name, acceptance criteria, `depends_on` edges, and optional metadata. The acceptance criteria *are* the verification contract. Whether the pass signal comes from CI, a human review, a smoke test, or a design walkthrough is free text on the `acceptance` field — not encoded as a node type.

- **MCP tools removed**: `bullseye_rework` and `bullseye_tunnels` are gone. The rework-cycle pattern (verify target → rework edge → work target → retry budget) no longer exists; the concepts that made it meaningful (`kind: verify`, `verifies`, `rework` back-edges) are removed. `bullseye_tunnels` has no meaning in the uniform-node model — tunnels were chains of work targets with no verify-kind checkpoint reachable; with no verify-kind distinction, the concept dissolves.

- **MCP tool added**: `bullseye_revert(cwd, id, reason)`. When a regression or new information shows a retired target was not as achieved as it looked, `bullseye_revert` moves it from achieved back to converging, clears the achieved date, and appends `Reverted YYYY-MM-DD: <reason>` to the target's context. Achievement-only — to resume a set-aside target, use `bullseye_put` with `status: identified`.

- **Rework → Revert reframing**: The rework pattern was a structured retry loop (verify-kind target detects failure → rework edge → re-enter work target → increment retry counter). The replacement is conceptually simpler: if something was believed achieved and turns out not to be, revert the achievement with a reason and resume work. No retry budgets, no automated loop, no back-edges in the graph. The agent decides when to stop retrying.

- **Frontier ordering simplified**: The repo-level frontier was previously ordered `(ascending distance-to-nearest-checkpoint, descending unblocking fanout, ascending ID)`. The checkpoint-distance term is gone — there are no longer any checkpoints as a structural concept. The ordering is now `(descending unblocking fanout, ascending ID)`. The `bullseye_summary` frontier annotation changes from `dist=N, fanout=M` to `fanout=M`.

- **Checkpoint / tunnel apparatus removed**: `is_checkpoint`, `checkpoint_distance`, `format_tunnel_warnings`, `suggest_checkpoint_candidates`, the tunnel-warning auto-append on mutations, the `## ⚠ Tunnel warnings` section in `bullseye_validate` output, and the "Blocked: tunnel, reshape the graph" recommendation in `bullseye_convergence` are all gone. The graph-shaping discipline those mechanisms tried to enforce moves from the tool to bullseye-the-practice: agents and humans shape the dependency graph based on acceptance criteria content, not structural node-type signals.

- **Design rationale**: The verify-kind construct had the same structural problem as the retired `showcase` flag (🎯T23): it encoded a verification moment (when and how the pass signal is emitted) into a node type, when that is a property of the acceptance criteria, not the node. The presence of a verify-kind node didn't actually change what checks were run — it just gave the agent a hint about which tool to call. Moving that hint into the acceptance prose (free text describing whether CI, human, smoke test, or design review provides the signal) loses no information and removes a node-type distinction that was increasingly generating noise (tunnel warnings, rework-cycle orchestration, checkpoint-distance ordering) while providing little steering value in practice. Bullseye-the-tool stays a minimal substrate (like Make); bullseye-the-practice carries the graph-shaping discipline.

- **Tests**: Tests covering rework cycles (`rework_*`), tunnel detection (`tunnels_*`), verify-kind frontier ordering, and checkpoint-distance behaviour are retired. Replaced with tests covering the `bullseye_revert` tool (revert achieved target, clear achieved date, append revert note, reject non-achieved target), the simplified frontier ordering (fanout-only, stable ID tiebreak), and legacy-key migration (v5 load of files with `kind`, `verifies`, `rework`, `retry_budget`, `retries` keys strips them cleanly on next save).

- **Settling clock**: Hard reset at the next release tag after this PR merges. This is a schema bump with removed tool surface area (`bullseye_rework`, `bullseye_tunnels`), a new tool (`bullseye_revert`), and narrowed parameters on `bullseye_put` (`kind` and `verifies` dropped). The settling clock records the date of the release tag, not the PR merge.

## 2026-07-06 -- /release v0.37.0

- **Outcome**: Released v0.37.0 (darwin-arm64, linux-amd64, linux-arm64). Two linked safety changes close the direct-`bullseye.yaml` editing incident. **🎯T39: Bullseye owns target ID allocation and hierarchy conventions.** `bullseye_put` gains `child_of` so agents can ask for the next child under a parent without choosing the final dotted number; explicit IDs are reserved for intentional placement or patches, and explicit dotted IDs ending in `.0` are rejected both at mutation time and by validation. **🎯T40: Bullseye rejects unsafe control characters before writing YAML.** Mutating handlers now reject non-whitespace C0 controls in caller-controlled strings before entering the locked mutation, naming the field and code point (for example U+0001) while leaving the file unchanged; newline, carriage return, and tab remain allowed. README and the agent guide document the new `bullseye_put` contract, and `CLAUDE.md` now imports the shared `AGENTS.md` instructions to prevent cross-agent drift. Settling clock resets at v0.37.0 — this release adds a `bullseye_put` input and narrows accepted IDs/free text.

## 2026-08-15 -- 🎯T72: ledger commit SHA stability

- **Outcome**: The 🎯T22 auto-commit amend rule is narrowed so a commit
  SHA bullseye has already shown a caller stays reachable. **Decision
  rule as it now stands** (superseding the rule recorded in the v0.25.0
  entry above): if `HEAD` is a commit *this process* created — bullseye
  records, per `(repo top-level, ledger pathspec)`, the SHA it read back
  from `git rev-parse HEAD` immediately after each of its own successful
  ledger commits — and that commit is still unpushed and still touches
  exactly `{bullseye.yaml}`, fold the new state via `git commit --amend
  --no-edit -- bullseye.yaml` (existing message preserved); otherwise
  create a fresh `Update bullseye.yaml` commit containing only
  `bullseye.yaml` (other staged changes left in the index untouched).
  Best-effort semantics are unchanged: outside a git repo, and on git
  failure or timeout, the auto-commit is a silent no-op and the file
  stays dirty. On any failed or timed-out commit the ownership record is
  forgotten, so the next mutation starts a fresh commit rather than
  amending something it can no longer vouch for.

- **Defect being closed**: the old rule decided amend eligibility from
  the changed-file set alone — "unpushed AND `HEAD` touches only
  `bullseye.yaml`" — which is true of *every* agent's ledger commit, not
  just this process's. Observed live by bullseye-po on 2026-08-15, twice
  within one hour: the reflog shows `fcab2fa -> 928e25c -> 12a0e26 ->
  c4e853d`, four amends in under three minutes, orphaning two SHAs that
  bs-t69 and bs-t70 had already cited as evidence in finish reports.
  Both agents had obtained their evidence correctly; it was dead on
  arrival by the time it was read. Ledger *content* was never at risk
  (`store` holds flock + CAS) — this is purely about SHA stability, and
  it matters because a cited SHA is how a reviewer re-checks a claim
  they did not watch happen. Once `git gc` prunes the orphans the
  failure goes silent: the SHA simply stops resolving.

- **Why ownership is tracked in process memory**: "same process" is
  exactly the boundary that makes folding safe, and a marker on disk
  would be shared with the very sibling processes we must not fold into.
  The cost is that consecutive *CLI* invocations each get their own
  commit, since each is its own process. Within one MCP server session —
  the case 🎯T22 was built for, and the one that produces long runs of
  mutations — folding is unchanged. The record is process-global rather
  than thread-local because an MCP server answers tool calls on
  whichever runtime thread is free, and all of them are one agent's
  session.

- **Tests**: new integration oracle `tests/ledger_sha_stability_test.rs`
  drives the real binary so the process boundary under test is a real
  one — `a_sha_from_one_process_survives_another_process_mutation` and
  `every_sha_shown_to_any_process_stays_reachable`, both asserting
  reachability with `git merge-base --is-ancestor` from a second process
  rather than inspecting the changed-file set. New unit tests in
  `src/git_commit.rs`:
  `does_not_amend_a_ledger_commit_this_process_did_not_create` (the
  defect in unit form) and
  `folds_consecutive_mutations_in_one_session_into_one_commit` (🎯T22's
  benefit preserved: five mutations in one session, one commit). The
  pre-existing amend test is renamed
  `amends_when_head_is_this_process_own_unpushed_yaml_commit` and now
  establishes ownership by making the first commit through bullseye
  itself; the mixed-index test's expectation flips from amend to fresh
  commit, since a hand-made `HEAD` is no longer ours to fold into.

- **Docs**: the decision rule is restated in STABILITY.md, in the
  `src/git_commit.rs` module docs, and here, so the written rule and the
  code agree.

## 2026-08-19 -- /release v0.46.0

- **Outcome**: Released v0.46.0 (darwin-arm64, linux-amd64, linux-arm64).
  **🎯T73: mutations do not write yaml-only git commits.** Mutating tools
  write `bullseye.yaml` and leave it dirty. `bullseye_convergence` does
  not `git commit` before invariants. Standing-invariants dirty-tree
  checks ignore the ledger (root and nested in-repo paths). Durability
  is `/commit` (always stage a dirty in-repo ledger) and `/push` (refuse
  if still dirty). The T22 auto-commit rail and T72 own-commit amend
  path are gone (`src/git_commit.rs` deleted). Settling clock resets at
  v0.46.0.
