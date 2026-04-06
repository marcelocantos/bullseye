# MCP Triad: targets + sawmill + mnemo

Design note for the integration of three MCP servers into a unified
convergence evaluation and execution system.

## The three servers

Each server owns a distinct concern:

| Server | Owns | Core question |
|--------|------|---------------|
| **targets** | The plan | What states should the project reach? What's blocked? What's next? |
| **sawmill** | The code | Does the codebase match the desired state? What can be queried or transformed? |
| **mnemo** | The history | What happened across sessions? What was decided? What's in flight? |

Together they replace the current `/cv` skill's monolithic approach
(parse markdown, spawn an agent to assess gaps, rank with a Python
script) with composable, typed, testable MCP tool calls.

## Integration surfaces

### 1. Executable acceptance criteria (targets <-> sawmill)

A target's `acceptance` field today is prose: "No platform #ifdefs
outside src/platform/". This is read by an LLM, which eyeballs the
code and makes a judgment. The result is non-deterministic and
expensive.

**Proposal:** Add an optional `checks` field to the target schema —
a list of references to sawmill verification primitives:

```yaml
T3:
  name: Platform code is isolated behind interfaces
  acceptance:
    - No platform #ifdefs outside src/platform/
    - Each platform has a dedicated compilation unit
  checks:
    - convention: no-platform-ifdefs    # sawmill convention check
    - query:                             # sawmill structural query
        kind: preprocessor_directive
        pattern: "ifdef|ifndef|if defined"
        exclude_path: "src/platform/"
        expect: 0
    - invariant: platform-isolation      # sawmill structural invariant (future, T19)
```

**Execution model:** `targets_verify` (new tool) iterates a target's
`checks`, calls sawmill tools for each, and returns a structured
pass/fail report. This makes verification deterministic, fast, and
repeatable — no LLM judgment needed for structural assertions.

**Sawmill capabilities today:**
- `check_conventions` — JavaScript-based checks against the parsed
  AST. Already functional.
- `query` + `find_symbol` — structural queries that can assert
  existence/absence of patterns.

**Sawmill capabilities needed (already designed):**
- Structural invariants (`teach_invariant` / `check_invariants`,
  sawmill 🎯T19) — a structured assertion language for relational
  properties. Blocked on LSP client (sawmill 🎯T13) but degrades
  gracefully to syntactic heuristics.

**Phasing:**
- Phase 1: `checks` field with `convention` and `query` types
  (works today with sawmill's existing tools)
- Phase 2: `invariant` type once sawmill 🎯T19 lands
- Phase 3: LLM-assisted check generation — agent reads prose
  acceptance criteria and proposes executable checks

### 2. Momentum-aware ranking (targets <-> mnemo)

`targets_rank` computes static priority from value, cost, and the
dependency graph. But two targets with the same weight are not
equivalent if one has been actively worked on for three sessions
and the other has been stale for a month.

**Proposal:** Extend `targets_rank` (or add `targets_rank_dynamic`)
to accept a momentum signal from mnemo:

```
targets_rank
  → static ranking from YAML graph

mnemo_recent_activity(repo: "targets")
  → per-repo session counts, recency, topics

Combined ranking:
  effective_priority = static_weight * momentum_factor
  where momentum_factor boosts targets with recent active sessions
  and decays stale ones
```

The momentum factor is advisory — it nudges the ranking toward
continuity (finishing what was started) without overriding the
structural priority. The exact formula should be tunable;
a reasonable starting point:

```
momentum = 1.0 + 0.3 * log(1 + recent_sessions) * recency_decay
recency_decay = exp(-days_since_last_session / 7)
```

**Data flow:** The `/cv` skill (or its successor) calls both
`targets_rank` and `mnemo_recent_activity`, merges the signals,
and presents the combined ranking. Neither server needs to know
about the other — the composition happens at the skill layer.

### 3. Structured context restoration (mnemo <-> targets)

mnemo's 🎯T10 (live context compaction) produces rolling summaries
of active sessions. These summaries are richer when anchored to the
target graph.

**Proposal:** The mnemo summarizer (a daemon-managed Sonnet instance)
calls `targets_list` to anchor its compaction output:

```json
{
  "targets_active": ["T3", "T10"],
  "targets_progressed": {
    "T3": "achieved — mnemo_recent_activity tool implemented"
  },
  "targets_next": "T9.1 (highest unblocked weight)"
}
```

Instead of free-text "what was I working on," the compaction speaks
the language of the target graph. A new session calling
`mnemo_restore` gets structured target context it can immediately
act on.

### 4. Rework diagnosis (all three)

When a verify target fails and triggers `targets_rework`:

1. **targets** records the backward edge with a diagnosis payload
   (already implemented).
2. **sawmill** identifies *what* failed — which convention was
   violated, which structural invariant broke, with line-level
   precision.
3. **mnemo** surfaces *prior attempts* — "this target was reworked
   once before in session X; here's what was tried and why it
   failed."

The rework diagnosis is the concatenation of all three signals:
structural failure (sawmill) + historical context (mnemo) +
retry budget status (targets). This gives the rework agent
maximum context for the next attempt.

**Data flow:**

```
verify target fails
  → sawmill check_conventions / check_invariants
    → structured failure report (which checks, which files, which lines)
  → mnemo_search("T5 rework" OR "platform isolation failure")
    → prior rework attempts and their outcomes
  → targets_rework(verify: "T5", diagnosis: combined_report)
    → resets upstream target, increments retry counter
    → agent resumes work with full diagnosis context
```

### 5. What `/cv` becomes

Today `/cv` is a ~200-line skill that:
1. Reads and parses `docs/targets.md` (fragile markdown parsing)
2. Runs `rank.py` for WSJF ranking
3. Spawns an agent to assess gaps (expensive, non-deterministic)
4. Formats and presents results

With the triad, it becomes a thin orchestrator (~20 lines):

```
1. targets_frontier → unblocked leaf targets
2. targets_rank → static priority ordering
3. mnemo_recent_activity → momentum signal (optional)
4. Merge rankings, present top candidates
5. For top candidate: targets_get → acceptance criteria
6. If checks defined: targets_verify → deterministic pass/fail
7. Present recommendation with confidence level
```

Steps 1-2 are fast typed RPCs. Step 3 is optional enrichment.
Steps 5-6 only run for the top candidate. Total: 3-6 tool calls
vs. the current unbounded agent exploration.

## Implementation order

1. **targets: `checks` schema field** — extend Target struct,
   update YAML parsing, validation. No execution yet.
2. **targets: `targets_verify` tool** — iterates checks, calls
   sawmill tools, returns structured report. Requires sawmill
   MCP client in the targets server (or composition at skill layer).
3. **Skill rewrite: `/cv`** — replace markdown parsing + rank.py
   with `targets_rank` + `mnemo_recent_activity` calls.
4. **mnemo: target-aware compaction** — summarizer calls
   `targets_list` to anchor output (depends on 🎯T10).
5. **Rework integration** — wire sawmill diagnosis into
   `targets_rework` payload.

Steps 1-2 can proceed independently of 3-5. Step 3 is the
highest-value integration point (immediate daily workflow
improvement).

## Open questions

- **MCP server composition**: Should targets call sawmill directly
  (server-to-server RPC), or should the skill layer orchestrate?
  Direct calls are faster but create coupling. Skill-layer
  orchestration is more flexible but adds latency. Likely answer:
  start with skill-layer, migrate hot paths to direct calls if
  latency matters.

- **Check portability**: Should `checks` reference sawmill-specific
  concepts (conventions, invariants) or define a neutral assertion
  format that multiple backends could implement? Neutral is more
  future-proof but adds abstraction cost. Likely answer: start
  sawmill-specific, abstract only if a second backend emerges.

- **Momentum formula**: The log/decay formula is a guess. Should be
  calibrated against real usage data from mnemo. Consider making it
  a named parameter in the targets YAML rather than hardcoding.
