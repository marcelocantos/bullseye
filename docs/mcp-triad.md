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

## 6. Global portfolio view

### The two-tier attention model

Within a single repo, agent capacity is effectively unlimited. The
state machine model optimises for parallel throughput — frontier
computation, fan-out, verification checkpoints. WSJF ranking is
unnecessary because agents can work everything on the frontier
simultaneously.

Across repos, the constraint changes. Human attention is finite.
You can't review, steer, and unblock work in 15 repos at once. The
question returns to classical scheduling: "What should I focus on
this hour, today, this week?" This is WSJF's home turf — scarce
capacity, competing demands, value-weighted prioritisation.

### The global graph

Each repo has its own `targets.yaml` with an internal target graph.
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

The targets server discovers all `targets.yaml` files across the
managed repo forest. Sources:

1. **`~/.claude/managed-repos.md`** — canonical repo list
2. **Walk `~/work/`** — fallback discovery
3. **mnemo_repos** — repos with recent session activity (may include
   repos not yet in managed-repos.md)

For each discovered repo, load `docs/targets.yaml` (if it exists).
Repos still using markdown `docs/targets.md` are excluded until
migrated, or optionally parsed with a best-effort adapter.

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
repo_priority = Σ (frontier_target_weight × momentum) / repo_count_on_frontier
```

Where:
- `frontier_target_weight` — WSJF weight of each target on the
  repo's frontier
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
    weight REAL NOT NULL,         -- effective priority
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
│ 🎯 mnemo/T10  w:1.25             │
│   Live context compaction        │
│                                  │
│ 🎯 targets/T1.3  w:2.67          │
│   /cv skill rewrite              │
│                                  │
│ 🎯 sawmill/T19  w:1.6            │
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
