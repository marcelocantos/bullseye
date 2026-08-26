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

Installation is a **multi-step process** — it is not complete until
the binary is installed, the MCP client is registered, the session
is restarted, and a tool call succeeds. Stopping after `brew
install` leaves a binary on disk that no agent can talk to.

Recommended (Homebrew, macOS / Linux):

```bash
brew install marcelocantos/tap/bullseye
```

Then register the stdio MCP server (pick the client you use):

```bash
claude mcp add --scope user bullseye -- bullseye
# or
grok mcp add --scope user bullseye -- bullseye
```

Restart the agent session, then verify with `bullseye --version`
and a `bullseye_open` call. There is no HTTP port and no
`brew services` definition — do not probe with `curl`.

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
Install bullseye from https://github.com/marcelocantos/bullseye.
Installation is not complete until every step succeeds:
1. brew install marcelocantos/tap/bullseye
2. Register the stdio MCP server (not HTTP, no port):
   - Claude: claude mcp add --scope user bullseye -- bullseye
   - Grok:   grok mcp add --scope user bullseye -- bullseye
3. Restart this session (the MCP registration only takes effect on the next session start).
4. Verify: bullseye --version, then call bullseye_open with cwd set to my current project.

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
grok mcp add --scope user bullseye -- bullseye
```

The server communicates over stdio using the MCP protocol.

## Storage locations

**Create vs discover** (🎯T61): discovery finds existing files only;
create may use a server `default_location` when per-call `location` is omitted.

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
committed file is always authoritative. Discovery never reads
`default_location`.

**Create default (optional).** Hosts that prefer external ledgers for
third-party repos can set `BULLSEYE_DEFAULT_LOCATION=external` or start
the server with `bullseye --default-location external`. Per-call
`location` on init/open/import still overrides. Without a default,
create tools prompt for `in_repo` or `external` as before.

**Ledger durability (🎯T73).** Mutating tools write `bullseye.yaml` and
leave it dirty — bullseye does not create `Update bullseye.yaml`
commits. Standing invariants (`make bullseye`) ignore ledger dirt so a
fresh mutation does not block `/cv`. Other dirt should be a loud
warning (exit 0), not a hard fail — leftover WIP is normal during
`/cv`; clean-tree remains a ship/release gate. `/commit` always
stages a dirty in-repo file; `/push` refuses if it is still dirty.
Yaml-only auto-commits mean a pre-T73 binary.

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

Bullseye is an **intent ledger** (desired states + dependencies + claim
lifecycle), not a task assigner. Prefer the **core** surface; legacy
names remain as shims. Full contract: [docs/api-v1-core.md](docs/api-v1-core.md).

### Core (day-to-day)

| Tool | Description |
|------|-------------|
| `bullseye_open` | Discover / init / session snapshot |
| `bullseye_query` | Reads — `view`: context (default), frontier, target, list, summary, graph, validate |
| `bullseye_commit` | Writes — `op`: track, block, split, achieve, defer, reopen, assign, unassign, postpone, wake, rehash. Mutation results include `ids`, `changed`, refreshed `frontier`. |
| `bullseye_plan_checks` | Plan-only mapping of a target's `checks` to sawmill invocations (does not run them) |

CLI twins: `bullseye open|query|commit|plan-checks`.

### Compatibility shims

`list`, `get`, `put`, `retire`, `set_aside`, `revert`, `subdivide`,
`frontier`, `validate`, `graph`, `init`, `startup_context`, `summary`,
`verify` — same handlers as the core tools above.

### Extended (L2)

| Tool | Description |
|------|-------------|
| `bullseye_portfolio` | Cross-repo portfolio summary (WSJF / enablers) |
| `bullseye_convergence` | Invariants hook + unreleased fixes + frontier recommendation |
| `bullseye_github_sync` | GitHub issues ⇄ targets via `gh` |
| `bullseye_sync_priorities` | Portfolio frontier → SQLite priorities table |
| `bullseye_import` | Import targets from markdown |
| `bullseye_resolve` | Resolve partial repo reference to absolute path |

See [agents-guide.md](docs/agents-guide.md) for parameters, schema,
workflows, and the [default agent snippet](docs/agents-guide.md#agent-integration).

## CLI subcommands

Core verbs (MCP twins):

| Subcommand | Purpose |
|------------|---------|
| `bullseye open` | Discover / init / context snapshot |
| `bullseye query` | Reads (`--view context|frontier|…`) |
| `bullseye commit` | Writes (`--op track|achieve|…`) |
| `bullseye plan-checks` | Emit sawmill check plan for a target |

Extended (cron / fleet):

| Subcommand | Purpose |
|------------|---------|
| `bullseye sync-priorities` | Scan the workspace, compute the portfolio frontier, and upsert each frontier target into a SQLite `targets_priorities` table. Designed for periodic invocation from cron or a daemon hook. See [mcp-triad.md §7](docs/mcp-triad.md) for the Protocol-app sync chain. |
| `bullseye github sync` | Mirror GitHub issues into bullseye targets and reflect target lifecycle back to issues, using the `gh` CLI (so authentication is your existing `gh` session — no token stored). Mirrored issues become `GH<n>` targets, keyed by the issue number; closing/reopening a mirrored target closes/reopens its issue. |
| `bullseye convergence` | Invariants hook + unreleased fixes + frontier recommendation (`--cwd`, `--skip-invariants`, `--momentum`). |
| `bullseye portfolio` | Cross-repo portfolio summary. |
| `bullseye import` | Import targets from markdown. |
| `bullseye resolve` | Resolve a partial repo reference to an absolute path. |

Example crontab entry (every 30 minutes):

```
*/30 * * * * /usr/local/bin/bullseye sync-priorities
```

Run `bullseye sync-priorities --help` for flags (`--db`, `--root`,
`--horizon`, `--max-depth`), or `bullseye github --help` for the issue
mirror (`--repo`, `--label`, `--assignee`, `--pull-only`, `--push-only`,
`--dry-run`).

Both subcommands are also exposed as MCP tools — `bullseye_sync_priorities`
and `bullseye_github_sync` — so an agent driving the MCP server can trigger
them directly. Every capability is reachable from both surfaces.

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

See [AGENTS.md](AGENTS.md) for architecture details.

## License

Apache 2.0 — see [LICENSE](LICENSE).
