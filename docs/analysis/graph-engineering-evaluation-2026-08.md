# Graph Engineering → Bullseye & Oracle-First

**Date:** 2026-08-01  
**Source:** [Anatoli Kopadze — Graph Engineering explained](https://x.com/AnatoliKopadze/status/2080668775796314331) (X longform / article, 2026-07-24).  
**Purpose:** Capture a one-time evaluation of which ideas in that piece could improve **bullseye** (intent ledger) and **oracle-first** (verification doctrine / skill), without turning bullseye into a multi-agent runtime.

This is a product/doctrine note, not a changelog. Re-read when deciding whether to file implementation targets.

---

## 1. What the source argues (compressed)

Multi-agent work is better framed as a **graph of jobs**, not a single loop:

| Concept | Claim |
|---------|--------|
| **Node** | One bounded job: defined input, defined output (contract). Free-text-only outputs make machine edges weak. |
| **Edge** | Real only when something **passes** along it — the next job *needs* the prior result. |
| **Fake-edge test** | For each sequential step: does B actually consume A’s result? If no, there is no edge; run in parallel. |
| **Sad chain** | Linear A→B→C→D is a graph with no width: latency is the sum; one stall blocks everything. |
| **Diamond** | Fan-out (breadth) → reduce (code/summarize) → synthesize (judgment). “Where is the split, where is the merge?” |
| **Checker** | Worker must not grade its own work; verifier needs a **fresh context** and a real signal (tests pass), not “agent said done.” |
| **Break modes** | Context collapse on fan-in; **false independence** (shared files/APIs); silent partial merge. |
| **When not to graph** | Small/isolated work; human wants to approve every step; exploration; truly sequential steps. |
| **Anchors** | Topology alone ≠ truth. Need nodes that **cannot be argued with** (tests that ran, money in bank). Frozen rules an optimizer would weaken. |
| **Cost** | Graphs buy width, not judgment; fleets burn tokens; human design/supervision still required. |

Engineers correctly note this is old systems design under a new name — that is a feature (trusted patterns), not a put-down.

---

## 2. Already aligned (do not rebuild)

| Source idea | Bullseye / oracle-first today |
|-------------|-------------------------------|
| Graph = jobs + wait-for edges | `depends_on`, frontier, uniform nodes |
| Diamond / fan-out–merge | `docs/shapes.md` diamond, subdivide, fan-out skill / worktree isolation |
| Topology ≠ truth | Acceptance free text as verification contract; no `kind` / showcase hard gates; T53 hygiene is advisory |
| Don’t force a graph on tiny sequential work | Ledger is not a planning gate; user intent overrides frontier |
| Separate check path | Oracle-first: no self-owned gate inputs; independent verification; “fresh inputs” |
| Parallel width | Frontier as parallelisable set; repo-scope ordering by unblocking fanout |

Bullseye should remain the **intent graph**, not Claude-style “workflow” orchestration or cost metering for agent fleets. Execution coordination belongs to skills, runners, and tools (mnemo history, `/cv`, workflows) — not to target IDs.

---

## 3. Ideas that could improve **bullseye**

Ranked by leverage vs schema risk. Prefer advisory / docs over new node types.

### 3.1 Fake-edge audit (high)

**Productize the source’s one test.** For each `depends_on` edge, ask whether the dependent’s acceptance/context actually **consumes** the predecessor’s outcome, or is only “typed order.”

- Surface as **advisory** graph hygiene (alongside T53 empty-frontier / buried-leaf warnings).
- Example: “edge A→B may be sequential prose only; candidates for parallel.”
- No hard block; agents and humans still type short IDs.

### 3.2 Node contracts (medium)

Source: free-text-only outputs make edges unusable by machines.

- Optional conventions or light fields: “produces: …” / “consumes: …” (or templates in `shapes.md` + agents-guide).
- Avoid reviving `kind` or hard schema splits — free-text structured conventions first.

### 3.3 Merge-step completeness (medium)

Source: silent node failure in wide graphs.

- When a node has many predecessors, validation/summary should **count expected vs achieved deps** and flag gaps — not only “all deps terminal.”
- Partial fan-in that looks complete is a false green.

### 3.4 Diamond filing assist (medium–low)

Shapes already name diamonds; a one-shot “file diamond: design ∥ work ∥ check → validate” helper or stronger `split` presets would match “only shape you need this year” without a runtime.

### 3.5 Explicit “not a graph” guidance (low cost)

Mirror source §8 in agents-guide: exploratory / single-bug / fully sequential → stay linear; graph is for **width**. Reduces over-`depends_on` webs (the process pain T53 was aimed at).

### 3.6 Explicitly out of scope for bullseye

- Multi-agent orchestration runtime, dynamic workflows, per-run cost caps.
- Turning the ledger into an execution DAG engine.

---

## 4. Ideas that could strengthen **oracle-first**

### 4.1 Name **anchors** (high)

Source §9 maps directly to existing doctrine: topology/reports alone do not buy truth.

- **Anchors** = checks that cannot be argued with (tests that **ran**, released artifact, CI green on the **shipped** path).
- **Report nodes** = agent narratives, summaries, “should pass.”
- Anti-pattern: audit that only re-reads the worker’s summary (same system grading itself).

Doctrine already says: no self-owned gate inputs; oracles must run on fresh product inputs. **Naming anchors** makes the failure mode teachable.

### 4.2 Worker ≠ verifier context (high)

Encode next to “no self-owned gate”:

- Verifier must **not** share the worker’s transcript/context.
- Prefer a new subagent, new tool run, or CI job.

Maps to the source’s “fresh context checker” without new product surface.

### 4.3 Multiple lenses (medium)

Source: correct / current / source real — different checks, not one soft LGTM.

- Where residue is large, split acceptance: **correctness oracle**, **freshness/staleness**, **provenance**.

### 4.4 False independence / hidden edges (medium)

Parallel workers sharing a worktree or rate-limited API = hidden edges.

- Isolation (worktree, separate cwd) is part of the **verification plan**, not only speed (fan-out / delegation already lean this way — name it).

### 4.5 Fan-in layering (medium)

On large fan-out: batch → summarize → synthesize; never dump all raw results into one judge. Fits “evidence not machinery” and avoids context collapse.

### 4.6 When a loop is enough (low cost)

If the fake-edge test finds no independent work: **one loop + one oracle** is correct; do not build a graph for ceremony.

---

## 5. Highest-leverage trio (if acting later)

1. **Bullseye:** fake-edge hygiene (advisory) + merge completeness.  
2. **Oracle-first:** name **anchors** + **fresh-context verifier** as non-negotiable.  
3. **Shared vocabulary:** diamond / fake edge / anchor in agents-guide so fleet and humans share words — without an orchestration product.

**Do not** reopen clone-scoped IDs or heavy schema for this evaluation; wins are **edge discipline and honest checks**.

---

## 6. Disposition

| Item | Status as of capture |
|------|----------------------|
| This document | Written for deferred review |
| Implementation targets for 3.x / 4.x | **Not** filed yet — wait for re-read |
| Bullseye as runtime | Explicit non-goal |

When re-reviewing: re-read §3–5, decide which items become bullseye targets vs oracle-first skill/doctrine edits vs docs-only, and drop anything that still looks like orchestration creep.

---

## 7. References

- Source post: https://x.com/AnatoliKopadze/status/2080668775796314331  
- Bullseye shapes: `docs/shapes.md`  
- Graph hygiene (T53): advisory warnings in validate/summary/startup  
- Oracle-first skill: `~/.claude/skills/oracle-first/SKILL.md` (+ `doctrine.md`)  
- Related session discussion: bullseye repo session evaluating the post against product + doctrine (2026-07/08)
