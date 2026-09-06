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
`rustc` from the repo toolchain. Measured 2026-09-06 with the burst
fan-out running alongside; the 50% tolerance is sized for exactly that.

## Locked table

| path | fixture | targets | min µs | median µs | iters |
|---|---|---:|---:|---:|---:|
| parse | synth-1k | 1000 | 9223 | 9589 | 209 |
| render | synth-1k | 1000 | 8460 | 8951 | 223 |
| frontier | synth-1k | 1000 | 406 | 431 | 300 |
| validate | synth-1k | 1000 | 1749 | 1975 | 300 |
| summary | synth-1k | 1000 | 2651 | 2915 | 300 |
| context | synth-1k | 1000 | 1876 | 2096 | 300 |
| mermaid | synth-1k | 1000 | 327 | 344 | 300 |
| parse | synth-5k | 5000 | 47535 | 48700 | 41 |
| render | synth-5k | 5000 | 43014 | 43938 | 46 |
| frontier | synth-5k | 5000 | 2536 | 2783 | 300 |
| validate | synth-5k | 5000 | 10203 | 11023 | 182 |
| summary | synth-5k | 5000 | 15012 | 15832 | 127 |
| context | synth-5k | 5000 | 11118 | 11743 | 171 |
| mermaid | synth-5k | 5000 | 1919 | 2134 | 300 |
| parse | synth-20k | 20000 | 191799 | 194120 | 11 |
| render | synth-20k | 20000 | 178226 | 179815 | 12 |
| frontier | synth-20k | 20000 | 13062 | 14241 | 141 |
| validate | synth-20k | 20000 | 47372 | 48573 | 42 |
| summary | synth-20k | 20000 | 69924 | 71410 | 29 |
| context | synth-20k | 20000 | 50019 | 51420 | 39 |
| mermaid | synth-20k | 20000 | 9608 | 10371 | 193 |
| parse | jevons | 797 | 16807 | 17468 | 115 |
| render | jevons | 797 | 21687 | 22164 | 90 |
| frontier | jevons | 797 | 162 | 172 | 300 |
| validate | jevons | 797 | 294 | 312 | 300 |
| summary | jevons | 797 | 452 | 499 | 300 |
| context | jevons | 797 | 342 | 362 | 300 |
| mermaid | jevons | 797 | 44 | 46 | 300 |
| portfolio-cold | ws-80 | 5550 | 61955 | 63637 | 31 |
| portfolio-warm | ws-80 | 5550 | 6099 | 6940 | 285 |
| portfolio-cold | ws-20k | 20000 | 221397 | 227436 | 10 |
| portfolio-warm | ws-20k | 20000 | 20168 | 21864 | 90 |

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
