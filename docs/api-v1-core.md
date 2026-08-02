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

- If `bullseye.yaml` exists (in-repo or external): return context view.
- If missing and `location` is set: init, then return context.
- If missing and `location` omitted: error `not_initialized` with the
  location prompt.

### `bullseye_query` views

| `view` | Meaning |
|--------|---------|
| `context` | Session snapshot (default when only `cwd` is given) |
| `frontier` | Unblocked leaves (+ fanout ordering) |
| `target` | Full record for `id` |
| `list` | Filtered index (`filter`: active / achieved / set_aside / all) |
| `summary` | Groups, frontier, blocked, stale |
| `graph` | Mermaid dependency graph |
| `validate` | Schema / graph errors only |

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

## CLI twins

```text
bullseye open [--location in_repo|external] [--cwd DIR]
bullseye query --view VIEW [--cwd DIR] [--id ID] [--filter FILTER]
bullseye commit --op OP …   # see --help
bullseye plan-checks --id ID [--cwd DIR]
```

Plus existing L2 subcommands (`github`, `sync-priorities`).
