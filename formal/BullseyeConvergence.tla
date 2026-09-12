---- MODULE BullseyeConvergence ----
(***************************************************************************
 * bullseye's target graph as an instance of the parameterized Convergence
 * module -- the second, structurally different instantiation.
 *
 * sqlpipe is a replication loop: a flat row set pushed across a lossy
 * channel until two copies agree.  bullseye is not a replication loop.  It
 * is a DAG whose *nodes change state* and whose nodes can *spawn children*:
 * an agent picks an unblocked target off the frontier, works it, and either
 * achieves it or discovers it was composite and subdivides it into children
 * that now gate it.  The convergence claim is that the frontier drains.
 *
 * Lives in this repo (formal/) and runs from `make bullseye` / `make tla`.
 * Mapping of the real system (this repo, schema v5) onto this model:
 *
 *   real                        model
 *   ------------------------    -------------------------------------------
 *   Status::Identified          status[t] = "identified"   (tracked)
 *   Status::Converging          status[t] = "converging"   (tracked, claimed)
 *   Status::Achieved            status[t] = "achieved"
 *   Status::SetAside            status[t] = "setaside"
 *   blocked                     derived: some child of t is not terminal
 *                               (schema v5 has no Blocked status; blocking
 *                               is an edge property, `depends_on`)
 *   deferred / postponed_until  t \in postponed
 *   reopened                    Reopen: achieved -> identified, cascading
 *                               up the ancestor chain
 *   bullseye_subdivide          AddChild: parent gains a child that gates it
 *   frontier                    active, not postponed, all children terminal
 *
 * The engine (the agent) holds one claim at a time; a claim can be lost at
 * any moment (crash, preemption, context loss).  That is this engine's
 * partial pass, the counterpart of sqlpipe's lost probe or lost response:
 * the target is left in "converging" and the next pass must re-observe the
 * ledger and pick up from whatever it finds.
 *
 * The gap measure is the interesting part.  Naively "count the open
 * targets" is not a ratchet: subdivision *adds* nodes, so an engine step
 * would widen the gap and IdempotentReentry would fail.  Subdivision is
 * refinement, not regression, so the measure gives every target a capacity
 *
 *     Cap(d) = (MaxChildren + 1) ^ (MaxDepth - d)
 *
 * and a subdivided parent's own residual weight is its capacity minus the
 * capacity it has handed to its children.  Spawning a child moves weight
 * from the parent to the child and leaves the total unchanged; achieving a
 * target removes its weight.  The parent's residual stays >= 1 because it
 * can hand out at most MaxChildren shares of Cap(d+1) out of (MaxChildren+1).
 * That is the whole content of "subdivision must not increase remaining
 * work", and the InflatingSplit mutation below breaks exactly it.
 *
 * Refinement mapping:
 *   sources = [roots, stopped]                 owner-declared intent
 *   actual  = the ledger + the agent's claim and postponement machinery
 *   Desired = the declared roots; Observe = the ledger without the agent's
 *   private scheduling state; gap = open targets reachable from a root.
 ***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    MaxTargets,   \* size of the target id pool
    MaxRoots,     \* how many root targets the owner may file
    MaxDepth,     \* deepest subdivision level
    MaxChildren,  \* branching factor of a subdivision
    Fault         \* subset of {"EagerRollup", "InflatingSplit", "DeleteTarget"}

\* Deliberate mutations, each breaking one universal property.
EagerRollup    == "EagerRollup"    \in Fault  \* achieve a parent with open children
InflatingSplit == "InflatingSplit" \in Fault  \* children spawn at the parent's own depth
DeleteTarget   == "DeleteTarget"   \in Fault  \* close the gap by deleting the target

VARIABLES
    roots,      \* ids the owner has declared as roots  (source)
    stopped,    \* the owner has stopped filing/reopening (source)
    status,     \* [Targets -> Statuses]
    parent,     \* [Targets -> Targets \cup {NoTarget}]
    rootOf,     \* [Targets -> Targets \cup {NoTarget}]  root of each target
    depth,      \* [Targets -> 0..MaxDepth]
    claim,      \* the one target the agent is working, or NoTarget
    postponed,  \* deferred targets, held off the frontier
    actor,      \* history: who took the last step
    fx          \* history: what the last step emitted

vars == <<roots, stopped, status, parent, rootOf, depth,
          claim, postponed, actor, fx>>

Targets    == 1..MaxTargets
NoTarget   == 0
Undeclared == "none"
OpenSt     == {"identified", "converging"}
TerminalSt == {"achieved", "setaside"}
Statuses   == {Undeclared} \cup OpenSt \cup TerminalSt

NoFx   == "none"
Engine == "engine"
Env    == "env"

Declared      == {t \in Targets : status[t] # Undeclared}
Free          == {t \in Targets : status[t] = Undeclared}
MinOf(S)      == CHOOSE x \in S : \A y \in S : x <= y
ChildrenOf(t) == {c \in Declared : parent[c] = t}

\* A parent may be closed only when every child has reached a terminal
\* disposition.  EagerRollup drops the check.
RollupOK(t) == EagerRollup \/ (\A c \in ChildrenOf(t) : status[c] \in TerminalSt)

RECURSIVE AncestorsOf(_)
AncestorsOf(t) == IF parent[t] = NoTarget
                  THEN {}
                  ELSE {parent[t]} \cup AncestorsOf(parent[t])

Init ==
    /\ roots = {}
    /\ stopped = FALSE
    /\ status = [t \in Targets |-> Undeclared]
    /\ parent = [t \in Targets |-> NoTarget]
    /\ rootOf = [t \in Targets |-> NoTarget]
    /\ depth = [t \in Targets |-> 0]
    /\ claim = NoTarget
    /\ postponed = {}
    /\ actor = Env
    /\ fx = NoFx

----
(* Environment steps: the owner files, defers, abandons and reopens; the
   world interrupts the agent. *)

\* The owner files a new root target.
FileRoot ==
    /\ ~stopped
    /\ Cardinality(roots) < MaxRoots
    /\ Free # {}
    /\ LET c == MinOf(Free) IN
        /\ roots' = roots \cup {c}
        /\ status' = [status EXCEPT ![c] = "identified"]
        /\ rootOf' = [rootOf EXCEPT ![c] = c]
        /\ depth' = [depth EXCEPT ![c] = 0]
    /\ actor' = Env
    /\ fx' = NoFx
    /\ UNCHANGED <<stopped, parent, claim, postponed>>

\* The owner stops filing and reopening: the convergence hypothesis.
Stop ==
    /\ ~stopped
    /\ stopped' = TRUE
    /\ actor' = Env
    /\ fx' = NoFx
    /\ UNCHANGED <<roots, status, parent, rootOf, depth, claim, postponed>>

\* The owner decides not to pursue a leaf target.  Terminal but not
\* achieved; it unblocks its dependents just like an achieved target.
\* Deferral is status-scoped in the real schema, so a target that reaches a
\* terminal disposition drops off the deferral list: a terminal target
\* carrying postponement residue is an invalid ledger (NoTerminalResidue).
SetAside(t) ==
    /\ ~stopped
    /\ status[t] \in OpenSt
    /\ ChildrenOf(t) = {}
    /\ status' = [status EXCEPT ![t] = "setaside"]
    /\ claim' = IF claim = t THEN NoTarget ELSE claim
    /\ postponed' = postponed \ {t}
    /\ actor' = Env
    /\ fx' = NoFx
    /\ UNCHANGED <<roots, stopped, parent, rootOf, depth>>

\* The symptom came back: an achieved target reopens, and so does every
\* achieved ancestor that had rolled up over it.  Any in-flight pass is
\* invalidated.
Reopen(t) ==
    /\ ~stopped
    /\ status[t] = "achieved"
    /\ LET reopened == {t} \cup {u \in AncestorsOf(t) : status[u] = "achieved"} IN
        status' = [u \in Targets |->
                      IF u \in reopened THEN "identified" ELSE status[u]]
    /\ claim' = NoTarget
    /\ actor' = Env
    /\ fx' = NoFx
    /\ UNCHANGED <<roots, stopped, parent, rootOf, depth, postponed>>

\* The owner defers a target off the frontier.
Postpone(t) ==
    /\ ~stopped
    /\ status[t] \in OpenSt
    /\ t \notin postponed
    /\ postponed' = postponed \cup {t}
    /\ claim' = IF claim = t THEN NoTarget ELSE claim
    /\ actor' = Env
    /\ fx' = NoFx
    /\ UNCHANGED <<roots, stopped, status, parent, rootOf, depth>>

\* Partial pass: the agent loses its claim mid-work (crash, preemption,
\* context loss).  The target stays "converging"; nothing else is undone.
LoseClaim ==
    /\ claim # NoTarget
    /\ claim' = NoTarget
    /\ actor' = Env
    /\ fx' = NoFx
    /\ UNCHANGED <<roots, stopped, status, parent, rootOf, depth, postponed>>

----
(* Engine steps: the agent reads the ledger, claims an unblocked target,
   subdivides it or achieves it, wakes deferred work, and otherwise polls. *)

\* Claim a frontier target: active, not deferred, every child terminal.
Claim(t) ==
    /\ claim = NoTarget
    /\ status[t] \in OpenSt
    /\ t \notin postponed
    /\ RollupOK(t)
    /\ status' = [status EXCEPT ![t] = "converging"]
    /\ claim' = t
    /\ actor' = Engine
    /\ fx' = "track"
    /\ UNCHANGED <<roots, stopped, parent, rootOf, depth, postponed>>

\* The target turned out to be composite: spawn a child that gates it and
\* release the claim.  InflatingSplit spawns the child at the parent's own
\* depth, so the child claims a full share of capacity the parent never
\* handed over.
AddChild(t) ==
    /\ claim = t
    /\ status[t] = "converging"
    /\ depth[t] < MaxDepth
    /\ Cardinality(ChildrenOf(t)) < MaxChildren
    /\ Free # {}
    /\ LET c == MinOf(Free) IN
        /\ status' = [status EXCEPT ![c] = "identified"]
        /\ parent' = [parent EXCEPT ![c] = t]
        /\ rootOf' = [rootOf EXCEPT ![c] = rootOf[t]]
        /\ depth' = [depth EXCEPT ![c] = IF InflatingSplit
                                         THEN depth[t]
                                         ELSE depth[t] + 1]
    /\ claim' = NoTarget
    /\ actor' = Engine
    /\ fx' = "subdivide"
    /\ UNCHANGED <<roots, stopped, postponed>>

\* Close the claimed target.
Achieve(t) ==
    /\ claim = t
    /\ status[t] = "converging"
    /\ RollupOK(t)
    /\ status' = [status EXCEPT ![t] = "achieved"]
    /\ claim' = NoTarget
    /\ actor' = Engine
    /\ fx' = "achieve"
    /\ UNCHANGED <<roots, stopped, parent, rootOf, depth, postponed>>

\* Bring a deferred target back onto the frontier.
Wake(t) ==
    /\ t \in postponed
    /\ postponed' = postponed \ {t}
    /\ actor' = Engine
    /\ fx' = "wake"
    /\ UNCHANGED <<roots, stopped, status, parent, rootOf, depth, claim>>

\* MUTATION: close the gap by deleting the target instead of achieving it.
DeleteRoot(t) ==
    /\ DeleteTarget
    /\ t \in roots
    /\ status[t] \in OpenSt
    /\ roots' = roots \ {t}
    /\ actor' = Engine
    /\ fx' = "delete"
    /\ UNCHANGED <<stopped, status, parent, rootOf, depth, claim, postponed>>

\* A pass that found nothing to do.  Reading the ledger is an observation,
\* not an effect.
Poll ==
    /\ actor' = Engine
    /\ fx' = NoFx
    /\ UNCHANGED <<roots, stopped, status, parent, rootOf, depth,
                   claim, postponed>>

----

Next ==
    \/ FileRoot
    \/ Stop
    \/ LoseClaim
    \/ Poll
    \/ \E t \in Targets :
        \/ SetAside(t) \/ Reopen(t) \/ Postpone(t)
        \/ Claim(t) \/ AddChild(t) \/ Achieve(t) \/ Wake(t) \/ DeleteRoot(t)

(* Fairness.  See contract.md, "Fairness obligation".

   Claim(t) and Achieve(t) need STRONG fairness, per target.  Both are
   intermittently disabled through no fault of the engine: Claim(t) needs
   the single claim slot free, and Achieve(t) needs the claim still held,
   which LoseClaim can revoke at any moment.  Under weak fairness the
   adversary interleaves Claim(t)/LoseClaim forever: neither action is ever
   *continuously* enabled, so weak fairness is satisfied vacuously and
   nothing is ever achieved.  SpecWeakFairness below is that behaviour, and
   TLC exhibits it.

   Wake(t) and Poll need only weak fairness: once enabled they stay enabled
   until taken.

   AddChild needs no fairness at all: subdivision is the optional
   optimisation, and soundness must not depend on it. *)

FairPerTarget ==
    /\ \A t \in Targets : SF_vars(Claim(t))
    /\ \A t \in Targets : SF_vars(Achieve(t))
    /\ \A t \in Targets : WF_vars(Wake(t))
    /\ WF_vars(Poll)

FairWeakOnly ==
    /\ \A t \in Targets : WF_vars(Claim(t))
    /\ \A t \in Targets : WF_vars(Achieve(t))
    /\ \A t \in Targets : WF_vars(Wake(t))
    /\ WF_vars(Poll)

FairDisjunctive ==
    /\ SF_vars(\E t \in Targets : Claim(t))
    /\ SF_vars(\E t \in Targets : Achieve(t))
    /\ WF_vars(\E t \in Targets : Wake(t))
    /\ WF_vars(Poll)

Spec                    == Init /\ [][Next]_vars /\ FairPerTarget
SpecWeakFairness        == Init /\ [][Next]_vars /\ FairWeakOnly
SpecDisjunctiveFairness == Init /\ [][Next]_vars /\ FairDisjunctive

----
(* Instantiation of the shared module. *)

Desired(s) == s.roots

\* The agent's private scheduling state (its claim, its deferral list) is
\* machinery, not ledger: it is in `actual` but not observable.
Observe(a) == [status |-> a.status, parent |-> a.parent,
               rootOf |-> a.rootOf, depth  |-> a.depth]

ObsDeclared(o)     == {t \in Targets : o.status[t] # Undeclared}
ObsChildren(o, t)  == {c \in ObsDeclared(o) : o.parent[c] = t}

\* Total work a target at depth d may ever stand for.  A subdivision hands
\* one share of Cap(d+1) to each child, out of the (MaxChildren+1) shares
\* the parent holds, so the parent's residual stays >= Cap(d+1) >= 1.
Cap(d) == (MaxChildren + 1) ^ (MaxDepth - d)

Weight(o, t) ==
    IF o.depth[t] >= MaxDepth
    THEN 1
    ELSE Cap(o.depth[t]) - Cardinality(ObsChildren(o, t)) * Cap(o.depth[t] + 1)

Diff(o, d) ==
    {[id |-> t, w |-> Weight(o, t)] :
        t \in {u \in ObsDeclared(o) : o.status[u] \in OpenSt /\ o.rootOf[u] \in d}}

RECURSIVE SumW(_)
SumW(S) == IF S = {} THEN 0
           ELSE LET x == CHOOSE y \in S : TRUE IN x.w + SumW(S \ {x})

GapSize(g) == SumW(g)

\* Safety: a target is never achieved over open children.  A rollup that
\* runs ahead of its children is a lie in the ledger, and it is a lie that
\* survives -- the parent stays achieved while the work below it is open.
Sound(o, d) ==
    \A t \in ObsDeclared(o) :
        o.status[t] = "achieved" =>
            \A c \in ObsChildren(o, t) : o.status[c] \in TerminalSt

Settled(s) == s.stopped

C == INSTANCE Convergence WITH
    sources   <- [roots |-> roots, stopped |-> stopped],
    actual    <- [status |-> status, parent |-> parent, rootOf |-> rootOf,
                  depth |-> depth, claim |-> claim, postponed |-> postponed],
    NoGap     <- {},
    NoEffects <- NoFx

TypeOK            == C!TypeOK
SettledIsStable   == C!SettledIsStable
GoalpostsFixed    == C!GoalpostsFixed
Soundness         == C!Soundness
IdempotentReentry == C!IdempotentReentry
SelfHealing       == C!SelfHealing
Stabilizes        == C!Stabilizes
Quiescence        == C!Quiescence
QuiescentNoOp     == C!QuiescentNoOp

----
(* bullseye's own theorem, kept to show it is what the instance proves. *)

\* Every declared target descends from a root the owner actually declared:
\* the engine never manufactures work of its own.
NoOrphanWork == \A t \in Declared : rootOf[t] \in roots

\* Deferral is status-scoped: a terminal target never carries postponement.
NoTerminalResidue == \A t \in postponed : status[t] \in OpenSt

\* The frontier drains: once the owner stops, no target stays open.
FrontierDrains == stopped ~> (\A t \in Targets : status[t] \notin OpenSt)

LocalTypeOK ==
    /\ status \in [Targets -> Statuses]
    /\ claim \in Targets \cup {NoTarget}
    /\ postponed \subseteq Targets
    /\ roots \subseteq Targets
    /\ \A t \in Targets : depth[t] \in 0..MaxDepth

====
