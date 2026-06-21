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

Recommended (Homebrew, macOS / Linux):

```bash
brew install marcelocantos/tap/bullseye
```

Or from source:

```bash
cargo install --path .
# or
cargo build --release   # binary at target/release/bullseye
```

Repository: <https://github.com/marcelocantos/bullseye>

### Quick start (for an agent)

If you'd rather have your coding agent set this up, paste the
following prompt into the agent:

```
Install bullseye from https://github.com/marcelocantos/bullseye:
1. brew install marcelocantos/tap/bullseye
2. claude mcp add --scope user bullseye -- bullseye
3. Restart this session (the MCP registration only takes effect on the next session start).
4. Verify by calling bullseye_startup_context with cwd set to my current project.

Then read https://raw.githubusercontent.com/marcelocantos/bullseye/master/docs/agents-guide.md
for the full agent guide.
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

1. **Out-of-tree lockfile.** Bullseye acquires an exclusive advisory
   lock (POSIX `flock(2) LOCK_EX`; Windows `LockFileEx`) on a 0-byte
   sentinel file under `std::env::temp_dir()/bullseye/locks/`, named
   by the hex `(dev_t, ino_t)` of the yaml's parent directory. Keying
   on the parent dir's inode pair means the lock follows the project
   across atomic-rename writes (which change the yaml's inode but not
   the parent's), repo directory renames (which keep the directory's
   inode), and symlinked access paths (canonicalised before stat). On
   macOS this resolves to the per-user `$TMPDIR`; on Linux to `/tmp`.
   The project directory itself stays clean — no lockfile artefact
   next to the yaml.
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
the simplest path is to rely on bullseye's CAS-on-`(mtime, len)`
detection — do your read-modify-write quickly; if bullseye's flock
window overlaps yours, its CAS check will detect your edit and
report a conflict rather than clobbering it. On conflict, retry. To
participate in bullseye's flock directly, replicate the lock-path
computation: `canonicalize` the yaml's parent directory, take its
`(dev_t, ino_t)`, and `flock` the file
`<temp_dir>/bullseye/locks/<dev>-<ino>.lock` (`mkdir -p` the parent
first; the lockfile auto-creates as a 0-byte sentinel).

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
| `bullseye_revert` | Move a target from achieved back to converging (e.g. a regression proves the achievement was premature). Clears the achieved date and appends a timestamped revert note to the target's context. |
| `bullseye_set_aside` | Set a target aside (parked / deferred / wont-fix) with a required rationale. Distinct from retirement: the target was not delivered, but it's removed from the active set and unblocks dependents the same way an achieved target does. |
| `bullseye_subdivide` | Split a parent target into one or more children in a single call, rewiring existing dependents per `mode`: `add` (parent unchanged, dependents gain children alongside), `aggregate` (parent becomes a converging umbrella that depends_on the children), or `retire` (parent retires as-scoped, dependents are rewired from parent to children). |
| `bullseye_verify` | Emit a structured plan that maps each of a target's `checks` to a sawmill tool invocation. Plan-only — the calling agent runs the checks and folds results back. |
| `bullseye_frontier` | Unblocked leaf targets ready for work, ordered by maximum unblocking fanout then target ID |
| `bullseye_validate` | Check schema conformance |
| `bullseye_graph` | Generate Mermaid dependency graph |
| `bullseye_import` | Import targets from markdown into YAML |
| `bullseye_init` | Create starter bullseye.yaml with sample target |
| `bullseye_startup_context` | Session startup context (frontier, recent achievements, warnings) |
| `bullseye_portfolio` | Cross-repo portfolio summary with frontier targets, including cross-repo edges |
| `bullseye_summary` | Consolidated status overview: groups, frontier ordered by unblocking fanout, blocked, stale |
| `bullseye_convergence` | End-to-end convergence evaluation: runs `make bullseye` for invariants, scans git for unreleased fixes, emits summary with frontier detail inline, and computes a deterministic next-action recommendation. Single call, replaces the old multi-tool `/cv` worker. |

See [agents-guide.md](docs/agents-guide.md) for detailed tool
parameters, the bullseye.yaml schema, usage workflows, and a
[copy-pasteable CLAUDE.md snippet](docs/agents-guide.md#agent-integration)
for wiring Bullseye into your project's agent instructions.

## CLI subcommands

Beyond the MCP server, bullseye exposes a small CLI for cron-driven
maintenance:

| Subcommand | Purpose |
|------------|---------|
| `bullseye sync-priorities` | Scan the workspace, compute the portfolio frontier, and upsert each frontier target into a SQLite `targets_priorities` table. Designed for periodic invocation from cron or a daemon hook. See [mcp-triad.md §7](docs/mcp-triad.md) for the Protocol-app sync chain. |
| `bullseye github sync` | Mirror GitHub issues into bullseye targets and reflect target lifecycle back to issues, using the `gh` CLI (so authentication is your existing `gh` session — no token stored). Mirrored issues become `GHI-<n>` targets, keyed by the issue number; closing/reopening a mirrored target closes/reopens its issue. |

Example crontab entry (every 30 minutes):

```
*/30 * * * * /usr/local/bin/bullseye sync-priorities
```

Run `bullseye sync-priorities --help` for flags (`--db`, `--root`,
`--horizon`, `--max-depth`), or `bullseye github --help` for the issue
mirror (`--repo`, `--label`, `--assignee`, `--pull-only`, `--push-only`,
`--dry-run`).

## Key concepts

- **Frontier**: The set of unblocked leaf targets that can be worked
  on right now, in parallel. The frontier is the *parallelisable set*
  — agents are expected to fan out across it rather than pick a single
  item at a time.
- **Frontier ordering**: Within a repo, frontier targets are ordered
  by descending unblocking fanout (count of active targets that depend
  on this one), tiebroken by ascending target ID. Per-target
  `value`/`cost` are not consumed by repo-level ordering — those are
  portfolio-scope inputs only.
- **Phase boundary**: Bullseye uses different prioritisation engines
  at repo and portfolio scopes. Inside a repo (sub-week horizon,
  human as decision-maker), ordering is driven by unblocking fanout.
  Across repos (weekly-plus horizon, human as bottleneck allocator),
  WSJF with momentum and cross-repo enabler propagation earns its
  keep. See `docs/mcp-triad.md` §9.
- **Verification**: The acceptance criteria on every target *are* the
  verification contract. Whether the pass signal comes from CI, a
  human review, a smoke test, or a design walkthrough is free text on
  the acceptance field — not a property of the node type. Every target
  is structurally identical; there is no separate "verify-kind".

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
