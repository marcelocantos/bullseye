// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Shared TTL policy for the process-global caches (🎯T78.1).
//!
//! Bullseye's caches were written for a process that lived exactly as
//! long as one agent session. Serving MCP from a supervised daemon
//! removed that ceiling, so entries could be served — and held — for
//! weeks. A TTL restores a bound that does not depend on the transport:
//! an entry older than [`TTL`] is a miss, and the next interaction
//! reloads it.
//!
//! Two distinct jobs, deliberately one policy:
//!
//! - **Correctness**, for [`crate::resolve`], whose workspace scan has
//!   no other validation. Without a TTL a repo cloned after the daemon
//!   started stays invisible until restart.
//! - **Eviction**, for [`crate::id_alloc`] and [`crate::store`], which
//!   are already exact (a git ref fingerprint and an mtime
//!   respectively) but grew one entry per repo touched, forever.
//!
//! Expiry is lazy: swept on insert rather than by a background task, so
//! there is no timer to own and a quiet daemon costs nothing.
//!
//! Deliberately keyed on nothing but elapsed time. The obvious
//! alternative — drop a directory's entry once its last connected
//! client goes away — reads well today and does not survive contact
//! with the roadmap: MCP2 is stateless, so there is no session whose
//! end could carry the invalidation, and no client identity to count.
//! A TTL is transport-independent: it means the same thing under
//! stdio, under a session-oriented HTTP daemon, and under a stateless
//! one.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Default lifetime of a cached entry.
///
/// Five minutes is far below any agent session, so a stale answer
/// cannot persist across the work that would notice it, and far above
/// a burst of tool calls, so the memo still does its job. The caches
/// this bounds are cheap to rebuild — a directory walk and a git log —
/// which is why the number can be conservative.
pub const TTL: Duration = Duration::from_secs(300);

/// Environment override for [`TTL`], in whole seconds.
///
/// Exists so the expiry path is testable — five minutes is not
/// something a test suite can wait out, and a policy that is only
/// tested at the helper level is a policy nobody has watched actually
/// evict anything. Also gives an operator a knob if a daemon ever
/// wants a different bound.
pub const TTL_ENV: &str = "BULLSEYE_CACHE_TTL_SECS";

/// The effective TTL. Read per call rather than memoised: it is dwarfed
/// by the directory walks and git subprocesses it guards, and memoising
/// it would make the override untestable in-process.
pub fn ttl() -> Duration {
    std::env::var(TTL_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(TTL)
}

/// True when `stamped` is older than the effective TTL.
pub fn expired(stamped: Instant) -> bool {
    stamped.elapsed() > ttl()
}

/// Drop every expired entry. Called on insert, so a cache is bounded by
/// what has been touched within [`TTL`] rather than by everything ever
/// touched.
pub fn sweep<K: Eq + Hash + Clone, V>(map: &mut HashMap<K, (Instant, V)>) {
    map.retain(|_, (stamped, _)| !expired(*stamped));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_entry_is_not_expired() {
        assert!(!expired(Instant::now()));
    }

    #[test]
    fn sweep_drops_only_the_expired() {
        let mut map: HashMap<&str, (Instant, u8)> = HashMap::new();
        let stale = Instant::now()
            .checked_sub(ttl() + Duration::from_secs(1))
            .expect("clock supports the offset");
        map.insert("old", (stale, 1));
        map.insert("new", (Instant::now(), 2));
        sweep(&mut map);
        assert!(!map.contains_key("old"), "expired entry must be evicted");
        assert!(map.contains_key("new"), "live entry must survive");
    }
}
