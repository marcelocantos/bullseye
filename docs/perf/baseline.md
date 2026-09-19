# Hot-path performance baseline

Locked numbers for `cargo bench --bench hot_paths`. The bench prints this
table; `--ratchet` re-measures a quick sample and compares against it.

## How to read it, and how to change it

`min µs` is the **minimum** of N timed iterations, not the mean. The gate
runs on a developer Mac that is often carrying a fan-out of other agents;
under load the mean tracks whatever else is running, while the minimum is
the best the code managed when it briefly had the machine to itself. That
is a property of the code, and it is the only statistic a tight threshold
can be honest about.

The gate is a **ratchet, not a floor**. A row fails when it is more than
50% slower than the locked number *and* when it is more than 50% faster
(`RATCHET_TOLERANCE` in `benches/hot_paths.rs`). A speed-up is welcome,
but it has to land as a deliberate edit to this file in the same commit —
otherwise the baseline rots upward, silently absorbs the next regression,
and stops meaning anything. A missing row also fails: a new hot path must
be locked before it can drift.

To re-lock: run `cargo bench --bench hot_paths` on a quiet machine, paste
the table below, and say in the commit message *why* each moved number
moved.

## Environment

MacBook Pro M4 Max, 128 GB, macOS 26 (Tahoe) arm64, release profile,
`rustc` from the repo toolchain. Re-measured 2026-09-07 on a **quiet**
machine (load average 3, fan-out drained). The numbers this replaces were
taken on 2026-09-06 while the burst fan-out was running and were therefore
load-contaminated; every row moved by less than 4%, which is the evidence
that the minimum statistic is doing its job. The 50% tolerance stays,
because the gate still has to pass when the machine is busy.

## Locked table

| path | fixture | targets | min µs | median µs | iters |
|---|---|---:|---:|---:|---:|
| parse | synth-1k | 1000 | 9307 | 9636 | 195 |
| render | synth-1k | 1000 | 8489 | 8938 | 223 |
| frontier | synth-1k | 1000 | 395 | 411 | 300 |
| validate | synth-1k | 1000 | 1688 | 1785 | 300 |
| summary | synth-1k | 1000 | 2587 | 2766 | 300 |
| context | synth-1k | 1000 | 1829 | 2007 | 300 |
| mermaid | synth-1k | 1000 | 322 | 341 | 300 |
| parse | synth-5k | 5000 | 48046 | 48954 | 41 |
| render | synth-5k | 5000 | 43793 | 45950 | 37 |
| frontier | synth-5k | 5000 | 2658 | 3044 | 300 |
| validate | synth-5k | 5000 | 10362 | 11264 | 178 |
| summary | synth-5k | 5000 | 15117 | 15975 | 126 |
| context | synth-5k | 5000 | 11233 | 11923 | 165 |
| mermaid | synth-5k | 5000 | 1951 | 2225 | 300 |
| parse | synth-20k | 20000 | 193970 | 199746 | 10 |
| render | synth-20k | 20000 | 178556 | 180667 | 11 |
| frontier | synth-20k | 20000 | 12486 | 13845 | 144 |
| validate | synth-20k | 20000 | 45878 | 47003 | 43 |
| summary | synth-20k | 20000 | 68019 | 70280 | 29 |
| context | synth-20k | 20000 | 49879 | 51153 | 39 |
| mermaid | synth-20k | 20000 | 9538 | 10148 | 195 |
| parse | jevons | 797 | 17184 | 17480 | 114 |
| render | jevons | 797 | 21356 | 22053 | 89 |
| frontier | jevons | 797 | 161 | 171 | 300 |
| validate | jevons | 797 | 297 | 311 | 300 |
| summary | jevons | 797 | 451 | 481 | 300 |
| context | jevons | 797 | 346 | 361 | 300 |
| mermaid | jevons | 797 | 46 | 46 | 300 |
| portfolio-cold | ws-80 | 5550 | 62668 | 66825 | 28 |
| portfolio-warm | ws-80 | 5550 | 6684 | 7598 | 186 |
| portfolio-cold | ws-20k | 20000 | 239278 | 448653 | 10 |
| portfolio-warm | ws-20k | 20000 | 22622 | 64335 | 32 |

## End-to-end reference (not gated)

Measured against the real `~/work` tree (133–153 repos, ~2.1k active
targets, ~6.4k achieved), release binary, three runs, best wall:

| command | wall |
|---|---|
| `bullseye portfolio` (max-depth 5, warm page cache) | 0.20 s |
| `bullseye portfolio --max-depth 12` | 2.28 s |
| `bullseye_portfolio` over the HTTP MCP server (`:18743`) | 0.71 s |

These are not gated rows — the tree they scan changes hourly — but they
are the sanity check that the synthetic workspace rows above are the
right shape. **Portfolio compute has never been the problem**; see
`docs/perf/portfolio-response-size.md`.
