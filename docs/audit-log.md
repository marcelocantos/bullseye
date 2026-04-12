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

- **Commit**: pending
- **Outcome**: Released v0.14.0 (darwin-arm64, linux-amd64, linux-arm64). 🎯T10: renamed `targets.yaml` → `bullseye.yaml`, moved from `docs/` to repo root, deleted markdown rendering entirely (`render.rs`, `bullseye_render` tool, `discover_markdown`, auto-render-on-save). Net -618 lines. Fixed mk stderr pattern — was matching "no recipe to make" (plan9 guess); actual marcelocantos/mk phrasing is "no rule to build". Added handler-level regression test for root-level convergence. 85 tests (35 unit + 50 integration). Settling clock restarts — tool surface break (`bullseye_render` removed) and file format break (`targets.yaml` → `bullseye.yaml`). Homebrew formula updated.
