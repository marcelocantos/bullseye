# Convergence Engine: Design Document

## Motivation

The convergence target system was originally designed around human
development workflows: scarce capacity, careful prioritisation, serial
execution with occasional delegation. Priority ranking answers "what's
the single best use of my time right now?" — a question rooted in
scarcity.

With AI agents doing the coding, the capacity constraint largely
disappears. Work that took a week takes hours. Multiple agents can run
in parallel. The question shifts from "what do I do now vs later?" to
"how do I structure work so agents can execute it all concurrently, and
I can see what's happening along the way?"

This demands a different model. The target DAG handles prioritisation
well but is weak on:

- **Failure and rework.** A DAG has no backward edges. When
  verification fails, the system has no vocabulary for "go back and
  try again with this diagnosis." Rework is handled ad hoc by the LLM.

- **Verification as a first-class concern.** The DAG treats
  verification as a property of targets (acceptance criteria) rather
  than as explicit checkpoints in the workflow. This makes it easy to
  accumulate unverified work — "tunnels" where agents drift without
  course correction.

- **Parallelism structure.** The DAG infers parallelism from lack of
  dependencies, but can't express coordinated concurrency, fan-out/
  fan-in patterns, or competing hypotheses.

- **Resource contention.** Two independent targets might need the same
  files, but the DAG has no model for this. Real parallelism depends on
  resource availability, not just logical independence.

- **Composability.** Common workflow patterns (code-and-test diamond,
  spike-then-build, oracle comparison) recur across every project but
  must be reinvented each time in ad hoc target structures.

## Core Model: Hierarchical State Machines

The system is a **hierarchical state machine** (in the Harel
statechart tradition) where:

- **Nodes are states of the system**, not desired outcomes. Each state
  represents a condition the project is in, with defined work to
  perform and defined exits.

- **Edges are transitions** carrying typed payloads. A transition says
  "this outcome occurred, here's what we learned, and here's where to
  go next." Edges can go backward — rework loops are first-class.

- **Hierarchical composition** allows any state to contain a
  sub-machine. The parent sees only entry and exit transitions. Internal
  structure is encapsulated. This is the decomposition mechanism.

- **Common patterns become reusable templates** — self-contained
  sub-machines with well-defined interfaces (entry context, exit
  transitions with payloads). Building a project means composing
  templates.

### States

A state has:

- **Entry conditions** — what must be true (and what information must
  be available) before this state is entered.
- **Work** — what the agent does while in this state. May be empty for
  pure decision/verification states.
- **Exit transitions** — named outcomes, each leading to another state
  and carrying a typed payload. Every state has at least one exit.
- **Resources** — what the state needs to acquire while active
  (see [Resources](#resources)).
- **Retry budget** — for states that can be re-entered via rework
  transitions, how many attempts before escalating.

A state is either:

- **Atomic** — a leaf. The agent enters, does work, exits via one of
  the defined transitions.
- **Composite** — contains a sub-machine. The parent enters when the
  sub-machine's initial state is entered, and exits when the
  sub-machine reaches a terminal state. The parent's exit transitions
  map to the sub-machine's terminal states.

### Transitions

A transition has:

- **Source state** — where it originates.
- **Target state** — where it leads. May be the same state (self-loop)
  or an ancestor/sibling in the hierarchy (rework).
- **Guard** — an optional condition that must hold for the transition
  to fire.
- **Payload** — information produced by the source state and consumed
  by the target. This is how learning propagates through the graph.
  A rework transition carries a diagnosis. A success transition carries
  an artifact reference. A spike completion carries knowledge.

Transition payloads are the mechanism by which backward edges avoid
repeating mistakes. You never re-enter a state with less information
than you had before.

### Parallelism

There are three kinds of concurrency in the model, each with different
semantics:

#### 1. Dependency-inferred parallelism (the common case)

States that have no dependency between them can execute concurrently.
The runtime examines the **frontier** — all states whose entry
dependencies are satisfied — and fans out across them. This is
declarative: you state constraints, the runtime infers parallelism.

This handles the vast majority of concurrent work. Two feature
implementations that touch different files. Writing code and writing
tests for a defined API contract. Independent sub-components.

No explicit "parallel region" construct is needed. The runtime
computes maximal parallelism from the dependency graph.

#### 2. Backdrops (co-resident environments)

Some states require a long-running process to be active while they
execute. A test suite needs a dev server. An integration test needs
both an app server and a database. An oracle comparison needs both
old and new systems serving traffic.

A **backdrop** is an ephemeral resource that is started before the
enclosed states begin and stopped after they complete (or fail). The
enclosed states don't depend on the backdrop in the graph sense — they
assume it. The runtime manages the lifecycle.

```
Backdrop(DevServer) {
  RunE2ETests → VisualCheck → PerformanceCheck
}
```

Backdrops are distinct from work states because they don't "complete" —
they run continuously and are torn down when no longer needed.

#### 3. Coordinated concurrency (rare)

Occasionally, states must actively interact during execution —
differential testing where the same requests go to two systems
simultaneously, or a live migration where old and new run in parallel
and traffic gradually shifts. These require explicit parallel regions
with defined synchronisation semantics.

This is the rarest form and may not need first-class support in the
initial design. It can be modelled as a composite state with custom
internal structure.

## Resources

States consume resources. The resource model determines actual
parallelism — two logically independent states that need the same file
can't truly run in parallel.

### Granularity

Code writing operates at **file-level** granularity. A state that
modifies `src/api/handler.rs` declares that file as an exclusive
resource. The runtime uses this to schedule non-overlapping work
preferentially.

### Resource types

- **Exclusive** — only one state can hold it at a time. A file being
  edited. A build output directory. A database schema being migrated.

- **Shareable** — multiple readers, one writer. Source code that has
  been written can be read by many consumers (tests, documentation
  generators, linters) but only one agent writes to it.

- **Multipliable** — contention can be resolved by creating instances.
  Git worktrees. Test databases. Container instances.

- **Persistent** — the resource outlives the state that produced it.
  Written source files, built artifacts, deployed services. These flow
  forward through the graph.

- **Ephemeral** — needed only while a state (or backdrop) is active.
  Dev servers, test fixtures, tunnels, temporary files.

### Contention as a scheduling hint, not a lock

The resource model is **advisory**. The runtime prefers non-overlapping
scheduling, but does not hard-block on resource conflicts. When
contention causes a failure (merge conflict, build race, corrupted
output), the verification step catches it and the rework transition
handles it.

This is a deliberate design choice. The cost of a perfect resource
locking protocol is high — both to build and to use. The cost of
occasionally failing and retrying is low, as long as the state machine
makes retry a natural transition. The system expects contention
failures and recovers gracefully rather than trying to prevent them.

The runtime should track retry patterns. If the same resource conflict
recurs, that's a signal that the decomposition needs refinement (the
work should be split differently to avoid the overlap) rather than that
the retry mechanism needs to be smarter.

## Verification

Verification is not a property of states — it is an explicit state in
the machine. Verification states are interspersed throughout the graph
as synchronisation points where the system stops and confirms that work
so far is on track.

### The tunnel problem

A sequence of work states with no intervening verification is a
**tunnel**. The agent can drift arbitrarily far in the wrong direction
before anyone notices. Tunnel depth — the number of hops between
verification states — is a graph health metric. The system should flag
paths with depth > 2 and suggest inserting a verification checkpoint.

### Verification states

A verification state:

- Declares which upstream states it **verifies** and against what
  (a contract, a golden file, a property).
- Has multiple exit transitions: pass, fail-with-diagnosis, and
  possibly escalate.
- The failure transition targets one of the verified upstream states,
  carrying a diagnosis payload.
- May need to attribute blame: "the contract was wrong" (cascade
  backward further) vs "the implementation was wrong" (local rework).

### Oracles

An oracle is the mechanism by which a verification state judges
outcomes. Oracles are pluggable — the verification state is
parameterised by its oracle type:

- **Automated test** — does the output satisfy formal assertions?
  Cheap, fast, precise, but only catches what the tests cover.

- **Property check** — does the output satisfy invariants even if the
  exact expected value isn't specified? (e.g., encode(decode(x)) == x,
  output is sorted, sum is preserved). Broader than specific tests.

- **Golden file** — did the output change from the last approved
  baseline? High signal that something happened, low signal about
  whether it's good or bad. Requires approval flow for expected
  changes.

- **Production oracle** — the most common oracle in practice. An
  existing production system whose output is accepted by customers.
  The new system's output is compared against the old. Differences
  must be explained: new system wrong, old system buggy, or
  legitimately different (with approval).

- **Compilation/type check** — if it compiles and type-checks, the
  contract holds at the interface level. Cheap verification that
  the structural contract is met.

- **Human review** — a person inspects the result. Expensive and slow,
  but can catch issues no automated oracle would. Reserved for
  judgment calls, UX assessment, architectural review.

- **Differential** — two independent implementations are compared.
  Agreement increases confidence; disagreement identifies bugs in one
  or both.

### Golden tests and the approval flow

Golden tests deserve special attention because they invert the usual
specification-implementation relationship. The current output *is* the
specification. The test detects change, not correctness.

When the golden output changes:

1. The system presents the diff to a reviewer.
2. The reviewer classifies the change: expected improvement, acceptable
   difference, or regression.
3. Expected improvements and acceptable differences update the golden
   baseline.
4. Regressions trigger a rework transition.

The **bootstrap problem**: the first run has no baseline. Someone must
approve the initial output as "good enough to be the reference." This
is a distinct state in the machine (no-baseline → produce-initial →
human-approves → baseline-established).

The **drift problem**: accumulated approved changes may cause the
golden baseline to drift from the original intent. Periodically, the
baseline itself needs review — a "recalibrate oracle" transition.

### Production oracle specifics

When the oracle is an existing production system:

- The comparison isn't equality — it's **acceptability transfer**.
  The old system isn't "correct," it's "accepted." The new system
  must be accepted in the same way, plus improvements.

- Differences fall into categories:
  - New system is wrong → rework
  - Old system was buggy → document known bug, approve delta
  - Legitimately different but equivalent → relax comparison criteria
  - Intentional improvement → document and approve

- The comparison criteria often need progressive relaxation. Strict
  byte-for-byte comparison is too tight (floating point, timestamps,
  key ordering). The relaxation itself is a mini state machine: each
  relaxation is justified and approved.

## Reusable Patterns (Templates)

Self-contained sub-machines with well-defined interfaces. Each template
specifies its entry context, internal structure, and exit transitions.
A project is built by composing templates and wiring their transitions.

### The Coding Diamond

The most fundamental template. Separates contract definition from
implementation, enabling parallel work on code and tests.

```
   ┌─────────────────────┐
   │  ContractDefined     │
   │  (API, interface,    │
   │   type signatures)   │
   └──────────┬───────────┘
         ┌────┴─────┐
    ┌────▼──┐  ┌────▼──┐
    │ Code  │  │ Tests │
    └───┬───┘  └───┬───┘
        └────┬─────┘
        ┌────▼──────┐
        │  Verify   │
        └───┬───┬───┘
   [pass]   │   │  [fail: contract wrong]
            │   └──→ ContractDefined (with: revision needed)
            │
   [fail: impl wrong]
            └──→ Code (with: failure diagnosis)
```

Entry: goal description, constraints.
Exits: pass (code + tests verified), escalate (retry budget exceeded).

### The Spike

Exploration when you don't know enough to define a contract. Produces
knowledge, not artifacts. The spike's output is discarded; only the
learning persists.

```
  ┌────────────┐
  │  Explore   │
  └──┬─────┬───┘
     │     │
[learned   [dead end]
 enough]       │
     │    ┌────▼──────────┐
     │    │ ReframeGoal   │
     │    │ (with: what   │
     │    │  we learned)  │
     │    └───────────────┘
     │
┌────▼────────────┐
│ ProceedWithBuild│
│ (with: knowledge│
│  from spike)    │
└─────────────────┘
```

Entry: vague goal, uncertainty about approach.
Exits: proceed (with knowledge), reframe (with reasons why the
original goal needs revision).

### The Oracle Comparison

Run a new system alongside a reference (often production) and validate
equivalence or explicable improvement.

```
┌─────────────────────────────┐
│  Backdrop(OldSystem)        │
│                             │
│  ┌───────────────┐          │
│  │ BuildNewSystem│          │
│  └───────┬───────┘          │
│  ┌───────▼───────┐          │
│  │CompareOutputs │          │
│  └──┬────┬────┬──┘          │
│     │    │    │             │
└─────┼────┼────┼─────────────┘
      │    │    │
[identical] │  [unexplained diff]
      │    │         │
      │ [explicable  └→ Investigate
      │  improvement]      │
      │    │          ┌────┴──────┐
      │    ▼          │           │
      │ ApproveDeltas [new wrong] [old buggy]
      │    │          │           │
      │    │          ▼           ▼
      │    │       FixNew    DocumentBug
      │    │          │        → ApproveDeltas
      ▼    ▼          │
   ConfidenceBuild ◄──┘
      │
      ▼
   CutOver
```

Entry: reference system available, new system buildable.
Exits: cut over (new system validated), abandon (new approach needed).

### Progressive Refinement

Iterative improvement toward a quality threshold. Each cycle produces
a better version, not a fix for a failure. The backward edge carries
feedback, not a bug report.

```
  ┌────────┐
  │ Draft  │
  └───┬────┘
  ┌───▼────┐
  │ Review │
  └──┬──┬──┘
     │  │
[good   [refine]
enough]    │
     │  ┌──▼───────────────────┐
     │  │ Draft                │
     │  │ (with: feedback from │
     │  │  previous review)    │
     │  └──────────────────────┘
     ▼
   Done
```

Entry: initial requirements.
Exits: done (quality threshold met), escalate (iteration budget
exceeded without convergence).

### The Migration

Old and new systems coexist during transition, with a controlled
cut-over.

```
NewReady → par(OldRunning, NewRunning) → ValidateEquivalence
  → [equivalent] → CutOver → Decommission
  → [divergent] → FixNew (with: divergence details) → NewReady
```

Entry: old system running, new system buildable.
Exits: decommissioned (migration complete), rollback (new system
can't match).

### The Rollout

A validated solution replicated across N contexts.

```
FirstInstance → Validate → ExtractTemplate
  → par(Instance2, Instance3, ..., InstanceN)
  → ValidateAll → Done
```

Any instance failure may indicate a problem with the template itself,
cascading back to template revision.

Entry: working solution in one context, list of target contexts.
Exits: all instances validated, template revision needed.

### Convergent Exploration

Multiple approaches tried in parallel when the best path is unknown.
Competing hypotheses, not parallel execution of the same plan.

```
par(ApproachA, ApproachB, ApproachC)
  → [any succeeds] → EvaluateWinners → Adopt(best) → Integrate
  → [all fail] → Reframe (with: what each approach taught us)
```

Entry: problem statement, candidate approaches.
Exits: one approach adopted, all approaches failed (reframe needed).

## Resilience

The model treats failure as a normal transition, not an exception. This
is the core resilience principle: **don't try to prevent all failures;
make recovery cheap.**

### Retry budgets

Every state that can be re-entered via a rework transition has a retry
budget. This is an explicit bound on loops — without it, verify →
rework → verify cycles can repeat indefinitely.

When the retry budget is exhausted, the state transitions to an
**escalation** exit rather than retrying. Escalation typically means:

- Surface the problem to the user for a decision.
- Or propagate the failure up the hierarchy — the parent machine's
  exit-on-failure transition fires.

### Failure attribution

When verification fails, the diagnosis must attribute the failure to
one or more upstream states:

- **Local failure** — the implementation is wrong, rework the
  specific state.
- **Contract failure** — the interface/contract was wrong, rework
  cascades backward to the contract definition, potentially
  invalidating other work that built on it.
- **Environmental failure** — resource contention, flaky test,
  transient error. Retry without diagnosis changes.

Contract failures are the most expensive because they cascade.
The state machine's structure should minimise the blast radius of
contract revisions by keeping the distance between contract definition
and contract verification short.

### Staleness cascades

When a state is revised (not just re-achieved, but its outputs or
contracts change), everything downstream that built on those outputs
becomes **stale**. Stale is not failed — it's "no longer verified."
Stale states need re-verification but may not need rework.

The runtime walks forward from the revised state and marks downstream
states as stale. The next verification pass determines which are still
valid and which need rework.

## Graph Health Metrics

The system should continuously monitor the graph structure for common
problems:

### Tunnel depth

The number of hops between verification states on any path. If > 2,
the system flags the path and suggests inserting a verification
checkpoint. The fix is often the same intervention that enables
fan-out — define an interface/contract that is both verifiable and
a parallelism boundary.

### Serial chains hiding parallelism

A sequence of states that appears serial but contains a hidden
interface that would enable fan-out. The system should prompt: "Is
there a contract in this chain that would let downstream work start
earlier?"

### Resource clustering

Multiple states in the frontier that contend on the same resources.
The system should suggest decomposition along resource boundaries
to maximise actual (not just logical) parallelism.

### Unbalanced fan-out

A fan-out where one branch is much longer than the others, creating
a bottleneck at the fan-in point. The system should suggest
decomposing the long branch or inserting intermediate verification
points.

## The Human's Role

In this model, the human's primary job is **designing the machine** —
deciding how to decompose work, where to place verification
checkpoints, which patterns to compose, and what oracles to use. This
is the creative, non-parallelisable work that requires understanding
interfaces and making judgment calls.

The agent's job is **executing within the machine** — entering states,
performing work, producing outputs that enable transitions. The machine
tells the agent what's possible next; the agent doesn't need to reason
about sequencing or prioritisation.

The human intervenes when:

- A state reaches an escalation exit (retry budget exceeded).
- A verification state requires a human oracle (review, approval).
- The graph health metrics flag structural problems.
- The machine itself needs revision (a pattern isn't working, a
  decomposition was wrong).

## Relationship to Value and Priority

Value and cost inform decomposition but don't drive sequencing:

- **Value** becomes a coarse filter: is this worth doing at all? A
  target with value < cost might be dropped entirely. But among
  targets worth doing, value doesn't drive sequencing — the machine
  structure does.

- **Cost** informs decomposition decisions. High-cost states should
  be decomposed further. But cost doesn't drive priority — it drives
  granularity.

- **Sequencing** is driven by the machine structure: dependencies,
  resource availability, and the desire to reach verification
  checkpoints early. The system computes **waves** — sets of states
  that can execute concurrently — ordered by when they produce
  verifiable results.

- **Review order** replaces priority. The human wants to see results
  in an order that lets them course-correct early. High-observability
  verification checkpoints are staged early, not because they're more
  valuable, but because they're more informative.

Note: WSJF (weighted shortest job first) ranking was present in early
versions of Bullseye but has been removed. Within a single repo,
frontier-first scheduling (work everything unblocked in parallel) is
the right model. Portfolio-level ranking across repos is deferred to
`bullseye_portfolio` (a future tool).

## Relationship to TLA+

The state machine model maps directly to TLA+ specifications:

- Each template is a TLA+ module with defined state variables,
  initial conditions, and next-state relations.
- Composition of templates is composition of TLA+ modules.
- Retry budgets and loop bounds are fairness and termination
  conditions.
- The runtime's scheduling of concurrent states is a model of
  process interleaving that TLC can explore.

This enables **verification of the machine itself** before execution:

- Does every path eventually reach a terminal state?
- Are verification checkpoints reachable on all paths?
- Do failure cascades terminate (no unbounded staleness propagation)?
- Are retry budgets sufficient (does the model converge under
  plausible failure rates)?

The TLA+ formalisation is a future step. The initial implementation
should be designed with formal verification in mind — clean state
definitions, explicit transitions, no implicit side effects — so
that adding TLA+ specifications later is a refinement, not a rewrite.

## Evolution Path

### Phase 1: Schema and runtime (current)

Replace the YAML target schema with a state machine schema. The MCP
server becomes a state machine runtime that:

- Loads the machine definition
- Tracks current state across all active regions
- Computes the frontier (states ready to execute)
- Schedules work with resource-aware parallelism
- Handles transitions, including rework loops
- Reports graph health metrics

### Phase 2: Template library

Build out the reusable templates (diamond, spike, oracle, refinement,
migration, rollout, exploration). Each template is a parameterised
sub-machine that can be instantiated into any project's graph.

### Phase 3: Skill integration

Rewrite `/cv`, `/target`, `/wrap` to operate on the state machine
rather than a flat target list. `/cv` computes the frontier and
suggests the next wave. `/target` adds states to the machine.
`/wrap` records transitions that occurred during the session.

### Phase 4: TLA+ formalisation

Define the formal semantics of the state machine model. Provide
TLA+ modules for each template. Enable model-checking of project
graphs before execution.
