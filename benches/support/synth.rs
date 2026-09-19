// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Deterministic synthetic ledgers for the perf harness.
//!
//! Real ledgers (jevons, mnemo, spyder) share a shape: a few hundred
//! top-level targets, dotted families two or three levels deep, a
//! majority of targets already achieved, `depends_on` edges that are
//! mostly parent → child plus a sprinkling of cross-family edges, and
//! long free-text `context` fields that dominate the byte count. This
//! module produces that shape at any size from a fixed seed, so a
//! benchmark row means the same thing on every run and every machine.
//!
//! The output validates cleanly under `graph::validate_blocking`:
//! active parents list every dotted child in `depends_on` (🎯T39.1),
//! achieved targets depend only on terminal targets (🎯T79), and extra
//! edges only point at earlier families, which makes family order a
//! topological order and rules out cycles by construction.

use std::collections::BTreeMap;

use bullseye::schema::{CrossEdge, Status, Target, TargetsFile};
use chrono::NaiveDate;

/// xorshift64* — a few lines, no `rand` dependency, and the same
/// sequence on every platform.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Zero is a fixed point of xorshift; nudge it.
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform integer in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }

    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

/// Fibonacci scale used by `value` / `cost`, plus 0 for "unscored".
const FIB: &[f64] = &[0.0, 1.0, 2.0, 3.0, 5.0, 8.0, 13.0];

/// Vocabulary for generated prose. Long enough that `context` strings
/// carry real byte weight, varied enough that the fake-edge heuristic
/// (`significant_tokens`) sees a realistic hit rate.
const WORDS: &[&str] = &[
    "frontier",
    "ledger",
    "daemon",
    "worker",
    "transcript",
    "stream",
    "render",
    "oracle",
    "gate",
    "commit",
    "release",
    "schema",
    "migration",
    "cache",
    "mutex",
    "lock",
    "timeout",
    "retry",
    "browser",
    "session",
    "harness",
    "fixture",
    "snapshot",
    "overlap",
    "layout",
    "virtual",
    "prefix",
    "height",
    "measure",
    "observer",
    "resize",
    "reload",
    "seat",
    "owner",
    "agent",
    "tool",
    "call",
    "envelope",
    "hash",
    "content",
    "banner",
    "yaml",
    "parse",
    "serialise",
    "achieved",
    "converging",
    "identified",
    "residual",
    "hermetic",
    "green",
    "regression",
    "quadratic",
    "index",
    "scan",
    "walk",
    "depth",
    "portfolio",
    "wsjf",
    "momentum",
    "enabler",
    "boost",
    "value",
    "cost",
    "evidence",
    "attestation",
    "the",
    "a",
    "and",
    "of",
    "so",
    "when",
    "after",
    "before",
    "still",
    "not",
];

fn prose(rng: &mut Rng, words: usize) -> String {
    let mut s = String::with_capacity(words * 7);
    for i in 0..words {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(rng.pick(WORDS));
    }
    s
}

/// Occasionally reference another target by id inside prose, the way
/// real contexts do ("same class as 🎯T4.2"). Feeds the fake-edge
/// heuristic's id-mention path.
fn prose_with_ref(rng: &mut Rng, words: usize, ids: &[String]) -> String {
    let mut s = prose(rng, words);
    if !ids.is_empty() && rng.chance(0.3) {
        s.push_str(" — same class as 🎯");
        s.push_str(rng.pick(ids));
    }
    s
}

fn date(rng: &mut Rng) -> NaiveDate {
    let day = 1 + rng.below(28) as u32;
    let month = 1 + rng.below(8) as u32;
    NaiveDate::from_ymd_opt(2026, month, day).expect("valid synthetic date")
}

/// Shape knobs for one synthetic ledger.
pub struct Shape {
    /// Total targets to generate (families are grown until this is met).
    pub targets: usize,
    /// Repo identifiers cross-repo edges may point at (empty → no cross edges).
    pub other_repos: Vec<String>,
}

/// One generated target before status/edge resolution.
struct Draft {
    id: String,
    family: usize,
    children: Vec<String>,
}

/// Generate a ledger of roughly `shape.targets` targets from `seed`.
pub fn ledger(seed: u64, shape: &Shape) -> TargetsFile {
    let mut rng = Rng::new(seed);

    // --- 1. Grow families: T<n>, T<n>.<m>, T<n>.<m>.<k>. ---
    let mut drafts: Vec<Draft> = Vec::with_capacity(shape.targets);
    let mut family = 0usize;
    while drafts.len() < shape.targets {
        family += 1;
        let root = format!("T{family}");
        let n_children = if rng.chance(0.4) { 0 } else { 1 + rng.below(6) };
        let mut root_children = Vec::new();
        let mut pending: Vec<Draft> = Vec::new();
        for m in 1..=n_children {
            let child = format!("{root}.{m}");
            let n_grand = if rng.chance(0.75) {
                0
            } else {
                1 + rng.below(4)
            };
            let mut grand = Vec::new();
            for k in 1..=n_grand {
                let gid = format!("{child}.{k}");
                grand.push(gid.clone());
                pending.push(Draft {
                    id: gid,
                    family,
                    children: Vec::new(),
                });
            }
            root_children.push(child.clone());
            pending.push(Draft {
                id: child,
                family,
                children: grand,
            });
        }
        drafts.push(Draft {
            id: root,
            family,
            children: root_children,
        });
        drafts.extend(pending);
    }
    drafts.truncate(shape.targets);
    let known: std::collections::HashSet<&str> = drafts.iter().map(|d| d.id.as_str()).collect();
    // Truncation may have dropped children; forget the dangling ones.
    let child_lists: Vec<Vec<String>> = drafts
        .iter()
        .map(|d| {
            d.children
                .iter()
                .filter(|c| known.contains(c.as_str()))
                .cloned()
                .collect()
        })
        .collect();

    // --- 2. Statuses, leaves first so a parent can be achieved only
    // when every child is terminal. Children always come after their
    // parent in `drafts`, so a reverse walk sees children first. ---
    let mut status: BTreeMap<String, Status> = BTreeMap::new();
    for (i, d) in drafts.iter().enumerate().rev() {
        let kids_terminal = child_lists[i]
            .iter()
            .all(|c| status.get(c).is_some_and(|s| s.is_terminal()));
        let s = if kids_terminal && rng.chance(0.6) {
            Status::Achieved
        } else if kids_terminal && rng.chance(0.08) {
            Status::SetAside
        } else if rng.chance(0.35) {
            Status::Converging
        } else {
            Status::Identified
        };
        status.insert(d.id.clone(), s);
    }

    // --- 3. Targets with edges. ---
    let all_ids: Vec<String> = drafts.iter().map(|d| d.id.clone()).collect();
    let mut targets = BTreeMap::new();
    for (i, d) in drafts.iter().enumerate() {
        let st = status[&d.id];
        let mut depends_on = child_lists[i].clone();

        // Cross-family edges: only into earlier families (acyclic by
        // construction); an achieved target only onto terminal ones.
        if d.family > 1 && rng.chance(0.25) {
            for _ in 0..(1 + rng.below(2)) {
                let cand = &drafts[rng.below(i)];
                if cand.family >= d.family {
                    continue;
                }
                if st.is_terminal() && !status[&cand.id].is_terminal() {
                    continue;
                }
                if !depends_on.contains(&cand.id) {
                    depends_on.push(cand.id.clone());
                }
            }
        }

        let (cross_enables, cross_depends) =
            if !shape.other_repos.is_empty() && !st.is_terminal() && rng.chance(0.06) {
                let repo = rng.pick(&shape.other_repos).clone();
                let target = Some(format!("T{}", 1 + rng.below(40)));
                let enables = vec![CrossEdge {
                    repo: repo.clone(),
                    target,
                    capability: None,
                    note: Some(prose(&mut rng, 6)),
                }];
                let depends = if rng.chance(0.5) {
                    vec![CrossEdge {
                        repo,
                        target: None,
                        capability: Some(prose(&mut rng, 3)),
                        note: None,
                    }]
                } else {
                    Vec::new()
                };
                (enables, depends)
            } else {
                (Vec::new(), Vec::new())
            };

        let acceptance = (0..(1 + rng.below(3)))
            .map(|_| {
                let words = 8 + rng.below(12);
                prose_with_ref(&mut rng, words, &all_ids[..i])
            })
            .collect();
        let context_words = if rng.chance(0.3) {
            0
        } else {
            10 + rng.below(90)
        };
        let context = prose_with_ref(&mut rng, context_words, &all_ids[..i]);
        let discovered = date(&mut rng);

        let name_words = 6 + rng.below(10);
        targets.insert(
            d.id.clone(),
            Target {
                name: prose(&mut rng, name_words),
                status: st,
                value: *rng.pick(FIB),
                cost: *rng.pick(FIB),
                actual_cost: None,
                set_aside_reason: (st == Status::SetAside).then(|| prose(&mut rng, 8)),
                attestation: (st == Status::Achieved).then(|| prose(&mut rng, 20)),
                acceptance,
                checks: Vec::new(),
                context,
                gates: Vec::new(),
                depends_on,
                cross_depends,
                cross_enables,
                tags: if rng.chance(0.1) {
                    vec![rng.pick(WORDS).to_string()]
                } else {
                    Vec::new()
                },
                strategy: None,
                origin: "synth".to_string(),
                discovered,
                achieved: (st == Status::Achieved).then(|| date(&mut rng)),
                owned_by: None,
                postponed_until: None,
                postpone_predicate: None,
            },
        );
    }

    TargetsFile {
        schema_version: Some(bullseye::schema::CURRENT_SCHEMA_VERSION),
        last_evaluated: None,
        release_surface: Vec::new(),
        targets,
    }
}
