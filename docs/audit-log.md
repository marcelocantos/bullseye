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
