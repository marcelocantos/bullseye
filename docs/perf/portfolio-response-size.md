# `bullseye_portfolio`: the cost was the response, not the scan

## Symptom

The owner's `bullseye_portfolio` MCP call was aborted after **1800 s**
having emitted no response and no progress. Reproduced exactly on
2026-09-06: 1821 s, aborted, nothing returned.

## What it was not

Every plausible compute explanation measures in the tens of milliseconds:

| measurement | result |
|---|---|
| `bullseye portfolio` CLI over `~/work` (133 repos, 2.1k active targets) | **0.20 s** |
| the same CLI built from `00b2ba1`, before the three algorithmic fixes | **0.23 s** |
| `--max-depth 12` (walk 2.4× deeper) | 2.28 s |
| `portfolio-cold` bench row, 80 synthetic repos | 62 ms |
| POST `tools/call` straight to the HTTP server on `:18743` | **0.71 s**, 205 KB |

There is no `gh`, `git`, or other subprocess anywhere on the portfolio
path, and no per-item network call: `discover_repos` walks the tree,
parses each `bullseye.yaml`, and computes. The scan was never slow.

## What it was

The MCP client reaches bullseye through `mcpbridge`, a stdio↔HTTP relay.
Driving that relay directly, with the server answering in 0.7 s behind it:

| scan | response bytes | relayed? |
|---|---:|---|
| one repo | 1.5 KB | 0.0 s |
| `marcelocantos/` @ depth 2 | 55 KB | 0.1 s |
| `github.com/` @ depth 3 | ~120 KB | **hung** |
| `~/work` @ depth 5 | 205 KB | **hung** |

The relay carries 55 KB and stalls indefinitely on the next size up — a
buffering bug in the bridge, not in bullseye. But bullseye handed it a
**205 KB** payload: every one of 1716 frontier targets across 133 repos,
one line each, unbounded in both repo count and targets per repo. A
prioritisation answer no caller can read, in a payload no transport
promised to carry.

## The fix

`format_portfolio` is now bounded (`DETAILED_REPOS`, `TARGETS_PER_REPO`,
`MAX_CROSS_EDGE_LINES` in `src/portfolio.rs`). The top 20 repos keep
their per-target WSJF reasoning, capped at 5 targets each; every
remaining repo still appears as one ranked line, so nothing is hidden —
only elided, and every elision states its own count.

Over the real `~/work`: **183 KB → 18 KB**, and through the same
`mcpbridge` stdio path that hung: **hung → 0.24 s**.

Pinned by `format_portfolio_response_stays_under_ceiling`, which fails at
503 KB with the bounds removed.

## Residue

`mcpbridge`'s stall on responses above ~64 KB is a real bug and is still
there. Bullseye is now far below the threshold, but any tool that grows
its response past it will hit the same silent hang, in any MCP server
behind that bridge.

## Progress reporting

`bullseye_portfolio` now emits `notifications/progress` every ten repos
while it scans, when the caller supplies a `progressToken`. Verified
against the running HTTP server with a listener on the notification
stream: 16 notifications (`scanned 10 repos — …` … `scanned 160 repos —
…`) alongside a result returned in 0.54 s.

This is a survivability fix, not the performance fix. The client aborts
a call that emits nothing for long enough, and it cannot distinguish a
slow scan from a wedged server; progress removes that failure mode for
any future workspace where the scan is genuinely slow. It would **not**
have rescued the 1800 s call — `mcpbridge` does not open the server's
notification stream at all (the server logs `transport stream does not
exists or is closed` for every notification sent through it), so the
notifications reach a direct HTTP client but not one behind the bridge.
Both are bridge bugs, and both belong upstream in `mcpbridge`.
