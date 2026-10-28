#![forbid(unsafe_code)]

//! Pure decision functions for engine idle shutdown and crash recovery.
//!
//! Uses a monotonic clock (`std::time::Instant`) without any wall-clock or OS dependencies,
//! making the lifecycle behavior 100% deterministically unit-testable at boundary conditions.

use std::time::{Duration, Instant};
use thiserror::Error;

/// Hard ceiling and default limits for idle timeout and crash retry.
pub const DEFAULT_IDLE_MINUTES: u32 = 10;
pub const MAX_IDLE_MINUTES: u32 = 120;
pub const DEFAULT_MAX_CRASHES_IN_WINDOW: usize = 3;
pub const DEFAULT_CRASH_WINDOW: Duration = Duration::from_secs(60);
pub const DEFAULT_CRASH_COOLDOWN: Duration = Duration::from_millis(500);

/// Error returned when the engine crashes repeatedly and exceeds the retry limit.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error(
    "engine crashed {crash_count} times within {window:?}, exceeding retry limit. Halting engine to protect system stability."
)]
pub struct CrashLimitExceeded {
    pub crash_count: usize,
    pub window: Duration,
}

/// Pure policy for determining whether an active engine should be shut down due to inactivity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleShutdownPolicy {
    idle_timeout: Duration,
    last_activity: Instant,
}

impl IdleShutdownPolicy {
    /// Create a new idle policy initialized with the given timeout in minutes and start time.
    pub fn new(idle_minutes: u32, now: Instant) -> Self {
        let clamped_minutes = idle_minutes.clamp(1, MAX_IDLE_MINUTES);
        Self {
            idle_timeout: Duration::from_secs((clamped_minutes as u64) * 60),
            last_activity: now,
        }
    }

    /// Record an active AI request, resetting the idle countdown.
    pub fn record_activity(&mut self, now: Instant) {
        if now >= self.last_activity {
            self.last_activity = now;
        }
    }

    /// Check if the engine has been idle for longer than the configured timeout.
    pub fn is_idle(&self, now: Instant) -> bool {
        if now < self.last_activity {
            return false;
        }
        now.duration_since(self.last_activity) >= self.idle_timeout
    }

    /// Return the remaining time before idle shutdown would trigger.
    pub fn remaining_time(&self, now: Instant) -> Duration {
        if now < self.last_activity {
            return self.idle_timeout;
        }
        let elapsed = now.duration_since(self.last_activity);
        self.idle_timeout.saturating_sub(elapsed)
    }

    /// Return the configured idle timeout duration.
    pub fn timeout(&self) -> Duration {
        self.idle_timeout
    }
}

/// Pure policy for engine crash recovery and restart throttling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashRecoveryPolicy {
    max_crashes: usize,
    window: Duration,
    cooldown: Duration,
    crash_timestamps: Vec<Instant>,
}

impl CrashRecoveryPolicy {
    /// Create a new crash recovery policy with default thresholds (3 crashes in 60s).
    pub fn new() -> Self {
        Self {
            max_crashes: DEFAULT_MAX_CRASHES_IN_WINDOW,
            window: DEFAULT_CRASH_WINDOW,
            cooldown: DEFAULT_CRASH_COOLDOWN,
            crash_timestamps: Vec::new(),
        }
    }

    /// Create with custom thresholds for testing or fine-tuning.
    pub fn with_thresholds(max_crashes: usize, window: Duration, cooldown: Duration) -> Self {
        Self {
            max_crashes,
            window,
            cooldown,
            crash_timestamps: Vec::new(),
        }
    }

    /// Evaluate a crash occurrence at monotonic time `now`.
    ///
    /// Returns:
    /// - `Ok(cooldown)`: restart is permitted after waiting `cooldown`.
    /// - `Err(CrashLimitExceeded)`: too many crashes occurred within the rolling window; do not restart.
    pub fn record_crash_and_check_restart(
        &mut self,
        now: Instant,
    ) -> Result<Duration, CrashLimitExceeded> {
        // 1. Prune crashes older than the rolling window
        self.crash_timestamps
            .retain(|&t| now >= t && now.duration_since(t) < self.window);

        // 2. Check if the threshold is already reached
        if self.crash_timestamps.len() >= self.max_crashes {
            return Err(CrashLimitExceeded {
                crash_count: self.crash_timestamps.len(),
                window: self.window,
            });
        }

        // 3. Record this crash
        self.crash_timestamps.push(now);

        Ok(self.cooldown)
    }

    /// Reset crash counters when the engine has run stably for the window duration.
    pub fn reset_if_stable(&mut self, uptime: Duration) {
        if uptime >= self.window {
            self.crash_timestamps.clear();
        }
    }

    /// Return the current number of tracked crashes in the window.
    pub fn active_crash_count(&self) -> usize {
        self.crash_timestamps.len()
    }
}

impl Default for CrashRecoveryPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idle_shutdown_policy_boundaries() {
        let t0 = Instant::now();
        let mut policy = IdleShutdownPolicy::new(10, t0); // 10 minutes = 600s
        assert_eq!(policy.timeout(), Duration::from_secs(600));

        // Just started: not idle
        assert!(!policy.is_idle(t0));
        assert_eq!(policy.remaining_time(t0), Duration::from_secs(600));

        // 599 seconds: not idle
        let t_599 = t0 + Duration::from_secs(599);
        assert!(!policy.is_idle(t_599));
        assert_eq!(policy.remaining_time(t_599), Duration::from_secs(1));

        // Exactly 600 seconds: idle!
        let t_600 = t0 + Duration::from_secs(600);
        assert!(policy.is_idle(t_600));
        assert_eq!(policy.remaining_time(t_600), Duration::ZERO);

        // 601 seconds: idle
        let t_601 = t0 + Duration::from_secs(601);
        assert!(policy.is_idle(t_601));

        // Activity occurs at 500s: resets timer
        let t_500 = t0 + Duration::from_secs(500);
        policy.record_activity(t_500);
        assert!(!policy.is_idle(t_600)); // 100s after activity, not idle
        assert_eq!(policy.remaining_time(t_600), Duration::from_secs(500));

        // Exactly 500 + 600 = 1100s: idle again!
        let t_1100 = t0 + Duration::from_secs(1100);
        assert!(policy.is_idle(t_1100));
    }

    #[test]
    fn test_crash_recovery_policy_retry_limiter() {
        let t0 = Instant::now();
        let mut policy = CrashRecoveryPolicy::with_thresholds(
            3,
            Duration::from_secs(60),
            Duration::from_millis(500),
        );

        // 1st crash at t = 10s: permitted
        let res1 = policy.record_crash_and_check_restart(t0 + Duration::from_secs(10));
        assert_eq!(res1, Ok(Duration::from_millis(500)));
        assert_eq!(policy.active_crash_count(), 1);

        // 2nd crash at t = 20s: permitted
        let res2 = policy.record_crash_and_check_restart(t0 + Duration::from_secs(20));
        assert_eq!(res2, Ok(Duration::from_millis(500)));
        assert_eq!(policy.active_crash_count(), 2);

        // 3rd crash at t = 30s: permitted (hits 3rd count)
        let res3 = policy.record_crash_and_check_restart(t0 + Duration::from_secs(30));
        assert_eq!(res3, Ok(Duration::from_millis(500)));
        assert_eq!(policy.active_crash_count(), 3);

        // 4th crash at t = 40s (within 60s window): REJECTED
        let res4 = policy.record_crash_and_check_restart(t0 + Duration::from_secs(40));
        assert_eq!(
            res4,
            Err(CrashLimitExceeded {
                crash_count: 3,
                window: Duration::from_secs(60),
            })
        );

        // After window passes (t = 80s): 1st crash at t=10s is pruned (>60s ago)!
        // Remaining in window: t=20s, t=30s (2 crashes). So a new crash is permitted!
        let res5 = policy.record_crash_and_check_restart(t0 + Duration::from_secs(80));
        assert_eq!(res5, Ok(Duration::from_millis(500)));
    }

    #[test]
    fn test_crash_recovery_policy_reset_on_stable_run() {
        let t0 = Instant::now();
        let mut policy = CrashRecoveryPolicy::with_thresholds(
            3,
            Duration::from_secs(60),
            Duration::from_millis(500),
        );

        policy
            .record_crash_and_check_restart(t0 + Duration::from_secs(10))
            .unwrap();
        policy
            .record_crash_and_check_restart(t0 + Duration::from_secs(20))
            .unwrap();
        assert_eq!(policy.active_crash_count(), 2);

        // Stable run for 65s (>= 60s window)
        policy.reset_if_stable(Duration::from_secs(65));
        assert_eq!(policy.active_crash_count(), 0);
    }
}
