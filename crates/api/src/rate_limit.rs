//! In-process rate limiting for credential endpoints.
//!
//! Deliberately simple: Elrond is a single-process deployment, so a shared
//! in-memory map is sufficient and avoids a Redis dependency. The counters are
//! keyed by client address and scope, use a fixed window, and are pruned
//! opportunistically so the map cannot grow without bound.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Number of entries tolerated before a prune sweep runs.
const PRUNE_THRESHOLD: usize = 4096;

/// A fixed-window counter for one key.
#[derive(Debug, Clone, Copy)]
struct Window {
    /// When the current window opened.
    started_at: Instant,
    /// Attempts recorded in the current window.
    hits: u32,
}

/// Outcome of a rate-limit check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The request may proceed.
    Allowed,
    /// The request must be refused.
    Limited {
        /// Seconds until the window resets.
        retry_after_seconds: u64,
    },
}

/// Counts attempts per client and scope.
#[derive(Debug, Default)]
pub struct RateLimiter {
    windows: Mutex<HashMap<(String, &'static str), Window>>,
}

impl RateLimiter {
    /// Creates an empty limiter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an attempt and decides whether it is allowed.
    pub fn check(
        &self,
        client: &str,
        scope: &'static str,
        limit: u32,
        window: Duration,
    ) -> Decision {
        self.check_at(client, scope, limit, window, Instant::now())
    }

    /// Clears the counter for a key.
    ///
    /// Called after a successful sign-in so a legitimate user who mistyped their
    /// password several times is not left throttled.
    pub fn reset(&self, client: &str, scope: &'static str) {
        let mut windows = self.windows.lock().expect("rate limiter lock");
        windows.remove(&(client.to_owned(), scope));
    }

    /// Injectable-time variant, used by the tests.
    fn check_at(
        &self,
        client: &str,
        scope: &'static str,
        limit: u32,
        window: Duration,
        now: Instant,
    ) -> Decision {
        let mut windows = self.windows.lock().expect("rate limiter lock");

        if windows.len() >= PRUNE_THRESHOLD {
            windows.retain(|_, entry| now.duration_since(entry.started_at) < window);
        }

        let key = (client.to_owned(), scope);
        let entry = windows.entry(key).or_insert(Window {
            started_at: now,
            hits: 0,
        });

        if now.duration_since(entry.started_at) >= window {
            *entry = Window {
                started_at: now,
                hits: 0,
            };
        }

        if entry.hits >= limit {
            let elapsed = now.duration_since(entry.started_at);
            let remaining = window.saturating_sub(elapsed);
            return Decision::Limited {
                // Round up so a caller that waits exactly this long is inside the
                // next window rather than one millisecond short of it.
                retry_after_seconds: remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0),
            };
        }

        entry.hits += 1;
        Decision::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(300);

    #[test]
    fn attempts_within_the_limit_are_allowed() {
        let limiter = RateLimiter::new();
        for _ in 0..3 {
            assert_eq!(
                limiter.check("198.51.100.7", "sign_in", 3, WINDOW),
                Decision::Allowed
            );
        }
    }

    #[test]
    fn the_attempt_after_the_limit_is_refused() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        for _ in 0..3 {
            limiter.check_at("198.51.100.7", "sign_in", 3, WINDOW, start);
        }

        let decision = limiter.check_at("198.51.100.7", "sign_in", 3, WINDOW, start);
        assert_eq!(
            decision,
            Decision::Limited {
                retry_after_seconds: 300
            }
        );
    }

    #[test]
    fn clients_are_counted_independently() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        for _ in 0..3 {
            limiter.check_at("198.51.100.7", "sign_in", 3, WINDOW, start);
        }
        assert_eq!(
            limiter.check_at("198.51.100.8", "sign_in", 3, WINDOW, start),
            Decision::Allowed,
            "one client must not throttle another"
        );
    }

    #[test]
    fn scopes_are_counted_independently() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        for _ in 0..3 {
            limiter.check_at("198.51.100.7", "sign_in", 3, WINDOW, start);
        }
        assert_eq!(
            limiter.check_at("198.51.100.7", "setup", 3, WINDOW, start),
            Decision::Allowed
        );
    }

    #[test]
    fn the_window_resets_once_it_elapses() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        for _ in 0..3 {
            limiter.check_at("198.51.100.7", "sign_in", 3, WINDOW, start);
        }
        assert!(matches!(
            limiter.check_at("198.51.100.7", "sign_in", 3, WINDOW, start),
            Decision::Limited { .. }
        ));

        let later = start + WINDOW;
        assert_eq!(
            limiter.check_at("198.51.100.7", "sign_in", 3, WINDOW, later),
            Decision::Allowed
        );
    }

    #[test]
    fn retry_after_shrinks_as_the_window_drains() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        for _ in 0..1 {
            limiter.check_at("198.51.100.7", "sign_in", 1, WINDOW, start);
        }

        let decision = limiter.check_at(
            "198.51.100.7",
            "sign_in",
            1,
            WINDOW,
            start + Duration::from_secs(60),
        );
        assert_eq!(
            decision,
            Decision::Limited {
                retry_after_seconds: 240
            }
        );
    }

    #[test]
    fn a_successful_sign_in_clears_the_counter() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        for _ in 0..3 {
            limiter.check_at("198.51.100.7", "sign_in", 3, WINDOW, start);
        }
        limiter.reset("198.51.100.7", "sign_in");
        assert_eq!(
            limiter.check_at("198.51.100.7", "sign_in", 3, WINDOW, start),
            Decision::Allowed
        );
    }

    #[test]
    fn stale_entries_are_pruned_so_the_map_cannot_grow_forever() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        for index in 0..PRUNE_THRESHOLD {
            limiter.check_at(&format!("10.0.0.{index}"), "sign_in", 5, WINDOW, start);
        }
        assert_eq!(limiter.windows.lock().expect("lock").len(), PRUNE_THRESHOLD);

        // One more check after the window has passed should sweep the stale keys.
        limiter.check_at("192.0.2.1", "sign_in", 5, WINDOW, start + WINDOW);
        assert_eq!(
            limiter.windows.lock().expect("lock").len(),
            1,
            "expired windows should have been pruned"
        );
    }
}
