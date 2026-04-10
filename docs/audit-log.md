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

- **Commit**: TBD
- **Outcome**: Released v0.7.0 (darwin-arm64, linux-amd64, linux-arm64). Two new tools: bullseye_startup_context (session start enrichment) and bullseye_portfolio (cross-repo target discovery). Import parser fix for code fences and em dashes. Targets achieved: 🎯T4 (dynamic session startup context), 🎯T2.1 (cross-repo discovery). Homebrew formula updated.
