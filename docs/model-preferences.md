# Model preferences (🎯T107)

Decision, 2026-11-10: use a first-class optional `models` block. The owner
approved separate execution and review preferences, the three capability
classes below, optional effort, per-provider pins, and caller-pin precedence.

```yaml
models:
  execute:
    class: frontier
    effort: high
    pins:
      claude: opus
  review:
    class: frontier
    pins:
      claude: fable
```

`execute` and `review` are independently optional; at least one is required
when the block is present. No block means no preference, including no
implicit review requirement. Updates replace the whole block; an omitted
role is removed. Omit `models` in a patch to leave it alone, or use
`clear: [models]` to remove it. Unknown fields, classes and efforts are errors.

## Capability and identity

The closed, ascending capability vocabulary is:

| Class | Intended work |
|---|---|
| `economy` | Mechanical changes and execution of a fully decided procedure |
| `standard` | Routine implementation, test harnesses and integration from a decided design |
| `frontier` | Difficult reasoning, numerics, architectural judgment and subtle-error review |

These are planner judgments about minimum capability, never benchmark claims
about a named product. A dispatcher owns the mapping to its currently
available models. Bullseye has no model catalogue and never routes, spawns,
or runs a model, following the `strategy` precedent from 🎯T15.2.

Concrete model ids churn; putting them in the class vocabulary makes every
ledger track provider releases. `pins` instead maps provider keys to model
ids or dispatcher-supported aliases. Keys use lowercase letters, digits,
hyphens and underscores. Values must be non-empty trimmed strings without
control characters. Unknown providers remain legal: Bullseye cannot know a
dispatcher's inventory. Pins for different providers are alternatives, not
instructions to switch providers. Unavailable pins must produce a visible
dispatcher decision or refusal; never silently invent a replacement id.

`effort` is optional: `low`, `medium`, `high`, `max`. No value means use the
dispatcher's default. The dispatcher translates or explicitly reports an
unsupported effort; this vocabulary is not a provider's API parameter list.

## Consumer contract

Resolve each role separately, in this order:

1. An explicit caller/owner model pin wins, preserving Jevons' `PickMintModel` contract.
2. The target's pin for the already selected provider wins over the class mapping.
3. The target's class selects a model from the dispatcher's capability mapping.
4. With no target preference, use the dispatcher's existing `TaskRoute` or default.

Report the selection source and any override. A reviewer must be a different
session from the author; equal classes are valid and do not prove or disprove
independence. The example deliberately places both Fable and Opus in
`frontier` while retaining their distinct family preferences as pins. A
review preference is not a dependency or a new Bullseye achievement gate.

Jevons 🎯T1395 is the initial consumer: its worker-mint path should read the structured
record before applying `TaskRoute{Model, Models, Class}`. A separately tracked
Jevons target owns that integration; Bullseye's completion does not claim that
Jevons already consumes the field.

## Surfaces and filtering

```sh
bullseye commit --op track --id T1 \
  --models '{execute: {class: standard, effort: high}}' --adopt-model-preferences
bullseye apply --id T1 --set 'models={review: {class: frontier}}'
bullseye query --view target --id T1 --format json
bullseye query --view frontier --model-class standard --format json
bullseye apply --id T1 --set 'clear=[models]'
```

MCP uses the same handlers: `bullseye_commit` takes a YAML/JSON string in
`models`, `bullseye_apply` takes a structured `models` block inside `fragment`,
and `bullseye_query` takes `model_class` and `format`. In both surfaces,
`format=json` emits a JSON document, carried as MCP text content. `target`
returns `{file,id,target,issues}`; `frontier` returns
`{file,targets:[{id,fanout,target}],issues,excluded}` in frontier rank order.
Each `target` is the complete typed record, so consumers read `target.models`
without parsing tags or human prose. Read the diagnostic fields: an invalid
target can be excluded by the existing degraded-frontier rules.

`model_class` is legal only for the frontier and describes the available
executor. `standard` includes unspecified, `economy` and `standard` execution
preferences. `frontier` includes every otherwise-workable target. A review-only
block leaves execution unspecified. Filtering happens after dependency,
ownership and postponement checks; it never makes blocked work ready and
does not change graph edges or the unfiltered frontier. Pins and effort do
not affect this filter: it is a capability view, not a provider availability
or routing decision. Text target/frontier/context/summary views show preferences too.

## Compatibility and migration

Absent blocks remain omitted. Ledgers without them retain the content-derived
v5/v6 stamp; a ledger containing preferences requires schema v7 so an older
writer refuses instead of dropping them. The first adoption is refused until
`adopt_model_preferences: true` is supplied on an apply request or commit
(CLI: `--adopt-model-preferences`). Upgrade every reader/writer first.
Clearing the final block restores the lowest representable stamp.

```sh
bullseye commit --op migrate-model-tags --cwd /path/to/repo --adopt-model-preferences
```

The explicit, atomic format migration converts `model:haiku` → execution
`economy`, `model:sonnet` → `standard`, and `model:opus`/`model:fable` →
`frontier`; `review:` uses the same mapping. Every original family is retained
as `pins: {claude: <family>}`. These pins are family aliases, not guessed current
model ids. Jevons' ladder currently contains versioned names that differ from
the planner's October family descriptions; copying those ids would change
intent. The consumer must resolve the aliases against its own catalogue.

Only converted tags move. Other tags and all other fields, including prose,
achievement dates, attestations and dependencies, are preserved. This is an
explicit representation migration of historical records, not a reopen and
re-achieve cycle. Unknown families, conflicting preferences, or an invalid
ledger refuse the whole write. Repeating a successful migration changes no
targets. Normal preference edits on achieved targets remain forbidden.

The frozen Go-fork tag inventory in `tests/fixtures/model-tags-golang-go.json`
records the source hash and all 43 annotated targets found on 2026-11-10
(all now achieved). The original target description's 45-target planning
subgraph is historical; the migration gate derives coverage from that frozen
inventory, not an implementation-maintained count. Verification also migrates
a full copy of the current real ledger and compares every unaffected field.
The live ledger stays readable by the installed v6 consumers until a compatible
release is installed and adoption is deliberately performed.

## Verification boundary

The gate runs omission/round-trip, closed vocabulary, refusal/atomicity,
immutability, minimum-class monotonicity, frozen-inventory migration, and
real CLI + HTTP MCP persistence tests. The latter uses an isolated daemon
and scratch ledger and is the product journey for this storage/read contract.
It does not launch a provider: Bullseye has no execution contract here.
Actual model selection and reviewer-session independence belong to the
consumer's live journey. No provider process contract is changed by this work.
