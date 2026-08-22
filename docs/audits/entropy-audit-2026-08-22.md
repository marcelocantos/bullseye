# Entropy audit — bullseye

- **Date:** 2026-08-22
- **Repository:** `/Users/marcelo/work/github.com/marcelocantos/bullseye`
- **Branch:** `master` (tracking `origin/master`)
- **HEAD:** `75751f30d9ce4c2cde7f168a9114346ccadea87a` — *Stop auto-committing bullseye.yaml after mutations (#128)*
- **Crate:** `bullseye 0.46.0` (matches git tag `v0.46.0`)
- **Mode:** full (entropy + hygiene)
- **Initial dirty state:** untracked `.claudia-mcp-home/` only. Treated as user-owned; not staged.

## Executive summary

Bullseye is a single Rust binary that is both an MCP server and a CLI for one intent-ledger file (`bullseye.yaml`). The shipped domain is a uniform DAG: `schema` owns the types, `store` owns durable YAML + flock/CAS, `graph` owns frontier/validate/render, `handler` + `main` expose MCP and CLI over the same functions. That direction is real and enforced in places (`CLI_ROUTES`, `STATUS_SCOPED_FIELDS`, reachability of the published release).

The headline structural mechanism is **adapter and projection concentration**: almost every product change still lands in `src/handler.rs` (2 489 lines, 61 commits since 2026-04) and `src/graph.rs` (1 985 lines, mixed compute + rendering), while an optional second GitHub path and several present-tense architecture docs sit off the CI ratchet. Hygiene is declared and currently holds its floors. There is no P0 current-correctness failure on the default feature set.

Highest-consequence findings: `handler.rs` still owns create/patch/retire domain rules (ENT-001); `graph.rs` mixes frontier, validation, Mermaid, and session text (ENT-002); `--features github-issues` never runs in CI (ENT-003); `docs/design.md` describes an HSM product that schema v5 removed (ENT-004); Homebrew releases skip checksums (ENT-005).

Unverified residue: live `gh`/issuepipe HTTP, cargo-audit/gitleaks (planned, not installed), miri, Windows lock behaviour, whether crates.io `rust-mcp-sdk` 0.9.0 still requires the vendor tokio patch.

## Scope and exclusions

**In scope:** `src/`, `tests/`, `build.rs`, `Cargo.toml`/`Cargo.lock`, `.github/workflows/`, `Makefile`, `hygiene.yaml`, `scripts/probe-published-release.sh`, product docs under `docs/` (except as named below), `AGENTS.md`/`README.md`/`STABILITY.md`.

**Named exclusions (not silent omissions):**

| Path | Role | Treatment |
|------|------|-----------|
| `vendor/rust-mcp-sdk/`, `vendor/rust-mcp-transport/` | Patched crates.io 0.9.0 copies (🎯T47 tokio trim) | Architecture of the patch is in scope; 70+ SDK sources are not line-audited. Warnings from these crates are recorded as vendor residue. |
| `target/` | Build output | Excluded. |
| `tests/fixtures/bullseye.yaml` | Test ledger | Not mixed into production conclusions. |
| `docs/analysis/`, `docs/audit/fable-2026-07.md`, `docs/audit-log.md` | Historical notes | Used as provenance, not as current architecture. |
| `.claudia-mcp-home/` | Untracked Grok/agent runtime | User-owned; not read as product code. |
| Empty `A/`, `B/` directories | Untracked, not in git | Ignored. |

Languages judged: Rust (product), bash (`scripts/probe-published-release.sh`). No Python/Go/C++/web frontend. SQLite is used only behind `feature = "sqlite"` in `priorities.rs`.

## Commands run

| Command | Version / notes | Exit | Shipped vs auxiliary | Limitations |
|---------|-----------------|------|----------------------|-------------|
| `git rev-parse --abbrev-ref HEAD`; `git rev-parse HEAD`; `git status --porcelain=v1 -b` | git | 0 | provenance | Snapshot before any write. |
| `rustc --version`; `cargo --version`; `rustfmt --version`; `cargo clippy --version` | rustc/cargo 1.96.0 (Homebrew); rustfmt 1.9.0; clippy 0.1.96 | 0 | toolchain | No `rust-toolchain.toml`; CI installs `stable`. |
| `cargo fmt --check` | rustfmt 1.9.0 | 0 | shipped (CI `Check formatting`) | Format only. |
| `cargo clippy -- -D warnings` | CI shape | 0 | shipped (CI `Clippy`) | Lib+bins only; vendor crates emit unused-code warnings that do not fail `-D warnings` on the package. |
| `cargo clippy --all-targets -- -D warnings` | Makefile `lint` shape | 0 | auxiliary vs CI | Currently green; CI does not run this. |
| `cargo test --quiet` | default features (`sqlite`) | 0 | shipped (CI `Test`) | 313 passed, 24 ignored doctests from `#[mcp_tool]` `request_params`. Local reachability tests ran (installed binary present). CI sets `BULLSEYE_REACHABILITY_CHECK=skip` on `check` and uses `scripts/probe-published-release.sh` instead. |
| `cargo check --features github-issues --offline` | auxiliary | 0 | **not** CI | Compiles the HTTP client locally; CI never does. |
| `cargo test --features github-issues --lib --offline --quiet` | auxiliary | 0 | **not** CI | Same 133 lib tests as default; HTTP module adds no tests. |
| `cargo tree --depth 1`; `cargo tree -d` | cargo 1.96.0 | 0 | auxiliary | Duplicate `hashbrown` 0.16.1 / 0.17.1 in the graph. |
| `/Users/marcelo/.claude/skills/hygiene/hygiene_check.py` | hygiene skill validator | 0 | hygiene (declared) | See Hygiene posture. `gh_setting` and `command` evidence ran against this machine. |
| Module import graph (local Python over `src/*.rs`) | auxiliary | 0 | locator | First pass false-cycled `schema↔graph↔store` via doc comments; corrected by `use crate::` inspection. |

Not run (unavailable / not declared; not installed for this audit): `cargo-audit`, `cargo-deny`, `gitleaks`, `cargo-machete`, `jscpd`, miri, coverage. `cargo nextest` is the language default but this repo’s shipped gate is `cargo test`.

## Observed architecture

### Entry points and deployable unit

One crate, one binary:

- MCP stdio server when invoked with no subcommand (`src/main.rs` → `server_runtime::create_server`).
- CLI subcommands that construct the same tool structs and call the same `handle_*` functions.
- Release artifacts: `bullseye-{ver}-{darwin-arm64,linux-amd64,linux-arm64}.tar.gz` plus Homebrew tap (`release.yml`).

Default features include `sqlite` (rusqlite bundled). `github-issues` (ureq) is opt-in.

### Directional dependencies (observed)

```
tools.rs (MCP schemas)
    ↑
handler.rs ──→ ops, graph, store, schema, id_alloc, import,
               portfolio, github, convergence, repo_guard, config, api
main.rs  ──→ handler, github, github_issues::http?, priorities?
                 │
schema.rs  ←── store.rs, graph.rs, ops.rs, import.rs, github.rs
config.rs  ←── store.rs
bounded.rs ←── github.rs, convergence.rs, id_alloc.rs, repo_guard.rs
```

No compile-time cycles. `schema.rs` has no `use crate::` edges; comments that mention `crate::graph::validate` / `crate::store::load` are documentation only. `graph` → `schema`; `store` → `schema` + `config`. `handler` is the fan-out hub (15 outbound module uses).

### Declared vs observed rules

**Agree**

- Surface parity: MCP tools share handlers with CLI; `src/cli.rs` `CLI_ROUTES` + `tests/cli_parity_test.rs` fail the build if a new MCP tool has no route.
- Uniform nodes, single `depends_on` edge type, schema v5 (`CURRENT_SCHEMA_VERSION = 5` in `src/schema.rs`).
- Core four tools (`open`/`query`/`commit`/`plan-checks`) dispatch into the shim handlers rather than duplicating mutation logic (`handle_query` / `handle_commit` in `src/handler.rs`).
- Status-scoped fields have one table (`STATUS_SCOPED_FIELDS`) used by validate, transition hygiene, and load-time heal.
- Discovery depth is bounded (`MAX_DISCOVER_DEPTH = 64`).
- Mutations take an out-of-tree flock + `(mtime, len)` CAS (`store::with_locked_mutation`).
- Published-release reachability is a separate CI job, not a version comparison.

**Inferred from code**

- `ops.rs` is only a partial domain layer: revert + subdivide live there; create/patch/retire/defer/assign/postpone still live in `handler.rs` next to MCP `CallToolResult`.
- `graph.rs` is the read-model: frontier, validation, graph hygiene, Mermaid, startup context, summary.
- Two GitHub→target adapters: `gh`-based `GH{n}` (`github.rs`) and issuepipe `GH{repo_id}-{n}` (`github_issues.rs`).

**Contradictions**

- `docs/design.md` present-tense HSM/statechart design vs shipped DAG (`docs/shapes.md`, schema v5, `AGENTS.md`).
- `AGENTS.md` architecture list omits `bounded`, `cli`, `config`, `convergence`, `github_issues`, `id_alloc`, `import`, `ops`, `priorities`, `repo_guard`, `resolve`, `version`.
- `store::with_locked_mutation` docs still say “sibling lockfile `<path>.lock`”; implementation keys a sentinel under `std::env::temp_dir()`.
- `hygiene.yaml` plans `NOTICE` and `docs/TODO.md` while the tree has `NOTICES` and fleet `AGENTS.md` bans TODO files.

**Unknown intent**

- Whether `docs/design.md` is retained as a future product or leftover.
- Whether `--features github-issues` is meant to be a supported ship surface (needs CI) or a prototype (should be documented as such).
- When the vendor tokio patch can be dropped (no automated check against upstream).

## Dimension vector

| Dimension | State | Evidence summary | Change from baseline |
|---|---|---|---|
| Architecture topology | concern | Clear schema→store/graph→handler direction, no cycles; handler and graph are oversized hubs. | First entropy report on this snapshot; April 2026 audit called the layout “clean” at ~1.7k LOC. |
| Redundancy / sources of truth | concern | Deliberate MCP shims + two GitHub identity schemes; competing architecture and attribution docs. | serde_yaml→`serde_yaml_ng` closed; NOTICE vs NOTICES and HSM doc remain. |
| Change amplification | concern | `handler.rs` 61 commits / `tools.rs` 43 / `graph.rs` 37 since 2026-04; new mutation fields still touch MCP schema + handler + CLI flags. | T45 unified dispatch; did not extract put/retire. |
| Local code quality | healthy | Product `fmt`/`clippy -D warnings` clean; UTF-8 `truncate` is char-based; `fs2` API marked deprecated in-tree. | April clippy/fmt findings closed. |
| Correctness / verification | concern | 313 tests green on shipped path; CLI parity and published-release probe exist; `github-issues` and `--all-targets` clippy are off CI. | Reachability job is new since T64/T68/T70. |
| Security / dependencies | concern | Lockfile pinned; no cargo-audit/gitleaks (planned T3); Homebrew `skip_checksum: true`; `fs2` 0.4.3. | Matches hygiene T2 floor, T3 gaps. |
| Build / release / operations | concern | Multi-target CI-built releases, rust-cache, provenance in `--version`; checksum skip; toolchain unpinned (`stable`). | Cache added 2026-04; provenance 🎯T69. |
| Documentation / governance | concern | Strong `docs/api-v1-core.md` + `STABILITY.md`; `design.md` and `AGENTS.md` module map disagree with code; April audit checkboxes stale. | Hygiene docs floor 0 is honest. |

No scalar score.

## Findings

### ENT-001: Ledger mutations still live in the MCP adapter

- **Priority:** P2
- **Dimensions:** Architecture topology; Change amplification; Local code quality
- **Status:** observed fact
- **Evidence:** `src/handler.rs` is 2 489 lines and the highest-churn source (61 commits since 2026-04-01). `handle_put` (`src/handler.rs:942`–`1247`) implements create/patch, historical-ID reservation, status parsing, and write-boundary checks against `PutTool`. `handle_commit` (`src/handler.rs:418`–`608`) only maps `op` onto those handlers. `src/ops.rs` (805 lines) owns revert and subdivide only. Adding a field still requires `src/tools.rs` (MCP schema, 24 `#[mcp_tool]` types), `handle_put` / other handlers, and `src/main.rs` CLI flags.
- **Mechanism:** Domain rules are typed with MCP `CallToolResult`/`PutTool`. A schema or validation fix cannot be unit-tested as a pure `TargetsFile` transform except where `ops` was extracted. The T45 unification stopped *duplicate* implementations; it did not stop *adapter-owned* implementations. Shotgun surgery on every new `op` or persisted field follows.
- **Blast radius:** Every mutating MCP/CLI path; every test that must construct `PutTool`/`CommitTool` (`tests/core_test.rs` calls both).
- **Counterevidence checked:** Dispatch *is* unified — shims and core ops share handlers. `tests/cli_parity_test.rs` gates new MCP tools. Subdivide/revert extraction shows the intended split. Tests do drive `handle_put`/`handle_commit` rather than only internals.
- **Smallest coherent remediation:** Move create/patch/retire/defer/assign/postpone into `ops` (or a `mutate` module) as `fn(&mut TargetsFile, …) -> Result<…, DomainError>`. Keep `handler.rs` as discover/lock/envelope/CLI mapping. Do not split the crate until that extraction lands.
- **Verification:** Domain tests that mutate a `TargetsFile` without `CallToolResult`; handler tests only for envelopes and lock/discover. A new persisted field that is not handled in `ops` should fail a table-driven status-transition test.
- **Ratchet candidate:** Architecture test or grep gate: `handle_put` body must call `ops::track` (or equivalent); no `file.targets.insert` in `handler.rs`.

### ENT-002: `graph.rs` mixes compute, validation, and all read rendering

- **Priority:** P2
- **Dimensions:** Architecture topology; Change amplification
- **Status:** observed fact
- **Evidence:** `src/graph.rs` is 1 985 lines, 37 commits since 2026-04. Public surface includes `frontier` (`:50`), `mermaid` (`:256`), `validate`/`validate_blocking` (`:473`, `:795`), hygiene warnings (`:530`–`:638`), `startup_context` (`:1084`), `summary` (`:1250`). `AGENTS.md:50` already lists this mix as one module.
- **Mechanism:** A frontier ranking change, a Mermaid subgraph flag, and a validate message share one compilation and review unit. Read-path bugs (UTF-8 truncate historically) sit next to blocking graph invariants. High fan-in (`api`, `handler`, `convergence`, `portfolio`) means a render tweak rebuilds and re-reviews validation.
- **Blast radius:** Every `query` view, convergence, portfolio, mutation envelopes (`api::frontier_ids_from_path`).
- **Counterevidence checked:** No import cycle; `schema` stays a leaf. UTF-8 `truncate` is now char-based (`src/graph.rs:1615`–`1621`). Degraded reads vs hard-fail validate (🎯T64) are coherent and documented in `docs/api-v1-core.md`.
- **Smallest coherent remediation:** Split by role, not by layer fashion: `graph/frontier.rs`, `graph/validate.rs`, `graph/mermaid.rs`, `graph/render.rs` (startup/summary text) under the existing module. Keep `graph::validate_blocking` as the single blocking oracle.
- **Verification:** Existing validate/frontier/mermaid tests moved with the files; `cargo test validate` / `frontier` / `mermaid` remain green.
- **Ratchet candidate:** File-size or module-boundary check once split; until then, do not add more renderers to `graph.rs`.

### ENT-003: `github-issues` is a production path CI never compiles

- **Priority:** P2
- **Dimensions:** Correctness / verification; Build / release / operations
- **Status:** observed fact
- **Evidence:** `Cargo.toml` defines optional `github-issues = ["dep:ureq"]`. HTTP client and `issues-poll` live under `#[cfg(feature = "github-issues")]` (`src/github_issues.rs:297`, `src/main.rs:89`–`108`, `src/main.rs:161`–`162`). `.github/workflows/ci.yml` runs `cargo test` with default features only (sqlite, not github-issues). `rg github-issues .github` is empty. Mapping unit tests in `github_issues.rs` compile without the feature (`event_target_id` at `:43`–`:48`); this audit’s `cargo test --features github-issues --lib` still ran 133 tests — no extra HTTP tests. Local `cargo check --features github-issues` succeeded (auxiliary).
- **Mechanism:** A compile break, ureq API change, or background-poller hang in the event path will not redden CI. Default release binaries omit the feature, so the failure shows up only when someone rebuilds with `--features github-issues` or sets `BULLSEYE_ISSUEPIPE_*`.
- **Blast radius:** Event-path consumers (🎯T33/T35); `issues-poll` CLI; MCP background spawn.
- **Counterevidence checked:** Default product does not link ureq. Mapping/filter logic is tested without HTTP. Feature-gating the network dependency is deliberate (`Cargo.toml` comment).
- **Smallest coherent remediation:** Either add a CI job `cargo test --features github-issues --offline` (hermetic; no live Master), or mark the feature unsupported in `STABILITY.md` and `README.md` until that job exists.
- **Verification:** CI job that fails if `src/github_issues.rs` `mod http` does not compile. HTTP behaviour against a live Master remains residue unless a recorded fixture server is added.
- **Ratchet candidate:** `ci.yml` step or hygiene `ci_step` for `cargo test --features github-issues`.

### ENT-004: Present-tense HSM design doc vs shipped uniform DAG

- **Priority:** P2
- **Dimensions:** Documentation / governance; Redundancy / sources of truth
- **Status:** observed fact
- **Evidence:** `docs/design.md:42`–`46`: “The system is a hierarchical state machine (in the Harel statechart tradition)” with rework edges, verify states, retry budgets. Schema comments (`src/schema.rs:44`–`:58`) record that v5 **removed** `kind` / `verifies` / `rework` / `retry_budget`. `docs/shapes.md:13`–`20` is explicit: post-v5 uniform nodes, `depends_on` only. `docs/mcp-triad.md:48` still proposes `targets_verify` as a new executing tool; the shipped surface is plan-only `bullseye_plan_checks` / `bullseye_verify`.
- **Mechanism:** Agents (and humans) who open `docs/design.md` first will plan features the product rejected. That is change amplification through a false architecture, not mere staleness.
- **Blast radius:** New graph features, executor/strategy work, onboarding.
- **Counterevidence checked:** `docs/analysis/graph-engineering-evaluation-2026-08.md` is dated and labelled a one-time evaluation. `STABILITY.md` and `docs/api-v1-core.md` match the shipped DAG. `design.md` is not linked from `AGENTS.md` as current.
- **Smallest coherent remediation:** Banner at the top of `docs/design.md` (and the `targets_verify` paragraph in `mcp-triad.md`): aspirational / superseded by schema v5; pointer to `docs/shapes.md` and `docs/api-v1-core.md`. Do not delete history.
- **Verification:** Doc test or hygiene `file` rule that `docs/design.md` contains `superseded` / `not implemented` near the HSM claim.
- **Ratchet candidate:** hygiene `docs.current-architecture` → `file: {path: docs/design.md, matches: 'superseded|not the shipped model'}`.

### ENT-005: Homebrew release skips checksums

- **Priority:** P2
- **Dimensions:** Security / dependencies; Build / release / operations
- **Status:** observed fact
- **Evidence:** `.github/workflows/release.yml:82` `skip_checksum: true` on `Justintime50/homebrew-releaser@v3`. hygiene `security.signed-releases` is planned T3 (`hygiene.yaml` “no signing/attestation step yet”). `ci.yml` has no top-level `permissions:` block (hygiene `security.actions-perms` planned T3).
- **Mechanism:** The advertised install path (`README.md` `brew install marcelocantos/tap/bullseye`) does not verify artifact checksums at tap update. Combined with unsigned GitHub release assets, a compromised `HOMEBREW_TAP_TOKEN` or release asset can ship a binary that brew will not checksum-reject.
- **Blast radius:** Every Homebrew install; fleet MCP binaries that track the tap.
- **Counterevidence checked:** Release binaries are CI-built per target triple, not locally (`release.yml` `build` job). Formula `test` runs `bullseye --version`. This is not a current known compromise.
- **Smallest coherent remediation:** Stop setting `skip_checksum: true`; generate and publish SHA-256 sums next to the tarballs and let the tap ingest them.
- **Verification:** Formula in `marcelocantos/homebrew-tap` contains `sha256` per bottle/asset; a mismatched asset fails brew.
- **Ratchet candidate:** hygiene evidence `absent:` inverted to a `ci_step`/`file` once checksums exist; do not ratchet until the tap actually verifies.

### ENT-006: CI clippy is weaker than the Makefile lint the repo dogfoods

- **Priority:** P3
- **Dimensions:** Correctness / verification; Build / release / operations
- **Status:** observed fact
- **Evidence:** CI: `cargo clippy -- -D warnings` (`.github/workflows/ci.yml:24`–`25`). Makefile: `cargo clippy --all-targets -- -D warnings` (`Makefile:38`). Both were green on this snapshot.
- **Mechanism:** Warnings only in `tests/*.rs` or integration binaries fail `make lint` / `make bullseye` locally and pass GitHub `check`. Contributors who skip the Makefile can merge test-only clippy debt.
- **Blast radius:** Integration tests (`tests/core_test.rs` et al.), `tests/cli_parity_test.rs`.
- **Counterevidence checked:** Current tree is clean under both commands. CI still denies warnings on lib+bin.
- **Smallest coherent remediation:** Make CI match Makefile: `cargo clippy --all-targets -- -D warnings`.
- **Verification:** A `#[allow]`-free unused import in `tests/core_test.rs` must fail CI.
- **Ratchet candidate:** Change the existing Clippy step; hygiene `quality.lint` evidence already points at that step name.

### ENT-007: Two GitHub→target identity schemes

- **Priority:** P3
- **Dimensions:** Redundancy / sources of truth
- **Status:** observed fact (dual path); inference (collision harm if both enabled on one repo)
- **Evidence:** `github::target_id` → `GH{number}` (`src/github.rs:52`–`55`), origin `github:{repo}#{n}`. `github_issues::event_target_id` → `GH{repo_id}-{number}` (`src/github_issues.rs:43`–`48`), origin `github:repo:{id}#{n}`. Comments in both modules say the namespaces are reserved so the paths can coexist. `id_is_conforming` accepts both (`src/graph.rs:479`–`497`).
- **Mechanism:** Two writers can insert different targets for the same GitHub issue. They do not share `origin`, so neither sync will treat the other as the same object. Lifecycle (open/closed vs achieved) can diverge.
- **Blast radius:** Repos that run both `bullseye github sync` and `issues-poll` / env consumer.
- **Counterevidence checked:** Different ID and origin formats are an explicit T34/T33 split. Event path is strict opt-in. Default binary has no HTTP consumer.
- **Smallest coherent remediation:** Document the mutex in `README`/`STABILITY.md`: do not enable both writers on one ledger. Optionally reject `github sync` creates when `origin` matches `github:repo:`.
- **Verification:** Integration test with both adapters against one fixture issue expects one target, or a documented hard error.
- **Ratchet candidate:** Manual attestation until a dual-writer test exists.

### ENT-008: Lock implementation and its own doc comment disagree

- **Priority:** P3
- **Dimensions:** Local code quality; Security / dependencies
- **Status:** observed fact
- **Evidence:** `src/store.rs:650`–`651` (“Open … a sibling lockfile `<path>.lock`”) vs `:669`–`:672` (lockfiles under `std::env::temp_dir()`, inode-keyed). Implementation uses `lock_path_for` + `fs2::try_lock_exclusive` with `#[allow(deprecated)]` (`src/store.rs:692`–`693`).
- **Mechanism:** Reviewers and agents hardening locking will look for `<yaml>.lock` beside the ledger and miss `/tmp` (Linux) or `$TMPDIR` (macOS) sentinels. Shared `/tmp` on multi-user Linux is a different threat model than a sibling lockfile.
- **Blast radius:** Concurrent mutation debugging; security review of flock.
- **Counterevidence checked:** README “Concurrency protocol” describes the temp-dir inode design correctly. Tests cover timeout/CAS.
- **Smallest coherent remediation:** Delete the sibling-lock sentence; point the numbered list at `lock_path_for`.
- **Verification:** Comment grep for `<path>.lock` returns none in `store.rs` docs.
- **Ratchet candidate:** None until docs match; then a file-content check is optional.

### ENT-009: `fs2` 0.4.3 is unmaintained and the used API is deprecated

- **Priority:** P3
- **Dimensions:** Security / dependencies; Local code quality
- **Status:** observed fact
- **Evidence:** Direct dep `fs2 = "0.4.3"` (`Cargo.toml`); lock checksum in `Cargo.lock`. Call site `try_lock_exclusive` allowed-deprecated (`src/store.rs:692`–`693`). hygiene `security.dep-audit` is planned, not enforced.
- **Mechanism:** Advisory or API removal will hit the mutation lock path with no cargo-audit job to notice. Replacement (`fs4` / rustix flock) is a one-module change but is not scheduled.
- **Blast radius:** All mutating tools.
- **Counterevidence checked:** Lock + CAS tests exist; protocol is documented. No current compile failure.
- **Smallest coherent remediation:** Switch to a maintained flock helper; keep the same timeout/CAS tests.
- **Verification:** `cargo deny` / `cargo audit` plus the existing lock tests.
- **Ratchet candidate:** hygiene `security.dep-audit` when cargo-audit is wired.

### ENT-010: Meta-docs and hygiene items compete with the tree and with fleet rules

- **Priority:** P3
- **Dimensions:** Documentation / governance; Redundancy / sources of truth
- **Status:** observed fact
- **Evidence:**
  - `NOTICES` exists (root, Apache attribution for vendor + SQLite). hygiene `governance.notice` still plans absent `NOTICE` (`hygiene.yaml:241`–`242`).
  - hygiene `docs.todo` plans `docs/TODO.md` (`hygiene.yaml:284`–`292`) “per global convention”; fleet `AGENTS.md` bans TODO files and requires bullseye targets.
  - `docs/audit-2026-04-07.md` still has unchecked items that the tree closed (CI, README, UTF-8 truncate, `serde_yaml` → `serde_yaml_ng`, LICENSE, agents-guide).
  - `AGENTS.md:45`–`55` module list omits half of `src/lib.rs`.
- **Mechanism:** A later `/hygiene init` or agent “close planned gaps” pass will add a banned `docs/TODO.md` or a second `NOTICE` file. Stale audit checkboxes look like open P0s.
- **Blast radius:** Hygiene T3 work; agent onboarding.
- **Counterevidence checked:** Validator currently **passes** because those items are `planned`/`absent` and sit above floors. `NOTICES` vs `NOTICE` is a fleet naming debate (`~/think` 🎯T2), not a missing file.
- **Smallest coherent remediation:** Point `governance.notice` at `NOTICES` (or rename after fleet decision). Change `docs.todo` to `skipped` with reason “bullseye.yaml is the sink”. Mark the April audit as historical. Update the `AGENTS.md` module list.
- **Verification:** `hygiene_check.py` still PASS; `docs/TODO.md` remains absent.
- **Ratchet candidate:** Only after hygiene.yaml is deliberately edited (out of scope for this audit).

### ENT-011: Integration tests are one 7 362-line crate

- **Priority:** P3
- **Dimensions:** Change amplification; Correctness / verification
- **Status:** observed fact
- **Evidence:** `tests/core_test.rs` 7 362 lines, 165 tests of 313. Other integration crates are small (`cli_parity_test.rs`, `reachability_test.rs`, `ledger_sha_stability_test.rs`, `version_provenance_test.rs`). `docs/test-gaps-2026-04-12.md` noted handler-bypass risk; a later `handle_convergence_resolves_repo_root` test exists (`tests/core_test.rs:1586`), but most convergence tests still call `convergence::convergence` directly.
- **Mechanism:** rust.md notes many `tests/*.rs` binaries multiply link time; here the opposite problem is one file that every ledger behaviour edit must search. Review and bisect cost grow with the file, not with the property.
- **Blast radius:** Test authors; cold `cargo test` link of `core_test`.
- **Counterevidence checked:** Tests pass; several tests do use `handle_*`. Splitting binaries would worsen link time if overdone.
- **Smallest coherent remediation:** Split by domain *inside* `tests/` as modules or a few files (`commit`, `query`, `convergence`, `store`) sharing helpers — not one file per test.
- **Verification:** `cargo test --test core_test` still 165 green after a mechanical split.
- **Ratchet candidate:** None until split; then `wc -l` per integration file.

### ENT-012: `issues-poll` is a capability the surface-parity oracle cannot see

- **Priority:** P3
- **Dimensions:** Architecture topology; Correctness / verification
- **Status:** observed fact
- **Evidence:** `CLI_ROUTES` (`src/cli.rs:109`–`139`) lists every MCP tool. `issues-poll` is a CLI subcommand (`src/main.rs:89`) with no `#[mcp_tool]` (background spawn is env-based). `tests/cli_parity_test.rs` therefore cannot fail if `issues-poll` is removed from `main` or `--help`.
- **Mechanism:** The standing sentence “every capability is on both surfaces” is true for MCP tools, false for this CLI-only feature. Drift is silent.
- **Blast radius:** Event-path operators.
- **Counterevidence checked:** Feature is optional; MCP spawn is the other surface when env is set. Parity test’s job is MCP↔CLI for `TargetTools`, which it does.
- **Smallest coherent remediation:** Either add `issues-poll` to a CLI-only route table tested like `CLI_ROUTES`, or document it as CLI/env-only in `STABILITY.md`.
- **Verification:** Deleting the `issues-poll` arm fails a help/dispatch test.
- **Ratchet candidate:** Extend `cli_parity_test.rs` with `CLI_ONLY_SUBCOMMANDS`.

### ENT-013: Vendored MCP SDK still compiles unused HTTP/auth surface

- **Priority:** P3
- **Dimensions:** Build / release / operations; Local code quality
- **Status:** observed fact
- **Evidence:** `vendor/README.md` and `Cargo.toml` `[patch.crates-io]`. tokio features are trimmed (`vendor/rust-mcp-sdk/Cargo.toml:252`–`265`). Every `cargo test`/`clippy` prints 11 unused-import/dead-code warnings from `rust-mcp-sdk` plus one from `rust-mcp-transport`. Bullseye depends with `default-features = false, features = ["server", "macros", "stdio"]`.
- **Mechanism:** The patch copies whole crates. Server/auth/HTTP types remain in the lib and warn. Refresh procedure is manual (`vendor/README.md`). If upstream publishes a non-`full` tokio stdio build, the vendor tree can linger unnoticed.
- **Blast radius:** Cold compile noise; future SDK bumps.
- **Counterevidence checked:** Feature trim is load-bearing (🎯T47, `docs/build-perf-2026-04-11.md`). Product clippy still exits 0.
- **Smallest coherent remediation:** Re-check upstream; if still required, `allow` at the patch crate or exclude unused modules. If upstream is fixed, delete `vendor/` and the patch.
- **Verification:** `cargo tree -i tokio` shows no `full`; `cargo clippy` has zero vendor warnings *or* vendor is gone.
- **Ratchet candidate:** Comment-only until upstream is measured.

### ENT-014: Error codes are recovered by substring after the fact

- **Priority:** P3
- **Dimensions:** Local code quality; Correctness / verification
- **Status:** observed fact
- **Evidence:** `api::format_error` writes `code=…`. `api::classify_message` (`src/api.rs:47`–`93`) maps free-text to `ErrorCode` via `contains`. `handler::tool_err` (`src/handler.rs:91`–`98`) uses it when the message is not already coded.
- **Mechanism:** A new `Apply` string that does not include the magic phrases is advertised as `invalid_args`. Agents branch on `code=`. Tests in `api.rs` cover current phrases only.
- **Blast radius:** MCP clients branching on error codes.
- **Counterevidence checked:** Many paths already use `coded_err`. Core contract documents codes. Mis-classification is fail-soft to `InvalidArgs`, not a silent ok.
- **Smallest coherent remediation:** Return `ErrorCode` from `ops`/`store`; stop classifying by English.
- **Verification:** A mutation error whose text omits “not found”/“conflict” still yields the specific code in the envelope.
- **Ratchet candidate:** Test that every `MutationError` / domain error variant has a unique `ErrorCode` without going through `classify_message`.

### ENT-015: Agent runtime dir is unignored

- **Priority:** P3
- **Dimensions:** Documentation / governance; Build / release / operations
- **Status:** observed fact
- **Evidence:** Initial `git status --porcelain=v1 -b` showed `?? .claudia-mcp-home/`. `.gitignore` lists `/target`, `.claude/`, `.bullseye/` (`.gitignore:1`–`12`) but not `.claudia-mcp-home/`. The tree contains session logs and `auth.json` under that prefix (untracked; not opened for secrets).
- **Mechanism:** `git add .` or a careless `git add -A` would stage agent runtime state, possibly credentials. `.claude/` was already learned as a similar class of dirt.
- **Blast radius:** Accidental commits; secret scanning (which is not in CI).
- **Counterevidence checked:** Currently untracked; this audit did not add it. User-owned.
- **Smallest coherent remediation:** Ignore `.claudia-mcp-home/` (and keep ignoring `.claude/`).
- **Verification:** `git check-ignore -v .claudia-mcp-home/grok/auth.json` reports the rule.
- **Ratchet candidate:** hygiene `file` on `.gitignore` matching `claudia-mcp-home` if adopted.

## Redundancy and competing sources of truth

| Fact | Authorities | Drift risk | Disposition |
|------|-------------|------------|-------------|
| Ledger contents | `bullseye.yaml` + `content_hash` | Direct edits detected (🎯T41) | Single owner; healthy |
| Tool surface | `tools.rs` `tool_box!` vs `CLI_ROUTES` vs `main.rs` match | New MCP tool without route fails `cli_parity_test` | Enforced for MCP tools; `issues-poll` excluded (ENT-012) |
| Core vs shim names | `handle_commit`/`handle_query` map onto shim handlers | Dual MCP names are intentional compatibility | Deliberate duplication; one implementation |
| GitHub issue identity | `GH{n}` vs `GH{repo_id}-{n}` | Two writers, two origins (ENT-007) | Documented split; mutex not enforced |
| Architecture story | `docs/shapes.md` + schema v5 vs `docs/design.md` HSM vs `docs/mcp-triad.md` `targets_verify` | Agents implement the wrong product (ENT-004) | Compete |
| Attribution filename | `NOTICES` on disk vs hygiene `NOTICE` | Double file or false planned gap (ENT-010) | Compete |
| Followable work | `bullseye.yaml` vs planned `docs/TODO.md` vs stale April audit checkboxes | Agents file the banned sink (ENT-010) | Compete |
| Version | `Cargo.toml` `0.46.0` = tag `v0.46.0`; `--version` adds `build.rs` provenance | Provenance exists specifically because crate version alone lied during T64 | Healthy split |
| Clippy invocation | CI lib+bin vs Makefile `--all-targets` | Test-only warnings (ENT-006) | Compete (gates) |
| Lockfile location | Comment `<path>.lock` vs `temp_dir` implementation vs README inode protocol | Reviewers look in the wrong place (ENT-008) | Comment is stale; README matches code |

## Healthy structure worth retaining

- **One mutation pipeline:** `handle_commit` / `handle_query` call the shim handlers; CLI constructs the same structs (`src/main.rs` `cli_*`). This is the right unification; ENT-001 is about moving the *bodies* out of the adapter, not undoing T45.
- **`STATUS_SCOPED_FIELDS`:** one table drives validate, `clear_illegal_status_scoped_fields`, and load-time heal (`src/schema.rs:458`–`533`). This is the T64 lesson encoded; do not go back to per-op field lists.
- **CLI parity oracle:** `src/cli.rs` + `tests/cli_parity_test.rs` is a standing architecture test. Extend it (ENT-012); do not replace it with comments.
- **Published-release reachability:** `tests/reachability_test.rs` + `scripts/probe-published-release.sh` + `ci.yml` `reachability` job. CI `check` opts out loudly; the probe job refuses `BULLSEYE_REACHABILITY_CHECK=skip`. This is the correct delivery oracle after T64.
- **Bounded subprocesses:** `src/bounded.rs` wall-clock + process-group kill on Unix. Convergence/git cannot hang the MCP budget without a coded timeout.
- **Discover/create split (🎯T61):** discovery never reads `default_location`; create may. In-repo wins over external.
- **Schema version gate:** `LoadError::VersionTooNew` refuses newer files rather than dropping fields.
- **Vendor tokio trim (🎯T47):** still justified until upstream stops `features=["full"]` on the stdio path.
- **Hygiene declared and currently holding floors** (see next section).
- **UTF-8 truncate and discover depth** from the April 2026 audit are actually fixed (`graph.rs:1615`, `store.rs:219`).

## Hygiene posture

`hygiene.yaml` exists. Validator: `/Users/marcelo/.claude/skills/hygiene/hygiene_check.py` from repo root. Exit **0 (PASS)**.

```
hygiene: bullseye   aspires tier 3

  dimension     held  floor
  correctness   T2    T2   ✓
  security      T2    T2   ✓
  quality       T2    T2   ✓
  deps          T2    T2   ✓
  release       T2    T2   ✓
  governance    T2    T2   ✓
  build         T2    T2   ✓
  docs          T0    T0   ✓
  perf          T0    T0   ✓
  vcs           T0    T0   ✓
  agent         T2    T2   ✓

gaps to close (unmet, tier ≤ 3):
  correctness: miri[T3]
  security: actions-perms[T3], dep-audit[T3], secret-scan[T3], signed-releases[T3]
  release: changelog[T3]
  governance: notice[T3], security-md[T3]
  docs: rustdoc[T3], todo[T3]
  perf: benchmarks[T3]
  vcs: pre-commit-hook[T3]
```

Overlap with entropy (not double-counted as hygiene drift):

- ENT-005 is the mechanism behind planned `signed-releases` plus `skip_checksum`.
- ENT-009 is why `dep-audit` matters for `fs2`.
- ENT-010 is hygiene items that would be *wrong* to close as written (`NOTICE`, `docs/TODO.md`).
- ENT-003/ENT-006 are not hygiene items today; they are ratchet candidates.

`deps.freshness` is `manual` with `last_verified: 2026-06-15`. Cadence is per-release; v0.46.0 is later than that date. The validator does not check the date against git tags.

## Oracle coverage and residue

| Property | Decided by | Notes |
|----------|------------|-------|
| rustfmt | Shipped CI `Check formatting` | Green. |
| clippy lib+bin `-D warnings` | Shipped CI `Clippy` | Green; tests not in this gate (ENT-006). |
| Unit + integration tests, default features | Shipped CI `Test` + local `cargo test` | 313 passed. |
| MCP↔CLI parity for `TargetTools` | Shipped `tests/cli_parity_test.rs` | Does not cover `issues-poll` (ENT-012). |
| Published binary heals status-scoped residue | Shipped `reachability` job / `scripts/probe-published-release.sh` | Behavioural, not a version compare. Local `check` skips. |
| Schema v5 / gates migration / hash | Shipped tests in store/schema/core_test | |
| Frontier, validate, degraded reads | Shipped graph + core_test | |
| `github-issues` HTTP compile/run | **Nothing in CI** | Local check only (ENT-003). |
| Live `gh` GitHub sync | Tests with `GhClient` fake; live `gh` residue | |
| Live issuepipe Master | No fixture server | Residue. |
| cargo-audit / gitleaks / CodeQL | Planned or skipped in hygiene | Residue. |
| miri / UB | Planned T3; essentially no `unsafe` in `src/` | |
| Homebrew checksum / signing | Not present (ENT-005) | |
| `docs/design.md` currency | None | ENT-004. |
| Windows flock | `fs2` LockFileEx; no Windows CI job | Residue (release matrix is darwin-arm64 / linux-amd64 / linux-arm64). |
| Coverage % / clone detector | Not configured | Not used as a verdict. |

**Owner residue (intent, not mechanical leftover):**

1. Is `docs/design.md` a future product or archive?
2. Is `--features github-issues` a supported ship surface?
3. NOTICE vs NOTICES — fleet filename (`~/think` 🎯T2)?
4. Keep MCP shims in `list_tools` indefinitely, or eventually hide them behind a compat flag?
5. Accept Homebrew unsigned/unchecked artifacts until a signing project exists?

## Remediation sequence

1. **Oracles first:** CI `cargo clippy --all-targets -- -D warnings` (ENT-006); CI `cargo test --features github-issues` or an explicit “unsupported” note (ENT-003). Keep the reachability job as-is.
2. **Truths:** Banner `docs/design.md` / triad `targets_verify` as superseded (ENT-004); retarget hygiene notice/todo items (ENT-010); fix `store.rs` lock comment (ENT-008); ignore `.claudia-mcp-home/` (ENT-015).
3. **Boundaries:** Extract put/retire/defer/assign/postpone into `ops` (ENT-001); split `graph.rs` by role (ENT-002). Do not crate-split yet (`docs/build-perf-2026-04-11.md` still shows cheap incremental builds).
4. **Remove residue after consumers move:** Dual GitHub mutex (ENT-007); `issues-poll` on a CLI-only table (ENT-012); typed errors instead of `classify_message` (ENT-014); `fs2` replacement (ENT-009); Homebrew checksums (ENT-005); vendor drop when upstream allows (ENT-013); split `core_test.rs` (ENT-011).
5. **Ratchet** only after the CI step or file rule exists. Do not edit `hygiene.yaml` in this audit.
6. **Re-run** this report’s commands on the same definitions (default-feature `cargo test`, both clippy shapes, `hygiene_check.py`, github-issues compile).

No production code was changed in this audit.
