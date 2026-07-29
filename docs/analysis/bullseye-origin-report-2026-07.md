# How Bullseye Became What It Is

**Date:** 2026-07-29  
**Sources:** git history of [marcelocantos/bullseye](https://github.com/marcelocantos/bullseye), project docs (`docs/design.md`, `docs/mcp-triad.md`, `docs/audit-log.md`, `STABILITY.md`, `bullseye.yaml`), and mnemo session/activity indexes for this repo.  
**Span:** first commit 2026-03-24 → present (~4 months, 39 minor releases v0.2–v0.39, schema v1→v5).

This is an origin-and-shape report, not a changelog. It explains *why* the system looks the way it does, including how the store moved from markdown to YAML, and closes with two reflections: engineering learnings and months of live use.

---

## 1. Before the binary: markdown targets and `/cv`

Long before the Rust crate existed, “convergence targets” were a **human+agent workflow** encoded in markdown:

- Files like `docs/targets.md` (🎯T-prefixed headings, status/weight/acceptance prose).
- A skill (`/cv`) that **parsed markdown**, often spawned an agent to assess gaps, and ranked work (historically with a Python script).

That design matched an earlier world: scarce human attention, careful prioritisation, “what is the single best next use of my time?” The markdown file was easy to read in PRs and edit by hand. It was also **fragile**: no formal schema, regex-tolerant parsing, format drift across repos, and non-deterministic “is this achieved?” judgments.

Bullseye is the answer to that bottleneck: **make the plan machine-owned** without giving up agent-driven execution.

---

## 2. Genesis (late March 2026): a typed YAML MCP server

| When | What |
|------|------|
| 2026-03-24 | `cargo init` → **Implement targets MCP server with YAML schema** |
| Same day | **YAML as source of truth**; `targets_render` auto-renders `docs/targets.md` for humans |

The founding commit is explicit:

> Replaces the markdown-based `targets.md` format with a structured YAML schema and provides typed tool operations instead of regex parsing.

Initial surface (then `targets_*` tools): list/get/add/update/retire/rank/validate/graph. Core ideas already present:

- **Desired states as testable properties** (acceptance criteria), not a to-do list.
- **Dependency graph** with validation (cycles, dangling refs).
- **Discovery** walking up from `cwd`.
- **WSJF-style ranking** (value/cost) — later demoted (see §4).

So YAML was not a late aesthetic choice; it was the **product thesis from day one**: reliable serde, typed tools, something agents mutate via MCP instead of hand-editing freeform markdown.

### YAML vs markdown (why YAML won)

Markdown stayed in the picture briefly as a **generated view**, not as the store:

1. **Parse reliability.** Markdown had “no formal schema — it's whatever `/cv` and `render.rs` produce” (T5.1 context). YAML + serde gives round-trips and validation; agents stop inventing field layouts.
2. **Mutation safety.** Tool-shaped writes (add/update/retire) beat “edit a heading and hope the skill still parses.”
3. **Graph as data.** Dependencies, statuses, and portfolio fields are first-class maps/lists — not prose that must be re-parsed.
4. **Dual-write transition, then delete the projection.** Early on: *YAML is SoT; markdown is rendered and never edited* (`targets_render` on every save). Migration plan (🎯T5, 2026-04-07) converted fleet markdown → YAML, dual-read for `/cv`, then cut over. **2026-04-12 (v0.14 / #32):** rename `targets.yaml` → **`bullseye.yaml` at repo root**, **delete `render.rs` and markdown auto-render entirely**. The human-readable view became **tool output** (`frontier`, `summary`, `convergence`) rather than a second file that could drift or get hand-edited.

In short: markdown was optimised for *reading in a browser*; YAML was optimised for *agents and tools as the primary editors*. After cutover, keeping a rendered twin added complexity without enough value.

---

## 3. Naming and the triad (early April 2026)

| When | What |
|------|------|
| 2026-04-04 | Hierarchical state-machine design (`docs/design.md`); frontier tool; `kind` / verifies / rework / tunnels |
| 2026-04-07 | **Rename product `targets` → `bullseye`** (crate, MCP name, tool prefixes) |
| Same week | `docs/mcp-triad.md`: **bullseye (plan) + sawmill (code) + mnemo (history)** |

The triad note frames the whole project: three MCP servers replace monolithic `/cv` (parse markdown → spawn agent → rank). Bullseye owns **intent**; sawmill owns **structure**; mnemo owns **what happened**. That architecture still organises agent directives and many later targets (T1.x).

The product rename mattered: “targets” was the domain noun; **Bullseye** is the ledger service you install and register.

---

## 4. Open-sourcing and shedding human-centric ranking (April 2026)

Rapid open-source packaging (audit, LICENSE, CI, Homebrew, agents guide) sat next to a philosophical shift:

**Remove WSJF from repo-level scheduling** (commit `5cd4e5e`):

> WSJF is a human-centric scheduling model that doesn't apply to agentic programming. Agents work the entire frontier in parallel… WSJF only matters at the portfolio level.

Consequences that stuck:

- **Frontier-first** within a repo (unblocked set = parallelisable work).
- **Single edge type** `depends_on` (parent/child and later `gates` collapsed or migrated).
- **value/cost retained** as portfolio metadata, not local ordering.
- **`bullseye_import`** for markdown→YAML migration; T5 cutover retired once the fleet moved.

This is the first major “usage taught us the model was wrong” moment: ranking for scarcity does not match multi-agent capacity.

---

## 5. Schema thickening, then thinning (April–May 2026)

### Additions that reflected real failure modes

- **Verification kinds, rework edges, retry budgets, tunnels** — try to make failure and checkpoints first-class (design.md’s agent-capacity motivation).
- **`showcase` / observable checkpoints** — force “user-visible progress” so graphs aren’t pure tunnels of work with no demo.
- **`set_aside`** — park work without fake “achieved.”
- **Immutability of achieved targets** — history is not rewritten casually.
- **External storage** — corporate/read-only repos cannot take in-repo `bullseye.yaml`.
- **Git-history ID allocation** — parallel branches must not invent colliding `T*` IDs.
- **Auto-commit of `bullseye.yaml`** — mutations become durable without asking the agent to remember `git commit`.
- **Envelope-leak / control-character guards** — agents pasting tool garbage into free text.
- **Convergence + standing invariants** (`make bullseye`) — “what next” only after the tree is green.

### Removals that reflected over-modelling

Schema versions (now **v5**) document a deliberate **thinning**:

| Version | Change |
|---------|--------|
| v2 | observable → showcase |
| v3 | `set_aside` |
| v4 | **Remove showcase construct** entirely |
| v5 | **Remove `kind` and checkpoint/tunnel apparatus** — all targets uniform; acceptance prose is the contract |

The showcase era produced a known pain (visible in mnemo topics): *everything blocked because nothing is a showcase; everything’s a tunnel* — graphs built in advance without enough evidence of what “done” looks like. The response was not more kinds of nodes; it was **fewer discriminators** and clearer lifecycle (`achieve` / `revert` / `defer`).

**Lesson:** first-class workflow vocabulary is seductive; in practice, free-text acceptance + hard `depends_on` + good tools beat a miniature process language.

---

## 6. From many tools to a four-tool ledger (June–July 2026)

The agent surface grew organically (`put`, `retire`, `subdivide`, `convergence`, portfolio, github, …). 🎯T45 crystallised the **core intent-ledger API**:

- `bullseye_open` / `bullseye_query` / `bullseye_commit` / `bullseye_plan_checks`
- Mutation envelope: `ok`, `ids`, `changed`, `frontier`, stable `code=` errors
- Legacy names as shims; portfolio/github/convergence as **L2**
- Policy: **user intent overrides the frontier**; bullseye records claims, does not assign work

This is the second major “usage-shaped” redesign: **too many entry points** made agents invent parallel workflows. A small core + explicit L2 matches how sessions actually behave.

Related operational hardening from live multi-agent use:

- **Child allocation / ban `T4.0`** — humans and agents confuse parent and child numbering.
- **“Never predict the next ID”** (T44) — even with git-history allocation, agents grepping max `T*` reintroduced TOCTOU in *narration*.
- **T41 direct-edit incident** (Codex/Claudia, Ruby one-liners, U+0001 corruption) — agents must not hand-edit the store; tools + (planned) preamble hash.
- **GitHub issues path** (T34 `gh` mirror → T31 issuepipe Master → T32/T33/T35 event path) — intent ledger as sink for external work queues.

---

## 7. What “bullseye” means today (snapshot)

**Product identity:** Intent ledger MCP + CLI — desired states, dependencies, frontier, claim lifecycle — not a task assigner.

**Storage:** `bullseye.yaml` (in-repo or external shadow tree); optional auto-commit; schema_version 5; ~76 targets in this repo’s own ledger (mostly achieved).

**Scheduling:** Repo = frontier fanout; portfolio = WSJF-ish cross-repo attention (when used).

**Integrations:** sawmill (checks), mnemo (momentum/history), GitHub (`github sync` + issuepipe), priorities SQLite, hygiene.yaml, continuous `/cv` via `bullseye_convergence`.

**Philosophy encoded in code and STABILITY.md:** pre-1.0 settling clock; surface changes reset the clock; 1.0 is a hard compatibility bar.

---

## 8. Reflection: engineering learnings

1. **Start with the failure mode of the previous system.** Regex markdown + LLM eyeballing “done” was non-deterministic and expensive. Typed storage + tools was the correct first move.

2. **Projections need an owner.** Dual YAML + rendered markdown only worked while the renderer was mandatory and markdown was never edited. Once tools produced better views, the projection became dead weight — delete it.

3. **Capacity changes the priority model.** Keeping WSJF at repo scope fought the product (parallel agents). Frontier-first was the coherent design; portfolio ranking is a different problem.

4. **Process DSLs accrete then collapse.** Kinds, verifies, showcase, tunnels answered real stories but raised the cost of *writing* a correct graph. Uniform targets + acceptance + `depends_on` is the durable core.

5. **Multi-agent concurrency is a product requirement.** Locks, CAS, mtime cache, git-history IDs, auto-commit, immutable achieved rows — all emerged from agents racing each other and from branches.

6. **Surface area is cognitive load for agents.** Many MCP tools ≈ many ways to do the wrong thing. Four-tool core with stable errors is an API design lesson from agent operators, not humans clicking a UI.

7. **Trust boundaries beat guidance alone.** “Please don’t edit YAML” failed; control-char rejection, immutable achieved, reserved IDs, and T41-style enforcement are the real fix.

8. **Self-hosting is the best dogfood.** Bullseye’s own `bullseye.yaml` is large and long-lived; features like auto-commit, convergence, and import were proven on the tool that implements them.

---

## 9. Reflection: months of using the tool

mnemo shows **heavy dogfooding** on this repo alone (dozens of sessions and tens of thousands of messages over a few months). Usage patterns that shaped the tool:

**What consistently worked**

- **`/cv` as the session spine** once `bullseye_convergence` + invariants hooks existed: one call → frontier + next action.
- **Targets as assertions** (“system is in state X”) instead of task tickets — when acceptance is honest, retirement is meaningful.
- **Frontier fan-out** for independent work; composite targets with sub-targets as **commits on one branch**, not one PR per sub-target (encoded hard in agent policy).
- **Portfolio / external storage** for “I need intent tracking where I can’t commit files.”
- **Issue mirroring** (T34 then issuepipe) as a bridge between GitHub’s task culture and the ledger model.

**What kept hurting**

- **Graph construction ahead of evidence** — elaborate `depends_on` webs and showcase requirements that blocked everything (“everything’s a tunnel”).
- **Agents treating bullseye as a planner of record** instead of a ledger — inventing targets for one-shot work, or grepping YAML for IDs instead of reading mutation results.
- **Direct file edits and parallel sessions** — corruption, local-only auto-commits, “max T+1” races, submodule/detached HEAD traps.
- **Release process friction** — merge/retire/version choreography and the occasional need to re-sync master; product and process co-evolved under stress.
- **MCP registration / binary skew** — “tool not found” or homebrew lagging source is a recurring operational tax.

**How use changed the user’s own workflow**

- Intent moved from prose docs and markdown checklists into a **queryable graph** that survives sessions (especially with mnemo for history).
- Default question shifted from “what should I prioritise?” to “what is unblocked and is the tree green?”
- Cross-repo life (portfolio, external mode, resolve-by-leaf-name) became as important as single-repo hygiene.
- Trust in agents increased for *execution* only when the ledger and CI invariants could catch lies about “done.”

**Net**

Bullseye is what you get if you take agentic coding seriously for a season: start by replacing fragile human docs with a schema, discover that human priority math is wrong for agents, over-build workflow types then strip them, and finally shrink the API to a ledger while hardening the file against the agents you hired to edit it. YAML over markdown is one chapter of that story — the durable theme is **intent as structured, tool-mediated, concurrency-safe state**.

---

## Appendix: chronology (compressed)

| Period | Theme |
|--------|--------|
| 2026-03-24 | YAML MCP server; markdown as render |
| 2026-04-04 | HSM design; verify/rework/tunnels |
| 2026-04-07 | Rename bullseye; triad; open-source push; T5 migration |
| 2026-04-08–12 | Drop WSJF; single depends_on; cut markdown render; `bullseye.yaml` root |
| 2026-04 mid–late | Portfolio, startup context, put, schema_version, convergence |
| 2026-04–05 | Showcase rise and fall; set_aside; locks; auto-commit; triad demos |
| 2026-05 | Schema v4–v5 thinning; subdivide; git ID alloc |
| 2026-06 | GitHub mirror; CLI/MCP parity; hygiene; resolve |
| 2026-07 | Four-tool API; ID discipline; issuepipe real-time path; build-perf |

*Report assembled for archival use; amend as memory or mnemo surfaces better first-hand quotes.*
