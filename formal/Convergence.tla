---- MODULE Convergence ----
(***************************************************************************
 * Universal convergence properties, parameterized by an engine.
 *
 * Every convergence engine -- sqlpipe replica sync, a crosshair strategy,
 * a bullseye target, a mnemo reconciler, a build system, a Kubernetes
 * controller -- has the same spine:
 *
 *     desired(sources)      -> State
 *     observe()             -> State
 *     diff(actual, desired) -> Gap
 *     repair(Gap)           -> Effects
 *     quiescent?()          -> bool
 *
 * This module states once what it means for such an engine to be
 * convergent.  An engine INSTANCEs it and supplies:
 *
 *   CONSTANT operators -- the engine's plug-in, the part No Free Lunch says
 *   cannot be shared:
 *     Desired(sources)          the declared desired state
 *     Observe(actual)           the observable actual state
 *     Diff(observed, desired)   the gap
 *     NoGap                     the Gap value meaning "nothing to do"
 *     GapSize(gap)              a natural-number measure of the gap;
 *                               0 iff the gap is NoGap
 *     Sound(observed, desired)  the safety predicate that must never break,
 *                               even in the middle of a pass
 *     Settled(sources)          "the environment has stopped": the
 *                               hypothesis under which the engine promises
 *                               to converge
 *     NoEffects                 the Effects value meaning "nothing emitted"
 *
 *   VARIABLES -- a refinement mapping from the engine's own state:
 *     sources   environment-owned inputs to Desired
 *     actual    everything the engine acts on, including its own
 *               machinery (cursors, in-flight messages, caches)
 *     actor     who took the last step: Engine or Env
 *     fx        the Effects the last step emitted (NoEffects if none)
 *
 * The engine keeps its own Init, Next and fairness.  Partial passes are
 * modelled by the engine's Next (interruption, message loss, preemption);
 * every property below quantifies over every reachable state, so it holds
 * mid-pass or not at all.
 *
 * Contract prose: ~/think/convergence-spec/contract.md (parameterized
 * module home). This copy is the instance dependency; keep it in lockstep
 * with that file when the theorem changes.
 ***************************************************************************)
EXTENDS Naturals

CONSTANTS Desired(_), Observe(_), Diff(_, _), NoGap, GapSize(_),
          Sound(_, _), Settled(_), NoEffects

VARIABLES sources, actual, actor, fx

vars == <<sources, actual, actor, fx>>

Engine == "engine"
Env    == "env"

desired   == Desired(sources)
observed  == Observe(actual)
gap       == Diff(observed, desired)
Quiescent == gap = NoGap

\* Classification of the step just taken.
EngineStep == actor' = Engine
EnvStep    == actor' = Env

----
(* Contract well-formedness: checked, not assumed. *)

TypeOK ==
    /\ actor \in {Engine, Env}
    /\ GapSize(gap) \in Nat
    /\ (Quiescent <=> GapSize(gap) = 0)

\* Settled must mean what it says: once settled, sources never change again.
SettledIsStable == [][Settled(sources) => UNCHANGED sources]_vars

\* The engine never edits its own inputs.  A repair that closes the gap by
\* moving the desired state is a Goodhart engine, not a convergence engine.
GoalpostsFixed == [][EngineStep => UNCHANGED sources]_vars

----
(* The four universal properties. *)

\* 1. Soundness preservation under partial passes.  The safety predicate
\*    holds in every reachable state, including every intermediate state of
\*    an interrupted pass.
Soundness == Sound(observed, desired)

\* 2. Idempotent re-entry.  An engine step never widens the gap.
\*    Re-entering a pass from any intermediate state -- redoing work that
\*    was already done -- is at worst a no-op.  Progress is a ratchet.
IdempotentReentry == [][EngineStep => GapSize(gap') <= GapSize(gap)]_vars

\* 3. Self-healing.  Once the environment settles, the gap closes ...
SelfHealing == Settled(sources) ~> Quiescent

\*    ... and stays closed.
Stabilizes == Settled(sources) ~> []Quiescent

\* 4. Quiescence.  Once the environment settles, the engine falls silent:
\*    from some point on no step emits effects.  At the fixed point a pass
\*    is a no-op.
Quiescence == Settled(sources) ~> [](fx = NoEffects)

\* Synchronous quiescence: an engine step taken AT the fixed point emits
\* nothing and changes nothing observable.  Only engines whose repair acts
\* on a current observation can meet this; an engine with asynchronous
\* observation (a probe in flight) legitimately fails it once per stale
\* observation.  Offered as an opt-in strengthening, not as universal.
QuiescentNoOp ==
    [][EngineStep /\ Quiescent => fx' = NoEffects /\ UNCHANGED observed]_vars

====
