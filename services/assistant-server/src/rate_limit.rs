//! Per-principal rate limiting for the paid voice endpoints.
//!
//! Size limits bound how expensive one request can be; this bounds how many a
//! caller can make. Without it a loop over `/v1/voice/speak` bills the account
//! at whatever rate the network allows.
//!
//! Deliberately a fixed-window counter in a `Mutex<HashMap>` rather than a
//! token bucket in Redis. The repository rule is to introduce no infrastructure
//! without a demonstrated need, and this needs to survive a single process
//! only: the limits exist to stop a runaway client and an obvious abuse loop,
//! not to coordinate a fleet. If the server is ever replicated this becomes
//! per-instance, which is a real limitation and is documented as one.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use uuid::Uuid;

/// One caller's usage of one endpoint within the current window.
#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    count: u32,
}

/// A fixed-window limiter keyed by principal and bucket name.
#[derive(Debug)]
pub struct RateLimiter {
    window: Duration,
    limit: u32,
    // A mutex is fine here: the critical section is a hash lookup and an
    // integer bump, far cheaper than the network call it guards.
    state: Mutex<HashMap<(Uuid, &'static str), Window>>,
}

/// What the caller should do about a request that was not allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimited {
    /// Seconds until the window resets, for `Retry-After`.
    pub retry_after_secs: u64,
}

impl RateLimiter {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            window,
            limit,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Records an attempt, returning `Err` when the caller is over the limit.
    ///
    /// `bucket` separates endpoints so a burst of transcriptions does not
    /// exhaust a user's synthesis budget.
    pub fn check(&self, user: Uuid, bucket: &'static str) -> Result<(), RateLimited> {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // Opportunistically drop windows that have expired, so an idle server
        // does not accumulate an entry per user it has ever seen.
        if state.len() > 1024 {
            state.retain(|_, w| now.duration_since(w.started) < self.window);
        }

        let entry = state.entry((user, bucket)).or_insert(Window {
            started: now,
            count: 0,
        });

        if now.duration_since(entry.started) >= self.window {
            *entry = Window {
                started: now,
                count: 0,
            };
        }

        if entry.count >= self.limit {
            let elapsed = now.duration_since(entry.started);
            return Err(RateLimited {
                retry_after_secs: self.window.saturating_sub(elapsed).as_secs().max(1),
            });
        }

        entry.count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_refuses() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        let user = Uuid::new_v4();

        for _ in 0..3 {
            assert!(limiter.check(user, "stt").is_ok());
        }
        assert!(limiter.check(user, "stt").is_err());
    }

    #[test]
    fn one_users_budget_is_not_spent_by_another() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        assert!(limiter.check(a, "stt").is_ok());
        assert!(limiter.check(a, "stt").is_err());
        // b must be unaffected by a exhausting its own window.
        assert!(limiter.check(b, "stt").is_ok());
    }

    #[test]
    fn buckets_are_independent() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let user = Uuid::new_v4();

        assert!(limiter.check(user, "stt").is_ok());
        assert!(limiter.check(user, "stt").is_err());
        // Exhausting transcription must not block synthesis.
        assert!(limiter.check(user, "tts").is_ok());
    }

    #[test]
    fn the_window_reopens_once_it_has_passed() {
        // A window short enough to elapse within the test, so this asserts real
        // behaviour rather than mocked time.
        let limiter = RateLimiter::new(1, Duration::from_millis(50));
        let user = Uuid::new_v4();

        assert!(limiter.check(user, "stt").is_ok());
        assert!(limiter.check(user, "stt").is_err());

        std::thread::sleep(Duration::from_millis(60));
        assert!(limiter.check(user, "stt").is_ok());
    }

    #[test]
    fn a_refusal_reports_when_to_retry() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let user = Uuid::new_v4();

        limiter.check(user, "stt").expect("first is allowed");
        let err = limiter.check(user, "stt").expect_err("second is refused");
        assert!(err.retry_after_secs >= 1);
        assert!(err.retry_after_secs <= 60);
    }
}
