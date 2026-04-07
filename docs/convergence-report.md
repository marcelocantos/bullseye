# Convergence Report

Evaluated: 2026-04-07
Branch: master @ ada7a0f

## Standing invariants

Standing invariants: all green.
- Tests: 44 passed, 0 failed
- CI: clean working tree, no open PRs

## Gap report

### 🎯T6.3 Copy-pasteable CLAUDE.md snippet for target management  [weight 3]
Gap: **not started**
Neither README.md nor agents-guide.md include a copy-pasteable CLAUDE.md snippet. The README covers installation and MCP config but not agent integration. The agents-guide documents tool parameters and workflows but has no CLAUDE.md section for users to paste.

### 🎯T4.1 mnemo_startup_context tool  [weight 2]
Gap: **not started** (status only)
This is a mnemo feature — no code exists in the bullseye repo. Implementation lives in the mnemo codebase.

### 🎯T6.1 bullseye_init tool creates starter targets.yaml  [weight 2]
Gap: **not started** (status only)
No bullseye_init tool exists in the MCP server. bullseye_add auto-creates the file (🎯T6.2, achieved), but there is no dedicated init with skeleton/sample content.

### 🎯T1.2 Momentum-aware ranking via mnemo  [weight 2]
Gap: **not started** (status only)
Ranking is pure WSJF today. No momentum signal integration.

### 🎯T1.4 Target-aware context compaction  [weight 2]
Gap: **not started** (status only)
Depends on mnemo T10. No implementation.

### 🎯T2.1 Cross-repo target discovery  [weight 2]
Gap: **not started** (status only)
No portfolio or cross-repo capability exists.

### 🎯T5.1 Markdown-to-YAML target converter  [weight 2]
Gap: **not started** (status only)
No converter tool exists. Manual conversion was done for this repo.

### 🎯T1.1 Executable acceptance checks via sawmill  [weight 2]
Gap: **not started** (status only)
No checks field in schema, no targets_verify tool.

### 🎯T5 Migration from markdown targets to bullseye  [weight 2]
Gap: **significant** (status only)
This repo runs both systems in parallel. No other repos migrated. No converter tool.

### 🎯T1 MCP triad integration  [weight 2]
Gap: **not started**
No integration between the three MCP servers exists yet. All sub-targets are identified, none converging.

### 🎯T2 Global portfolio view  [weight 2]
Gap: **not started**
All sub-targets identified. Blocked chain: T2.1 -> T2.2 -> T2.3 -> T2.4.

### 🎯T3 Protocol app priority sync  [weight 1]
Gap: **not started** (status only)
Depends on portfolio view (T2.3). Furthest from completion.

## Recommendation

Work on: **🎯T6.3 Copy-pasteable CLAUDE.md snippet for target management**

Reason: Highest effective weight (3) among unblocked leaf targets with the lowest cost (1). Both the markdown ranking system and bullseye agree this is the top frontier target. It is a docs-only change with immediate adoption value -- adding a CLAUDE.md section to README.md and agents-guide.md that users can paste into their projects to activate bullseye integration.

## Suggested action

Add a "## Agent integration" section to `docs/agents-guide.md` containing a copy-pasteable CLAUDE.md snippet. The snippet should instruct agents to use bullseye tools for target management (list, rank, frontier for assessment; add, update, retire for lifecycle). Then reference this section from `README.md`. Keep it standalone -- no dependency on other skills or setup beyond having the MCP server registered.

## Bullseye scorecard

**Ranking**:        +1
**Blocking**:       +1
**Data quality**:   0
**Overall**:        +1
**Markdown rec**:   🎯T6.3 Copy-pasteable CLAUDE.md snippet for target management
**Bullseye rec**:   🎯T6.3 Copy-pasteable CLAUDE.md snippet for target management
**Notes**: Both systems agree on the recommendation. Bullseye's ranking is slightly better -- it correctly separates parents from leaves and presents a clean frontier of 8 actionable targets. The markdown rank.py mixes parents and leaves in the same list, making it harder to see what's actually workable. Blocking analysis is equivalent but bullseye's presentation is cleaner (dedicated blocked section vs inline annotations). Data quality is fine -- all targets have value/cost, all edges are present, no stale fields. The targets were bulk-imported today so there has been no drift yet. Overall +1: bullseye adds value through the frontier concept and cleaner blocked/unblocked split, but the recommendation is the same. The real test will come when targets start moving and drift between the two systems becomes possible.

<!-- convergence-deps
evaluated: 2026-04-07T00:00:00Z
sha: ada7a0f

T6.3:
  gap: not started
  assessment: "Neither README nor agents-guide include a CLAUDE.md snippet."
  read:
    - README.md
    - docs/agents-guide.md

T4.1:
  gap: not started
  assessment: "mnemo feature, no code in bullseye repo."
  read: []

T6.1:
  gap: not started
  assessment: "No bullseye_init tool exists."
  read: []

T1.2:
  gap: not started
  assessment: "No momentum signal in ranking."
  read: []

T1.4:
  gap: not started
  assessment: "Depends on mnemo T10."
  read: []

T2.1:
  gap: not started
  assessment: "No portfolio capability."
  read: []

T5.1:
  gap: not started
  assessment: "No converter tool."
  read: []

T1.1:
  gap: not started
  assessment: "No checks field or verify tool."
  read: []

T5:
  gap: significant
  assessment: "This repo runs both systems. No other repos migrated."
  read: []

T1:
  gap: not started
  assessment: "No integration between triad servers."
  read: []

T2:
  gap: not started
  assessment: "All sub-targets identified."
  read: []

T3:
  gap: not started
  assessment: "Depends on portfolio view."
  read: []

bullseye:
  ranking: 1
  blocking: 1
  data_quality: 0
  overall: 1
  markdown_rec: T6.3
  bullseye_rec: T6.3
-->
