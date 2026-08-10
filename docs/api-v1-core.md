# Bullseye core API contract (🎯T45)

Pre-1.0 freeze of the **intent ledger** surface. Agents should live on
these four tools. Everything else is either a compatibility shim or an
**extended** (L2) integration.

## Product posture

Bullseye is a shared, concurrent-safe **ledger of desired states**.
Agents plan; bullseye records, unblocks, and hardens claims.

- **User intent overrides the frontier.** The graph proposes; chat
  disposes.
- **Commit at boundaries** (discover lasting work, finish it, park it,
  reopen a bad claim) — not on every micro-step.
- **Do not treat bullseye as a planning gate** for one-shot tasks that
  already have a clear user objective.

## Core tools

| Tool | Role |
|------|------|
| `bullseye_open` | Discover / init / session snapshot |
| `bullseye_query` | All reads (`view=…`) |
| `bullseye_commit` | All ledger mutations (`op=…`) |
| `bullseye_plan_checks` | Emit sawmill check plan only (does not run checks) |

### `bullseye_open`

- If `bullseye.yaml` exists (in-repo or external via `discover_anywhere`):
  return context view. **Discovery never uses `default_location`.**
- If missing and `location` is set: init at that location, then context.
- If missing and `location` omitted but server `default_location` is set
  (`--default-location` / `BULLSEYE_DEFAULT_LOCATION`, 🎯T61): init using
  that default (create-path only), then context.
- If missing and neither per-call location nor server default: error
  `not_initialized` with the location prompt.

**Collision policy:** when both in-repo and external files exist,
`discover_anywhere` returns the **in-repo** path (v0.16).

### `bullseye_query` views

| `view` | Meaning |
|--------|---------|
| `context` | Session snapshot (default when only `cwd` is given) |
| `frontier` | Unblocked leaves (+ fanout ordering) |
| `target` | Full record for `id` |
| `list` | Filtered index (`filter`: active / achieved / set_aside / all) |
| `summary` | Groups, frontier, blocked, stale |
| `graph` | Mermaid dependency graph (default: whole **active** graph; optional `scope` / `nodes` / `seeds` / `expand` — 🎯T57) |
| `validate` | Schema / graph errors only |

#### Reads degrade; only `validate` hard-fails (🎯T64)

Every blocking validation error bullseye produces is attributed to a
single target, so an invalid target is a hole in the graph, not a wall
in front of it. Reads behave accordingly:

| `view` | On a validation error somewhere in the file |
|--------|----------------------------------------------|
| `frontier` | Answers. Invalid targets are dropped from the ready set; the errors and the excluded IDs are reported in a `## Validation errors (degraded read)` banner above the answer. |
| `context`, `summary` | Answer, same banner, frontier computed over the healthy graph. |
| `list` | Answers in full; each offending target is annotated inline with `INVALID: <message>`. |
| `target` | Answers in full; errors on *that* target are appended as an `INVALID:` note, errors elsewhere are irrelevant to it. |
| `validate` | **Hard-fails by design** — reporting these errors *is* its contract, so it reports them and nothing else. Degrading it would leave the ledger with no surface that tells the truth about its own health. |

`bullseye_convergence` and `bullseye_portfolio` degrade like `frontier`.

This is the fix for the 2026-08-10 jevons incident: one stale
`set_aside_reason` on one achieved target made *every* call against that
ledger — frontier included — return only the error, and no supported op
could repair it.

Two further guards close that incident:

- **Transition hygiene.** `schema::STATUS_SCOPED_FIELDS` is the single
  owner of "which fields are legal for status X". Every op that changes
  a status calls `Target::clear_illegal_status_scoped_fields`, so
  leaving a status drops what that status owned — `set_aside_reason`,
  `attestation`, the `achieved` date, `owned_by`, and the postpone
  pair — and a newly-added status-scoped field is covered by both
  validation and transition hygiene from one table row.
- **Load-time self-heal.** A file that *already* carries illegal residue
  is normalised in `store::load`, alongside the legacy
  `gates → depends_on` migration and for the same reason: a ledger only
  a hand edit can repair is a ledger the tools have failed. The repair
  persists on the next save; `bullseye_commit op=rehash` is the explicit
  load-and-save round trip that does nothing else. Dropped values are
  not relocated — they describe a disposition that no longer holds, and
  the previous content stays in the git history of `bullseye.yaml`.

#### `view=graph` parameters (🎯T57)

| Param | Default | Meaning |
|-------|---------|---------|
| `scope` | `active` | Status filter: `active` \| `all` \| `achieved` \| `set_aside` |
| `nodes` | — | Explicit node-ID list (naive subgraph; edges only when both ends selected) |
| `seeds` | — | Seed IDs for intelligent expansion |
| `expand` | — | From seeds: `ancestors` (walk `depends_on`), `descendants` (reverse-blocks), `children` / `parents` (ID hierarchy), `frontier` (1-hop frontier neighbors) |

Default with no filters = pre-T57 behaviour (full active graph, `depends_on` edges).
Disjoint components are allowed (no error solely for disconnected selection).
Response is fenced ` ```mermaid ` source suitable for chat renderers (e.g. jevons 🎯T59).
CLI twin: `bullseye query --view graph [--scope …] [--nodes A,B] [--seeds S] [--expand ancestors,…]`.

### `bullseye_commit` ops

| `op` | Meaning | Maps to (shim) |
|------|---------|----------------|
| `track` | Create or patch a target | `bullseye_put` |
| `block` | Inject this target into others' `depends_on` (`id` + `blocks`) | `put` with `blocks` |
| `split` | Subdivide parent into children | `bullseye_subdivide` |
| `achieve` | Retire as achieved (requires `attestation`) | `bullseye_retire` |
| `defer` | Set aside with reason | `bullseye_set_aside` |
| `reopen` | Revert an achieved target | `bullseye_revert` |
| `assign` | Mark owned-by-another (`id` + `owner` + `reason`) | — |
| `unassign` | Clear ownership exclusion | — |

On **track create**, the allocated ID is knowable **only** from the
result envelope (`ids:`) — never predict it (TOCTOU).

Create requires `name` + `acceptance`. **`value` / `cost` are optional**
(portfolio annex; omit at repo scope → stored as 0.0 unscored).

**`op=achieve` requires `attestation`** (🎯T58): non-empty free text on how
you believe the target is met (SHA, test name, persona oracle, owner smoke,
residual risk). Same class of check as `defer`'s `reason` — words in a box,
not formal proof. Persisted on the target (`attestation` field +
`Achieved YYYY-MM-DD: …` context line). Missing / whitespace-only / trivial
tokens (`done`, `ok`, …) are rejected with a nudge in the error copy.

### `bullseye_plan_checks`

Same behaviour as legacy `bullseye_verify`: returns a plan of sawmill
invocations. Bullseye never executes the checks.

## Mutation result envelope

Successful mutations return a text payload beginning with a structured
header, then a human summary:

```text
# result
ok: true
op: track
ids: T12
changed: T12
frontier: T10, T12, T15
file: /path/to/bullseye.yaml

Created 🎯T12 "…"
```

- `ids` — primary allocated / operated IDs (comma-separated).
- `changed` — all targets whose records changed (may include dependents
  rewired by `blocks` / `split`).
- `frontier` — refreshed unblocked leaf IDs after the write.

## Error codes

Errors are text messages that include a stable `code=` token:

| Code | When |
|------|------|
| `not_initialized` | No `bullseye.yaml` for this cwd |
| `conflict` | CAS lost update / external edit during write |
| `immutable_achieved` | Content edit on achieved target without reopen |
| `id_reserved` | Explicit create collides with git-history ID |
| `validation` | Schema / graph validation failure on write path |
| `unsafe_repo` | Repo state refuses auto-commit mutations |
| `not_found` | Target or path not found |
| `invalid_args` | Bad op/view/parameter combination |

Example:

```text
code=immutable_achieved
message: 🎯T1 is achieved — its content is immutable. …
```

## Extended (L2) tools

Not part of the core agent happy path:

- `bullseye_portfolio`, `bullseye_convergence`
- `bullseye_github_sync`, `bullseye_sync_priorities`
- `bullseye_import`, `bullseye_resolve`

Legacy tools (`list`, `get`, `put`, `retire`, …) remain as **shims**
that call the same handlers.

## Create default location (🎯T61)

Server-level **create-only** default — does not affect discovery:

| Mechanism | Example |
|-----------|---------|
| CLI (MCP start) | `bullseye --default-location external` |
| Environment | `BULLSEYE_DEFAULT_LOCATION=external` |

Resolution for create tools (`init`, `open` create, `import`):

1. Per-call `location` → always wins.
2. Else server `default_location` if set.
3. Else location prompt / `not_initialized`.

## CLI twins

```text
bullseye [--default-location in_repo|external]   # MCP server; create default only
bullseye open [--location in_repo|external] [--cwd DIR]
bullseye query --view VIEW [--cwd DIR] [--id ID] [--filter FILTER]
bullseye commit --op OP …   # see --help
bullseye plan-checks --id ID [--cwd DIR]
```

Plus existing L2 subcommands (`github`, `sync-priorities`).
