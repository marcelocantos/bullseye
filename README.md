# Bullseye

An MCP server for managing **targets** — desired project states
expressed as testable properties, with dependency tracking and
frontier computation.

Targets live in `bullseye.yaml` (source of truth). Each repo chooses —
once, at `bullseye_init` time — whether its `bullseye.yaml` lives
**in-repo** (committed alongside the code) or **external** (in a shadow
tree under `~/.local/share/bullseye/` mirroring the cwd's absolute path).
Discovery checks both locations and uses whichever already exists. See
[Storage locations](#storage-locations) below.

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
# Binary at target/release/bullseye
```

## MCP client configuration

Add to `.mcp.json` (project scope) or `~/.claude.json` (user scope):

```json
{
  "mcpServers": {
    "bullseye": {
      "command": "bullseye",
      "args": []
    }
  }
}
```

Or via the CLI:

```bash
claude mcp add --scope user bullseye -- bullseye
```

The server communicates over stdio using the MCP protocol.

## Storage locations

Each repo picks its location **once**, when you call `bullseye_init`:

| Location | Where `bullseye.yaml` lives | Use when |
|----------|-----------------------------|----------|
| `in_repo` | Inside the repo, discovered by walking up from `cwd`. | You own the repo and the team has adopted bullseye. |
| `external` | Shadow tree under `~/.local/share/bullseye/` mirroring the cwd's absolute path. Discovery walks up the shadow tree the same way it walks up the real tree. | Repo is read-only to you (corporate repos where bullseye isn't adopted), or targets are personal to you. |

External mode is purely path-driven — no assumptions about git remotes
or `host/org/repo` layouts — so monorepos, non-git directories, and
unconventional workspaces all resolve identically.

**Discovery is automatic after init.** Every target-operating tool calls
`discover_anywhere(cwd)`, which checks the in-repo walk-up first and
then the shadow walk-up. Whichever file already exists wins. If both
exist (edge case — e.g. a moved repo), **in-repo wins**: an explicit
committed file is always authoritative.

**No global config file.** Each repo's location is encoded by where
its `bullseye.yaml` lives on disk. There's nothing to sync across
machines, no `~/.config/bullseye/` directory, and no machine-wide
setting to get wrong.

## Concurrency protocol

`bullseye.yaml` is expected to be edited by bullseye **and** by humans,
scripts, and other tools. Every mutating bullseye tool follows this
protocol so concurrent writers serialise cleanly and lost-update races
don't silently clobber each other:

1. **Sibling lockfile.** Bullseye acquires an exclusive advisory lock
   on `<dir>/bullseye.yaml.lock` — a 0-byte sentinel file next to the
   yaml. POSIX uses `flock(2) LOCK_EX`; Windows uses `LockFileEx`.
   The lock is on the separate lockfile (not the yaml itself) so the
   anchor stays stable across atomic renames of the yaml.
2. **Bounded wait.** Lock acquisition times out after ~5 s with a
   structured error naming the contended lockfile. Another tool
   hanging on the lock does not hang bullseye indefinitely.
3. **Fresh read.** Inside the lock, bullseye re-reads the yaml from
   disk, bypassing its in-memory parse cache. Any prior-version state
   held across tool calls is invalidated.
4. **CAS on `(mtime, len)`.** Before writing back, bullseye re-stats
   the yaml. If either field changed between read and write — caught
   when a non-flock-honouring writer (a text editor, a quick-edit
   script) modified the file under our nose — the mutation fails with
   a conflict error and is not applied.
5. **Atomic write.** The new yaml is written to a sibling tempfile in
   the same directory, fsync'd, then renamed into place. Readers see
   either the old or the new file, never a half-written one.
6. **Lock release on drop.** The flock is released when bullseye's
   file handle drops at the end of the operation.

**If your tool wants to edit `bullseye.yaml` safely alongside bullseye:**
acquire an exclusive `flock(2)` / `LockFileEx` on
`<dir>/bullseye.yaml.lock` before your read-modify-write window and
release it after. The lockfile will be auto-created if it doesn't yet
exist. If you don't honour advisory locks (e.g. a plain `vi` or a
shell one-liner), bullseye's CAS check will detect your edit and
report a conflict rather than clobbering it — on conflict, retry.

**First time in a new repo:**

```
# Pick in-repo (team-adopted repo, committing alongside code):
bullseye_init location=in_repo

# Or external (read-only / corporate repo, or personal use):
bullseye_init location=external
```

After that, every other bullseye tool just works.

## Tools

Bullseye exposes 17 MCP tools. All target-operating tools accept a
`cwd` parameter; discovery resolves the targets file automatically.

| Tool | Description |
|------|-------------|
| `bullseye_list` | List targets (active/achieved/all) |
| `bullseye_get` | Get a single target by ID with full detail |
| `bullseye_put` | Upsert a target — create (auto- or explicit ID) or patch in one call. Rejects content patches on achieved targets unless re-opened in the same call. |
| `bullseye_retire` | Mark a target achieved |
| `bullseye_verify` | Emit a structured plan that maps each of a target's `checks` to a sawmill tool invocation. Plan-only — the calling agent runs the checks and folds results back. |
| `bullseye_frontier` | Unblocked leaf targets ready for work, ordered by distance-to-nearest-observable-checkpoint then unblocking fanout |
| `bullseye_rework` | Trigger rework from a failed verification |
| `bullseye_tunnels` | Detect work targets with no observable checkpoint reachable within N hops |
| `bullseye_validate` | Check schema conformance |
| `bullseye_graph` | Generate Mermaid dependency graph |
| `bullseye_import` | Import targets from markdown into YAML |
| `bullseye_init` | Create starter bullseye.yaml with sample target |
| `bullseye_startup_context` | Session startup context (frontier, recent achievements, warnings) |
| `bullseye_portfolio` | Cross-repo portfolio summary with frontier targets, including cross-repo edges |
| `bullseye_summary` | Consolidated status overview: groups, frontier ordered by distance + fanout, blocked, stale |
| `bullseye_convergence` | End-to-end convergence evaluation: runs `make bullseye` for invariants, scans git for unreleased fixes, emits summary with frontier detail inline, and computes a deterministic next-action recommendation. Single call, replaces the old multi-tool `/cv` worker. |

See [agents-guide.md](docs/agents-guide.md) for detailed tool
parameters, the bullseye.yaml schema, usage workflows, and a
[copy-pasteable CLAUDE.md snippet](docs/agents-guide.md#agent-integration)
for wiring Bullseye into your project's agent instructions.

## Key concepts

- **Frontier**: The set of unblocked leaf targets that can be worked
  on right now, in parallel.
- **Observable targets**: Work targets that produce something the
  human decision-maker can look at and react to — a checkpoint.
  Marked with `observable: true`. Verify-kind targets are
  observable by definition. Repo-level frontier ordering rewards
  shortest-path-to-nearest-observable-target, so chains of opaque
  work targets get flagged rather than ordered.
- **Tunnels**: Work targets with no observable checkpoint reachable
  within N hops — a signal to reshape the graph by adding an
  intermediate observable target or promoting an existing work
  target with the `observable` flag.
- **Verification**: Verify-kind targets validate work targets. When
  verification fails, a rework edge re-enters the upstream target
  with a diagnosis and retry budget.
- **Phase boundary**: Bullseye uses different prioritisation engines
  at repo and portfolio scopes. Inside a repo (sub-week horizon,
  human as decision-maker), ordering is driven by distance-to-
  observable-checkpoint — `value`/`cost` are not consumed. Across
  repos (weekly-plus horizon, human as bottleneck allocator), WSJF
  with momentum and cross-repo enabler propagation earns its keep.
  See `docs/mcp-triad.md` §9.

## Development

```bash
cargo build          # Build
cargo test           # Run unit + integration tests
cargo clippy         # Lint
cargo fmt --check    # Check formatting
```

See [CLAUDE.md](CLAUDE.md) for architecture details.

## License

Apache 2.0 — see [LICENSE](LICENSE).
