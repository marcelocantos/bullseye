# Graph-shape vocabulary

Bullseye's primary artifact is the **shape** of the target graph.
Individual nodes are uniform — same structure, same field set, no
`kind` discriminator. The leverage lives in how the nodes are wired:
which depend on which, where the graph fans out, where it converges,
where it chains.

This file catalogues the recurring shapes worth naming. Naming them
gives the agent and the human a shared vocabulary for talking about
graph state mid-planning ("this is hiding a diamond"), and gives the
tool surface (`bullseye_put` with `blocks`, `bullseye_subdivide` with
modes) the names of the operations that produce them.

The shapes are described in the **post-v5 model**: uniform nodes,
`depends_on` is the only structural edge type. No `kind`, no
`verifies`, no `rework`, no `showcase`. If the pass signal comes
from CI, a human ack, or a smoke test, that is described in the
node's acceptance criteria as free text — it is not encoded in the
graph.

## Reading the diagrams

ASCII conventions used below:

- `[X]` is an active node (any status that is not terminal).
- `(X)` is an achieved or set-aside node (terminal, drawn for context).
- `A → B` means **B `depends_on` A**: A must clear before B is unblocked.
  Arrowheads point in the direction the agent works through the graph,
  not in the direction `depends_on` lists. The two readings are
  duals; the diagram orientation matches workflow intuition.

## When to reach for the vocabulary

Run the catalogue against any new or in-flight target whose acceptance
criteria read like "do X, then Y, then check Z". That prose is almost
always hiding a subgraph the YAML doesn't yet make explicit. Naming the
shape — "this is a diamond", "this is a contract-first" — is how the
agent prompts itself to file the decomposition before doing the work,
not after.

For the discipline, see also `~/.claude/convergence.md` § *Graph
shapes*.

---

## diamond

```
        [build]
      ↗         ↘
[design]         [validate]
      ↘         ↗
        [tests]
```

**Heuristic.** Reach for the diamond when a single target's acceptance
reads as "design the thing, do the parallel implementation work, then
prove it together". The fork (build ∥ tests) is what makes the diamond
worth modelling — both sides depend on the same prior decision, and a
single downstream node depends on both sides retiring. Anything that
fits "decide once, work in parallel, converge once" is a diamond.

**When to use.** Pure sequential chains shouldn't be diamonds — there
is no parallelism. A two-step "design → build" without an independent
verification stream isn't a diamond — that's a chain. A diamond
requires at least two genuinely independent middle branches that share
a common predecessor *and* a common successor.

**Worked example.** Extending `bullseye_subdivide` with a `tail`
parameter (🎯T27 in this repo's bullseye.yaml) was filed as a diamond:

- **T27.1 (design)** — design `bullseye_subdivide` and ship the
  three-mode skeleton.
- **T27.2 (build)** — extend retire mode with the `tail` parameter
  for explicit dependent rewiring.
- **T27.3 (tests)** — add end-to-end tests covering each reshape
  pattern from this file.
- **T27.4 (docs)** — update tool descriptions to reference this file
  so the named shapes are reachable from the tool surface.

T27.2 and T27.3 fan out from T27.1, and T27.4 depends on both. Filed
in a single `bullseye_subdivide(parent=T27, mode=retire, tail=[T27.4])`
call: every dependent of T27 is re-pointed at T27.4 only, not at the
whole subgraph.

**Tool call.**

```
bullseye_subdivide(
  parent="X",
  mode="retire",
  children=[
    {id: "X.1", name: "Design",   depends_on: []},
    {id: "X.2", name: "Build",    depends_on: ["X.1"]},
    {id: "X.3", name: "Tests",    depends_on: ["X.1"]},
    {id: "X.4", name: "Validate", depends_on: ["X.2", "X.3"]},
  ],
  tail=["X.4"],
)
```

Without `tail`, retire mode rewires every dependent of X to depend on
all four children — functionally equivalent (X.4 transitively depends
on the rest), but noisy. `tail=["X.4"]` keeps the dependent edges
clean.

---

## fan-out

```
            [child A]
           ↗
[parent] → [child B]
           ↘
            [child C]
```

**Heuristic.** Reach for the fan-out when one prerequisite enables
many genuinely independent children. The point is that the children
can be worked in parallel — there is no convergence node downstream
that forces them to synchronise.

**When to use.** If the children secretly need each other ("write
all the docs, then publish") that's not a fan-out — it's a chain
with a hidden barrier. If a single tail node depends on all of them
that's a diamond. A pure fan-out has no shared tail.

**Worked example.** A common shape: an interface lands as the parent,
and several consumers fan out from it. Each consumer can ship on its
own; no synthesis pulls them back together.

**Tool call.**

```
bullseye_subdivide(
  parent="P",
  mode="aggregate",
  children=[
    {name: "Consumer A", depends_on: []},
    {name: "Consumer B", depends_on: []},
    {name: "Consumer C", depends_on: []},
  ],
)
```

`aggregate` mode keeps the parent as a converging umbrella that
retires once all the children retire. Use `add` mode instead if the
parent's own deliverable should retire independently of the children.

---

## chain

```
[A] → [B] → [C] → [D]
```

**Heuristic.** Pure sequential dependency: each step needs the prior
one done. No parallelism, no convergence.

**When to use.** When the work genuinely is one-at-a-time, and naming
the sequence in the graph (rather than burying it in a single node's
acceptance prose) clarifies what's next. Chains are also the right
shape when each intermediate state is itself a meaningful
intermediate deliverable — something the agent or user might want to
showcase before moving on.

**When NOT to use.** Don't manufacture a chain from prose just for
the sake of decomposition. "Write the code, then run the tests, then
commit" is one node's acceptance, not a chain — the steps have no
independent meaning. Chains earn their place when intermediates
deserve to be addressable on their own.

**Fake-edge test.** Before wiring `B depends_on A`, ask: does B’s
acceptance or context actually **consume** A’s outcome? If not, the
edge is sequential prose only — drop it or keep A and B as one node.
See agents-guide “Graph discipline” and
`docs/analysis/graph-engineering-evaluation-2026-08.md` (🎯T56).

**Optional contracts (convention, not schema).** Free-text lines in
acceptance or context:

- `produces: <artifact or outcome the next node needs>`
- `consumes: <what this node needs from predecessors>`

These make real edges legible without new fields or a `kind`
discriminator. Prefer them on diamonds and multi-predecessor merges.

**Merge completeness (🎯T59).** A multi-predecessor merge (diamond
reduce, fan-in) can look “almost green” while one pred is still open.
Bullseye does not invent a new edge type: validate/summary **advisory**
hygiene reports expected vs terminal predecessor counts on nodes with
2+ `depends_on` when fan-in is partial (mixed terminal/active deps).
Vocabulary and agents-guide: *merge completeness*. Source:
`docs/analysis/graph-engineering-evaluation-2026-08.md` §3.3.

**Fake edge (🎯T60).** A `depends_on` is real only when the dependent
**consumes** the predecessor’s outcome. validate/summary **advisory**
hygiene (`fake_edge_warnings` inside `graph_hygiene_warnings`) flags
edges where B’s acceptance/context does not mention A’s id, a
significant name token, or a significant acceptance token — typed order
only. Vocabulary: *fake edge*. Source:
`docs/analysis/graph-engineering-evaluation-2026-08.md` §3.1. See
agents-guide “Graph discipline” for the full heuristic.

**Tool call.** Build chains incrementally with `bullseye_put`,
threading `depends_on` as you go:

```
bullseye_put(name="A")                                       → assigns T1
bullseye_put(name="B", depends_on=["T1"])                    → assigns T2
bullseye_put(name="C", depends_on=["T2"])                    → assigns T3
```

To **insert a step into an existing chain** (e.g. add `B'` between
`B` and `C`), use the `blocks` sugar:

```
bullseye_put(name="B'", depends_on=["T2"], blocks=["T3"])
```

This files B', has it depend on B, and re-points C to depend on B'
in one call — no separate patch needed on C.

---

## choke-point

```
[A]
   ↘
[B] → [G] → [D]
   ↗         ↘
[C]           [E]
              ↘
               [F]
```

**Heuristic.** Many parallel branches converge through a single
node; many more fan out from it. Think of `G` as the gate where
everything has to roll up before anything downstream gets unblocked.
Reach for it when a piece of work is the **single hinge** between an
upstream phase and a downstream phase.

**When to use.** A natural choke-point exists when several upstreams
all contribute to a state, *and* several downstreams all consume it.
The decision to file it as one node is justified when the rolling-up
work is itself a thing — a release, a migration cut-over, a shared
contract being signed off.

**When NOT to use.** If the upstreams all feed a node that has only
one downstream consumer, it's a fan-in (the dual of fan-out), not a
choke-point. The "choke" comes from the bidirectional squeeze.

**Tool call — hoisting an existing choke-point.** If A, B, C already
exist and you realise they all need to feed a new prerequisite G that
then blocks D, E, F, file G once with `blocks` listing every
downstream consumer, and have G `depends_on` every upstream
contributor:

```
bullseye_put(
  name="G",
  depends_on=["T_A", "T_B", "T_C"],
  blocks=["T_D", "T_E", "T_F"],
)
```

This is a graph-shaping move — one call rewires six existing nodes.

---

## spike-then-decide

```
[spike] → [option A]
       ↘
         [option B]
       ↘
         [option C]
```

**Heuristic.** A research / prototyping target gates a fan-out of
parallel implementation options. The spike's deliverable is not the
production thing — it's the *decision* that picks which downstream
branch to actually pursue.

**When to use.** The signature is that the spike retires when the
agent or user has enough information to choose, not when production
code lands. The fan-out below is a list of mutually exclusive
options. After the decision, the unchosen options should be retired
or set aside (with a reason: "rejected after spike T_spike").

**When NOT to use.** If after the spike all the downstream branches
will get built, this isn't spike-then-decide — it's a plain fan-out
gated by the spike. The "decide" in the name is the load-bearing
part.

**Tool call.**

```
bullseye_subdivide(
  parent="design-X",
  mode="aggregate",
  children=[
    {name: "Spike: prototype both approaches"},
  ],
)
bullseye_put(name="Option A", depends_on=["spike-id"])
bullseye_put(name="Option B", depends_on=["spike-id"])
bullseye_put(name="Option C", depends_on=["spike-id"])
```

After the spike retires and an option is chosen, set the others
aside:

```
bullseye_set_aside(id="option-b-id", reason="rejected — spike showed
  cost > benefit. See achieved 🎯T_spike.")
```

---

## contract-first

```
[define contract]
      ↓
   [impl A]   [impl B]   [impl C]
      ↓          ↓          ↓
   [test A]   [test B]   [test C]
      ↓          ↓          ↓
              [integrate]
```

**Heuristic.** Define an API, schema, or protocol up front; then
multiple implementations and their tests fan out in parallel against
the contract; then a single integration node converges them. This is
how you parallelise across implementers (or sessions, or sub-agents)
without painful merge conflicts at the contract level.

**When to use.** Two or more places will implement the same
interface, and getting them to agree later is expensive. Reach for
it whenever a parallel fan-out depends on a shared shape — schemas,
protocols, message formats, function signatures, plugin interfaces.

**When NOT to use.** If only one implementation will exist, a contract
node is overhead. Contract-first earns its place when at least two
implementations will need to slot into the same shape.

**Tool call.** Combine contract-as-node + `subdivide` for the
implementation fan-out, then a `bullseye_put` with `depends_on`
listing every implementation for the integration node:

```
bullseye_put(name="Define contract: X")  → T_contract

bullseye_subdivide(
  parent=T_contract,
  mode="add",
  children=[
    {name: "Impl A", depends_on: []},
    {name: "Impl B", depends_on: []},
    {name: "Impl C", depends_on: []},
  ],
)

bullseye_put(
  name="Integrate against contract X",
  depends_on=[T_contract_1, T_contract_2, T_contract_3],
)
```

---

## migration

```
[prepare] → [cut-over]
         ↘             ↘
          [keep old running]   [verify new]
                               ↓
                            [remove old]
```

**Heuristic.** When replacing a working system, the prepare phase
sets up the new thing; the cut-over phase routes traffic / callers /
data to it; the old system keeps running in parallel until the new
one is verified; only then does the old system get removed.

**When to use.** Reach for it whenever something already exists and
must keep working through the switchover. Schema migrations, API
rewrites, infra moves, daemon replacements. The "keep old running"
node is the load-bearing part — it forces an explicit acknowledgment
that the old system is still load-bearing during the transition.

**When NOT to use.** Pure new-feature work isn't a migration — there
is no old system to keep running. Use a diamond or fan-out instead.

**Tool call.** Files as five or so explicit nodes; the structure is
worth the verbosity because skipping a step here is how migrations
break in production.

```
bullseye_put(name="Prepare new system")          → T_prep
bullseye_put(name="Cut over to new",
             depends_on=[T_prep])                → T_cut
bullseye_put(name="Keep old running through cutover",
             depends_on=[T_prep])                → T_keep
bullseye_put(name="Verify new in production",
             depends_on=[T_cut, T_keep])         → T_verify
bullseye_put(name="Remove old system",
             depends_on=[T_verify])              → T_remove
```

The `keep_old_running` node is what the agent will be tempted to
omit. Don't.

---

## Agent discipline: interrogate, then file

Before committing to a single-node target whose acceptance reads as
multi-phase prose ("do X, then Y, then check Z"), run the catalogue
against it. Ask:

1. **Is there a fork inside the prose?** Two independent things that
   could run in parallel after one prior step? → diamond.
2. **Is there one prerequisite enabling many independents?** → fan-out.
3. **Is there one node that everything has to roll up through?** →
   choke-point.
4. **Is one step a "decide" that picks among options?** →
   spike-then-decide.
5. **Is there an interface that multiple things will hang off?** →
   contract-first.
6. **Is there an old version that must stay alive during the switch?**
   → migration.
7. **Is it actually sequential, and do the intermediates have
   independent meaning?** → chain.
8. **Are the steps a single coherent piece of work whose
   intermediates aren't separately addressable?** → leave it as one
   node.

The mistake to avoid: filing a single node whose acceptance criteria
encode a subgraph in prose. The graph is the artifact — if the shape
matters, draw it in the graph.

For the global directive that codifies this rule, see
`~/.claude/convergence.md`.
