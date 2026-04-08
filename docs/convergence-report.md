# Convergence Report

Evaluated: 2026-04-08
SHA: 83675ad

## Standing invariants

- Tests: **PASS** (46 tests)
- CI: **GREEN** (last run: v0.3.0 release, success)
- Clippy/fmt: not checked this run (CI covers them)

## Gap report

All 20 active targets are at status "Identified" with no implementation progress. This is the first convergence evaluation for this repo.

### 🎯T4.1 mnemo_startup_context tool  [frontier, weight 2.5]
Gap: **not started**
This target lives in the mnemo repo, not bullseye. No work can happen here.

### 🎯T1.2 Momentum-aware ranking via mnemo  [frontier, weight 1.7]
Gap: **not started**
Partially implementable here (ranking API accepts momentum param), but the mnemo signal source doesn't exist yet. Schema and ranking changes in bullseye could be done speculatively.

### 🎯T1.4 Target-aware context compaction  [frontier, weight 1.7]
Gap: **not started**
Lives in mnemo. Depends on mnemo T10. No work possible here.

### 🎯T2.1 Cross-repo target discovery  [frontier, weight 1.7]
Gap: **not started**
Fully implementable in bullseye. New tool that walks managed-repos.md / ~/work/ to discover targets.yaml files. Independent of other MCP servers.

### 🎯T5.1 Markdown-to-YAML target converter  [frontier, weight 1.7]
Gap: **not started**
Fully implementable in bullseye. A bullseye_import tool that parses docs/targets.md and produces docs/targets.yaml. High practical value for migration.

### 🎯T1.1 Executable acceptance checks via sawmill  [frontier, weight 1.6]
Gap: **not started**
Schema change (optional `checks` field) is implementable here. The verify tool needs sawmill, which exists as an MCP server already.

### Parent rollups

- 🎯T1 MCP triad integration: converging (0/5 sub-targets achieved)
- 🎯T2 Global portfolio view: converging (0/4 sub-targets achieved)
- 🎯T3 Protocol app priority sync: converging (0/2 sub-targets achieved) -- deeply blocked
- 🎯T4 Dynamic session startup context: converging (0/2 sub-targets achieved)
- 🎯T5 Migration from markdown targets: converging (0/2 sub-targets achieved)

### Blocked targets (status only)

- 🎯T4.2 CLAUDE.md auto-call directive -- blocked by T4.1
- 🎯T1.3 /cv skill rewrite against MCP tools -- blocked by T1.1, T1.2
- 🎯T2.4 /cv global mode -- blocked by T2.3, T1.3
- 🎯T2.2 Cross-repo dependency edges -- blocked by T2.1
- 🎯T2.3 Portfolio-level WSJF ranking -- blocked by T2.1, T2.2
- 🎯T3.1 targets_priorities SQLite table -- blocked by T2.3
- 🎯T3.2 Protocol Today page Focus section -- blocked by T3.1
- 🎯T5.2 Global CLAUDE.md and skill directives -- blocked by T5.1
- 🎯T1.5 Rework diagnosis integration -- blocked by T1.1

## Recommendation

Work on: **🎯T5.1 Markdown-to-YAML target converter**

Reason: Among the 6 frontier targets, only 🎯T5.1 and 🎯T2.1 are fully implementable within the bullseye repo. The others require work in mnemo or sawmill first. 🎯T5.1 has equal weight to 🎯T2.1 but higher strategic leverage: it unblocks the migration path (🎯T5.2 and the broader 🎯T5 goal), provides immediate practical value for migrating other repos to bullseye, and exercises bullseye's own dogfooding story. 🎯T2.1 (cross-repo discovery) is also viable but less urgent -- there's only one repo using bullseye right now.

Both the markdown ranking and bullseye ranking agree on the frontier set. Both systems show these targets at weight ~1.7. The markdown system's top-ranked unblocked *leaf* is 🎯T4.1 (weight 2.5), but that target lives in mnemo, not here. Within this repo, 🎯T5.1 is the highest-leverage actionable work.

## Suggested action

Add a `bullseye_import` tool to the MCP server that reads a markdown targets file (the format produced by `render.rs`) and emits a valid `targets.yaml`. Start by examining `render.rs` to understand the output format, then write the inverse parser. Test against `docs/targets.md` itself as the canonical fixture.

## Bullseye scorecard

**Ranking**:        +1
**Blocking**:       +1
**Data quality**:   +1
**Overall**:        +1
**Markdown rec**:   🎯T4.1 mnemo_startup_context tool
**Bullseye rec**:   🎯T4.1 mnemo_startup_context tool (via T4 parent at weight 3)
**Notes**: Ranking +1: bullseye's integer weights lose the fractional distinction (T4.1=2.5 vs T1.2=1.7 in markdown become both weight=2 in bullseye), but the parent-level ranking (T4 at weight 3) correctly surfaces the T4 family as top priority. Blocking +1: bullseye's frontier computation correctly identified the same 6 unblocked leaves as the markdown system, with cleaner presentation. Data quality +1: targets.yaml passes validation, all 24 targets present, edges complete, no missing fields. The YAML is well-formed because this repo bootstrapped its own targets -- real migration from markdown repos will be the harder test. Overall +1: bullseye produces equivalent results with less effort (3 tool calls vs parsing a 400-line markdown file). The integer weight rounding is a minor weakness but doesn't change recommendations in practice.

<!-- convergence-deps
evaluated: 2026-04-08T00:00:00Z
sha: 83675ad

T4.1:
  gap: not started
  assessment: "Lives in mnemo repo. No implementation here."
  read:
    - docs/targets.yaml
    - docs/targets.md
    - docs/mcp-triad.md

T1.2:
  gap: not started
  assessment: "Partially implementable (ranking param). Mnemo signal source missing."
  read:
    - src/schema.rs

T1.4:
  gap: not started
  assessment: "Lives in mnemo. Depends on mnemo T10."
  read: []

T2.1:
  gap: not started
  assessment: "Fully implementable in bullseye. No progress."
  read:
    - src/schema.rs

T5.1:
  gap: not started
  assessment: "Fully implementable in bullseye. No progress."
  read:
    - src/render.rs

T1.1:
  gap: not started
  assessment: "Schema change doable here. Verify tool needs sawmill."
  read:
    - src/schema.rs

bullseye:
  ranking: 1
  blocking: 1
  data_quality: 1
  overall: 1
  markdown_rec: T4.1
  bullseye_rec: T4.1
-->
