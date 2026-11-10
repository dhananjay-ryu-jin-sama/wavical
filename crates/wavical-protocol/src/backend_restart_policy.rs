#![forbid(unsafe_code)]

//! Pure decision functions for backend crash recovery and restart throttling.
//!
//! Uses a monotonic clock (`std::time::Instant`) without any wall-clock or OS dependencies,
//! making the restart behavior 100% deterministically unit-testable at boundary conditions.

use std::time::{Duration, Instant};
use thiserror::Error;

/// Default thresholds from plan 1.10.
pub const DEFAULT_MAX_BACKEND_RESTART_RETRIES: usize = 3;
pub const DEFAULT_BACKEND_RESTART_WINDOW_SECS: u64 = 60;

/// Error returned when the backend crashes repeatedly and exceeds the retry limit.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error(
    "backend crashed {crash_count} times within {window_secs}s, exceeding maximum retry limit ({max_retries}). Giving up to protect system stability."
)]
pub struct BackendCrashLimitExceeded {
    pub crash_count: usize,
    pub window_secs: u64,
    pub max_retries: usize,
}

/// Pure policy for backend crash recovery and restart throttling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRestartPolicy {
    max_retries: usize,
    window: Duration,
    cooldown: Duration,
    crash_timestamps: Vec<Instant>,
}

impl Default for BackendRestartPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendRestartPolicy {
    /// Create a new backend restart policy with plan 1.10 thresholds:
    /// 3 retries within 60 seconds.
    pub fn new() -> Self {
        Self {
            max_retries: DEFAULT_MAX_BACKEND_RESTART_RETRIES,
            window: Duration::from_secs(DEFAULT_BACKEND_RESTART_WINDOW_SECS),
            cooldown: Duration::from_millis(500),
            crash_timestamps: Vec::new(),
        }
    }

    /// Create with custom thresholds for testing.
    pub fn with_thresholds(max_retries: usize, window: Duration, cooldown: Duration) -> Self {
        Self {
            max_retries,
            window,
            cooldown,
            crash_timestamps: Vec::new(),
        }
    }

    /// Record a crash and evaluate whether a restart is permitted at the current monotonic instant.
    ///
    /// If fewer than `max_retries` crashes have occurred within the sliding `window`,
    /// records the crash and returns the recommended cooldown before restarting.
    /// If the retry limit is exceeded, returns `Err(BackendCrashLimitExceeded)`.
    pub fn record_crash_and_check(
        &mut self,
        now: Instant,
    ) -> Result<Duration, BackendCrashLimitExceeded> {
        // Prune entries outside the sliding window
        self.prune_old_crashes(now);

        if self.crash_timestamps.len() >= self.max_retries {
            return Err(BackendCrashLimitExceeded {
                crash_count: self.crash_timestamps.len() + 1,
                window_secs: self.window.as_secs(),
                max_retries: self.max_retries,
            });
        }

        self.crash_timestamps.push(now);
        Ok(self.cooldown)
    }

    /// Reset the crash counter when the backend has been running stably.
    pub fn record_stable_run(&mut self) {
        self.crash_timestamps.clear();
    }

    /// Return the number of crashes recorded in the current active window.
    pub fn crash_count(&self, now: Instant) -> usize {
        self.crash_timestamps
            .iter()
            .filter(|&&t| now >= t && now.duration_since(t) <= self.window)
            .count()
    }

    /// Prune crash records older than the sliding window.
    fn prune_old_crashes(&mut self, now: Instant) {
        let window = self.window;
        self.crash_timestamps
            .retain(|&t| now >= t && now.duration_since(t) <= window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restart_policy_allows_up_to_max_retries() {
        let mut policy = BackendRestartPolicy::new();
        let t0 = Instant::now();

        // 1st crash
        assert!(policy.record_crash_and_check(t0).is_ok());
        assert_eq!(policy.crash_count(t0), 1);

        // 2nd crash
        let t1 = t0 + Duration::from_secs(5);
        assert!(policy.record_crash_and_check(t1).is_ok());
        assert_eq!(policy.crash_count(t1), 2);

        // 3rd crash
        let t2 = t1 + Duration::from_secs(10);
        assert!(policy.record_crash_and_check(t2).is_ok());
        assert_eq!(policy.crash_count(t2), 3);

        // 4th crash within 60s -> Rejected!
        let t3 = t2 + Duration::from_secs(5);
        let err = policy.record_crash_and_check(t3).unwrap_err();
        assert_eq!(err.max_retries, 3);
        assert_eq!(err.window_secs, 60);
    }

    #[test]
    fn test_restart_policy_prunes_after_window_expires() {
        let mut policy = BackendRestartPolicy::new();
        let t0 = Instant::now();

        assert!(policy.record_crash_and_check(t0).is_ok());
        assert!(
            policy
                .record_crash_and_check(t0 + Duration::from_secs(10))
                .is_ok()
        );
        assert!(
            policy
                .record_crash_and_check(t0 + Duration::from_secs(20))
                .is_ok()
        );

        // At t0 + 61s, t0 has fallen out of the 60s window
        let t_future = t0 + Duration::from_secs(61);
        // Crash count should be 2 now
        assert_eq!(policy.crash_count(t_future), 2);

        // A new crash is allowed!
        assert!(policy.record_crash_and_check(t_future).is_ok());
        assert_eq!(policy.crash_count(t_future), 3);
    }

    #[test]
    fn test_restart_policy_reset_on_stable_run() {
        let mut policy = BackendRestartPolicy::new();
        let t0 = Instant::now();

        assert!(policy.record_crash_and_check(t0).is_ok());
        assert!(
            policy
                .record_crash_and_check(t0 + Duration::from_secs(5))
                .is_ok()
        );
        assert_eq!(policy.crash_count(t0 + Duration::from_secs(5)), 2);

        // Stable run signal
        policy.record_stable_run();
        assert_eq!(policy.crash_count(t0 + Duration::from_secs(5)), 0);

        // Can record 3 more crashes
        assert!(
            policy
                .record_crash_and_check(t0 + Duration::from_secs(10))
                .is_ok()
        );
        assert!(
            policy
                .record_crash_and_check(t0 + Duration::from_secs(11))
                .is_ok()
        );
        assert!(
            policy
                .record_crash_and_check(t0 + Duration::from_secs(12))
                .is_ok()
        );
    }
}
