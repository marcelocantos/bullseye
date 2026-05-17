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

### 2. Momentum-aware frontier ordering (targets <-> mnemo)

`targets_frontier` returns all unblocked targets. Within a repo,
all frontier targets can be worked in parallel, so ordering is
advisory rather than prescriptive. But a momentum signal from mnemo
can guide which frontier target to focus on when human attention is
limited.

**Proposal:** The `/cv` skill (or its successor) calls both
`targets_frontier` and `mnemo_recent_activity`, using momentum as
a tiebreaker when suggesting focus:

```
targets_frontier
  → unblocked targets from the YAML graph

mnemo_recent_activity(repo: "targets")
  → per-repo session counts, recency, topics

Combined suggestion:
  preferred = frontier_target with highest recent momentum
  where momentum_factor boosts targets with recent active sessions
  and decays stale ones
```

Note: WSJF ranking (`targets_rank`) has been removed from Bullseye.
Within a single repo, frontier-first scheduling is the right model —
agents work everything unblocked in parallel. Portfolio-level ranking
across repos is deferred to `targets_portfolio` (a planned future
tool, see section 6).

The momentum factor is advisory — it nudges attention toward
continuity (finishing what was started) without blocking parallelism.
The exact formula should be tunable; a reasonable starting point:

```
momentum = 1.0 + 0.3 * log(1 + recent_sessions) * recency_decay
recency_decay = exp(-days_since_last_session / 7)
```

**Data flow:** The `/cv` skill calls both `targets_frontier` and
`mnemo_recent_activity`, merges the signals, and presents a suggested
focus order. Neither server needs to know about the other — the
composition happens at the skill layer.

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

### 4. Revert and re-entry (all three)

When a target turns out not to be as achieved as it looked — a
regression surfaces, an acceptance criterion that wasn't fully
checked fails later — `bullseye_revert` moves it back to converging
and records a timestamped note. The three servers then contribute:

1. **targets** records the revert with a reason and clears the
   achieved date (already implemented in `bullseye_revert`).
2. **sawmill** identifies *what* failed — which convention was
   violated, which structural invariant broke, with line-level
   precision.
3. **mnemo** surfaces *prior attempts* — "this target was worked
   once before in session X; here's what was tried and why it
   didn't hold."

The re-entry diagnosis is the concatenation of all three signals:
structural failure (sawmill) + historical context (mnemo) +
revert reason (targets). This gives the agent maximum context for
the next attempt.

**Data flow:**

```
target regresses after retirement
  → sawmill check_conventions / check_invariants
    → structured failure report (which checks, which files, which lines)
  → mnemo_search("T5 prior work" OR "platform isolation failure")
    → prior sessions and their outcomes
  → bullseye_revert(cwd, "T5", diagnosis: combined_report)
    → moves target to converging, records revert note
    → agent resumes work with full context
```

### 5. What `/cv` becomes

Today `/cv` is a ~200-line skill that:
1. Reads and parses `bullseye.yaml` (fragile markdown parsing)
2. Spawns an agent to assess gaps (expensive, non-deterministic)
3. Formats and presents results

With the triad, it becomes a thin orchestrator (~20 lines):

```
1. targets_frontier → unblocked leaf targets
2. mnemo_recent_activity → momentum signal (optional)
3. Suggest focus order, present top candidates
4. For top candidate: targets_get → acceptance criteria
5. If checks defined: targets_verify → deterministic pass/fail
6. Present recommendation with confidence level
```

Step 1 is a fast typed RPC. Step 2 is optional enrichment.
Steps 4-5 only run for the top candidate. Total: 2-5 tool calls
vs. the current unbounded agent exploration.

## Implementation order

1. **targets: `checks` schema field** — extend Target struct,
   update YAML parsing, validation. No execution yet.
2. **targets: `targets_verify` tool** — iterates checks, calls
   sawmill tools, returns structured report. Requires sawmill
   MCP client in the targets server (or composition at skill layer).
3. **Skill rewrite: `/cv`** — replace YAML parsing + rank.py
   with `targets_rank` + `mnemo_recent_activity` calls.
4. **mnemo: target-aware compaction** — summarizer calls
   `targets_list` to anchor output (depends on 🎯T10).
5. **Rework integration** — wire sawmill diagnosis into
   `targets_rework` payload.

Steps 1-2 can proceed independently of 3-5. Step 3 is the
highest-value integration point (immediate daily workflow
improvement).

## 6. Global portfolio view

### The two-tier attention model

Within a single repo, agent capacity is effectively unlimited. The
model optimises for parallel throughput — frontier computation and
fan-out across all unblocked targets simultaneously. Priority ranking
is unnecessary within a repo because agents can work everything on
the frontier in parallel.

Across repos, the constraint changes. Human attention is finite.
You can't review, steer, and unblock work in 15 repos at once. The
question returns to classical scheduling: "What should I focus on
this hour, today, this week?" Value-weighted prioritisation is
appropriate at this portfolio level — scarce human capacity,
competing demands across repos.

### The global graph

Each repo has its own `bullseye.yaml` with an internal target graph.
The global view meshes these into a cross-repo quasi-graph:

```
repo: mnemo
  T10 (live context compaction)
    depends: jevon Process/Manager API  ←── cross-repo edge
    
repo: targets
  T1.1 (executable acceptance checks)
    depends: sawmill conventions        ←── cross-repo edge
  T1.4 (target-aware compaction)
    depends: mnemo T10                  ←── cross-repo edge

repo: sawmill
  T19 (structural invariants)
    enables: targets T1.1 phase 2       ←── cross-repo edge
```

Cross-repo dependencies are currently tracked informally in `context`
fields and memory notes. The global view makes them explicit and
computable.

### Discovery

The targets server discovers all `bullseye.yaml` files across the
managed repo forest. Sources:

1. **`~/.claude/managed-repos.md`** — canonical repo list
2. **Walk `~/work/`** — fallback discovery
3. **mnemo_repos** — repos with recent session activity (may include
   repos not yet in managed-repos.md)

For each discovered repo, load `bullseye.yaml` (if it exists).

### Cross-repo edges

A new field on targets:

```yaml
T10:
  name: Live context compaction
  cross_depends:
    - repo: marcelocantos/jevon
      capability: "claude.Process / manager.Manager API"
      note: "Summarizer lifecycle management"
  cross_enables:
    - repo: marcelocantos/targets
      target: T1.4
```

`cross_depends` is advisory — it doesn't block frontier computation
(the dependency is on a capability, not a target state). But it
surfaces in the global view so the human can make informed decisions.

`cross_enables` tracks value propagation — work on this target
unblocks work in another repo.

### Global ranking

At the portfolio level, each repo gets an aggregate score:

```
repo_priority = Σ (frontier_target_value × momentum) / repo_count_on_frontier
```

Where:
- `frontier_target_value` — value score of each target on the
  repo's frontier (value/cost ratio used as priority signal at
  the portfolio level, where human attention is the scarce resource)
- `momentum` — from mnemo_recent_activity (session recency/frequency)
- `repo_count_on_frontier` — number of unblocked targets (more
  frontier = more parallelisable = less human attention needed
  per unit of progress)

The global `/cv` presents:
1. **Portfolio ranking** — repos ordered by aggregate priority
2. **Top target per repo** — what to work on if you enter this repo
3. **Cross-repo blockers** — targets in repo A that unblock work
   in repo B (these get a priority boost because their value
   propagates)
4. **Momentum report** — which repos have recent activity, which
   are stale

### Tool: `targets_portfolio`

A new MCP tool on the targets server:

```
targets_portfolio
  repos: [list of repo paths, or "all" for discovery]
  days: recency window for momentum (default 7)
  
Returns:
  - Per-repo: frontier targets, aggregate priority, momentum
  - Cross-repo edges: blockers and enablers
  - Recommended focus: top 1-3 repos with reasoning
```

This tool calls mnemo_recent_activity for momentum data. The
composition question (direct MCP call vs skill-layer) applies here
too — start with skill-layer orchestration.

### Interaction with `/cv`

`/cv` gains a scope parameter:

- `/cv` (no args, current behaviour) — evaluate current repo
- `/cv global` — portfolio-level evaluation across all repos
- `/cv scan` — lightweight single-repo scan (existing)

The global evaluation is heavier (discovers repos, loads multiple
YAML files, calls mnemo) so it runs only when explicitly requested
or at session start when no specific repo context is established.

## 7. Protocol app integration (portfolio → phone)

The portfolio view produces a ranked list of what to focus on. That
ranking should surface on the Protocol app's Today page — so the user
sees their top priorities when they pick up their phone in the morning.

### Data flow

```
targets_portfolio (laptop)
  → top N frontier targets with context
  → written to a SQLite table (e.g., targets_priorities)

sqlpipe (bidirectional sync over pigeon relay)
  → replicates targets_priorities to Protocol's protocol.db on phone

Protocol Today page (Android)
  → renders priorities section above the daily checklist
```

### The sync layer

Protocol already uses SQLite (`protocol.db`) for all storage. sqlpipe
provides bidirectional SQLite replication with reconnect diff sync.
pigeon (formerly tern) provides the encrypted WebSocket relay. jevon
🎯T10 already tracks sqlpipe-based state sync for the jevon mobile
app — the same infrastructure serves Protocol.

### Schema: `targets_priorities` table

Laptop-owned (Protocol is read-only for this table):

```sql
CREATE TABLE targets_priorities (
    id TEXT PRIMARY KEY,          -- "mnemo/T3", "targets/T1.1"
    repo TEXT NOT NULL,           -- "marcelocantos/mnemo"
    name TEXT NOT NULL,           -- "Active work dashboard data"
    priority REAL NOT NULL,       -- effective priority (portfolio-level value signal)
    context TEXT,                 -- why this is important now
    horizon TEXT DEFAULT 'today', -- "today", "tomorrow", "this_week"
    updated_at TEXT NOT NULL      -- ISO timestamp
);
```

The `horizon` field maps to Protocol's daily view — "today" items
appear on today's page, "tomorrow" on tomorrow's. "this_week" items
appear in a separate section or as lower-priority entries.

### Sync cadence

A cron job (or mnemo daemon hook) runs `targets_portfolio` periodically
(e.g., every 30 minutes) and upserts results into `targets_priorities`.
sqlpipe replicates the changes to the phone on next sync. The phone
sees fresh priorities without any manual action.

### Protocol UI

The Today page gains a "Focus" section above the checklist:

```
┌──────────────────────────────────┐
│ Focus                            │
│                                  │
│ 🎯 mnemo/T10  p:1.25             │
│   Live context compaction        │
│                                  │
│ 🎯 targets/T1.3  p:2.67          │
│   /cv skill rewrite              │
│                                  │
│ 🎯 sawmill/T19  p:1.6            │
│   Structural invariants          │
├──────────────────────────────────┤
│ Checklist                        │
│ ☐ Morning routine                │
│ ...                              │
└──────────────────────────────────┘
```

Tapping a priority could deep-link to the repo (future), or just
show the target context in a detail sheet.

## 8. Dynamic session startup context (mnemo)

When Claude Code starts a session, it injects static context via
`<system-reminder>` — CLAUDE.md files, git status, date. mnemo can
enrich this with dynamic context derived from transcript history.

### What to inject

The MCP server description (the text that appears in the
`system-reminder` tool listing) can include dynamic content generated
at registration time or refreshed periodically:

1. **Recently active repos** — "You've worked on mnemo (12 sessions),
   sawmill (8), targets (3) in the last 7 days."
2. **Active targets** — top frontier targets from the portfolio view
3. **Last session summary** — if compaction (🎯T10) is running, the
   most recent compacted context for this project
4. **Stale targets** — targets not progressed in >14 days

### Implementation options

**Option A: MCP tool description** — The `mnemo_self` or a new
`mnemo_context` tool includes dynamic text in its description field.
MCP tool descriptions are served fresh on each connection, so they
can include recent data. Downside: tool descriptions are meant to
be stable; dynamic content is a mild abuse of the protocol.

**Option B: Startup tool call** — CLAUDE.md instructs the agent to
call `mnemo_startup_context` at session start. Returns a structured
context block with recent repos, active targets, and last session
summary. The agent integrates this into its working context. More
explicit, no protocol abuse, but requires a tool call round-trip.

**Option C: MCP resource** — Expose the startup context as an MCP
resource (e.g., `mnemo://startup-context/{project}`). Resources are
designed for dynamic content that the client reads at will. If
Claude Code supports MCP resources in the system prompt, this is
the cleanest approach.

**Recommended:** Option B for immediate value (works today), with a
migration path to Option C when MCP resource support matures.

### Recently active repos — the obvious first feature

This is high-value and low-cost. `mnemo_recent_activity` already
exists. A `mnemo_startup_context` tool wraps it with formatting:

```
Recent activity (last 7 days):
  mnemo        12 sessions  last: 2h ago   targets: T3 (achieved), T10 (new)
  sawmill       8 sessions  last: 1d ago   targets: T18 (achieved)
  targets       3 sessions  last: 4h ago   targets: T1, T2 (new)
  pigeon        2 sessions  last: 3d ago
  protocol      1 session   last: 6d ago
```

This immediately answers "where was I?" without running `/waw` or
`/cv`. The agent sees it in context and can decide whether to
continue prior work or start something new.

## 9. Repo-level prioritisation: the phase-boundary hypothesis (updated for 🎯T25)

Bullseye runs two distinct prioritisation engines with *different
objective functions*. The split isn't a layering accident; it
reflects what actually changes as the time horizon stretches from
hours to weeks.

### Within a repo: the human as decision-maker (sub-week horizon)

Inside a single repo, the agent's capacity is effectively unlimited.
Throughput optimisation through value/cost weighting is *noise* at
this scale — agents can work every frontier target in parallel, so
there's no throughput to optimise in the first place.

What actually matters is moving as much of the graph as possible per
unit of agent effort. The frontier is the *parallelisable set*:
agents fan out across all of it, not just the top-ranked item. The
repo-level ordering is therefore a guide for prioritising within the
fan-out, not a serialisation constraint.

The repo-level frontier is sorted by (🎯T25, v0.28.0):

1. **Descending unblocking fanout.** Targets that free the most
   downstream work move more of the graph per unit effort. This is
   the count of active targets listing this one in `depends_on`.
2. **Ascending target ID.** Pure determinism.

`value`, `cost`, `momentum`, and the earlier distance-to-checkpoint
signal do not enter this ordering. The distance-to-checkpoint and
tunnel apparatus was retired in 🎯T25 — the uniform-node model
removes the verify-kind distinction that made checkpoints a
structural concept, and the ordering simplifies accordingly.

### Across repos: the human as bottleneck allocator (weekly-plus horizon)

Zoom out to the portfolio level and the constraint changes shape.
The human can't steer fifteen repos in a single afternoon; human
attention itself becomes the scarce resource. Classical
value-weighted scheduling earns its keep here: WSJF, momentum,
cross-repo enablement. That's what `src/portfolio.rs` implements,
and it's the correct scope for those signals.

The two engines use the same target graph but ask different
questions. Repo scope asks "Which of these unblocked targets frees
the most downstream work?". Portfolio scope asks "Which repo should
I enter at all this week?". Mixing the signals in either direction
corrupts both answers — that's why `Target::value` and
`Target::cost` carry explicit doc comments declaring them
portfolio-scope, and why repo-level frontier ranking takes no
momentum parameter.

### Why value/cost still exist

They remain on every target because the portfolio engine consumes
them, and because the human uses them as shorthand when sketching
new targets (they're quick fields to fill in and carry useful
signal at the portfolio level). Dropping them would break the
portfolio view. The repo-level code paths do not consume them —
they're portfolio-scope inputs, not universal sort keys.

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

- **Global ranking granularity**: Should the portfolio rank repos
  (coarse) or individual targets across repos (fine)? Repo-level is
  simpler and matches the human decision ("which project do I enter?").
  Target-level is more precise but may be noise — the top target in
  a low-priority repo is still low-priority. Likely answer: rank
  repos, then show per-repo frontier within each.

- **Cross-repo edge discovery**: Manual `cross_depends`/`cross_enables`
  fields are accurate but high-friction. Could mnemo detect cross-repo
  references automatically (sessions that touch multiple repos, search
  queries that reference another repo's targets)? Worth exploring as
  an enrichment layer on top of manual edges.
