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

- **Commit**: pending
- **Outcome**: Released v0.9.0 (darwin-arm64, linux-amd64, linux-arm64). Three user-visible additions: `bullseye_summary` gets an optional `momentum` parameter for caller-provided recency/frequency scaling of WSJF ranking (🎯T1.2, #11); `schema_version` field in `targets.yaml` with upgrade-prompt enforcement (#13); `bullseye_startup_context` and `bullseye_portfolio` no longer silently drop repos on broken targets files (#12, #14) — parse/IO errors degrade gracefully, schema-version mismatches are surfaced prominently. Internal: `store::load` returns a typed `LoadError` enum; `portfolio::discover_repos` returns `PortfolioScan { repos, warnings }`. No MCP tool surface removals. Settling clock unchanged from v0.8.0. Homebrew formula updated.
