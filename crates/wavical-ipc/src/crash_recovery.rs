//! Pure crash recovery decision logic for webview renderer process failure (WP6).
//!
//! Enforces:
//! - First crash -> Retry (reload once).
//! - Repeated crash within 30 seconds -> GiveUp (stop retrying and show error).
//! - Repeated crash after 30 seconds -> Retry (allow recovery if sufficient time has elapsed).

use std::time::{Duration, Instant};

/// Sliding time window within which repeated renderer crashes cause GiveUp.
pub const CRASH_RETRY_WINDOW: Duration = Duration::from_secs(30);

/// Action decided upon a webview renderer crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashRecoveryDecision {
    /// Reload the page.
    Retry,
    /// Stop retrying and show an error dialog.
    GiveUp,
}

/// Decide whether to retry (reload) or give up after a renderer process crash.
///
/// Inputs:
/// - `previous_crashes`: slice of timestamps representing past crash events.
/// - `now`: timestamp of the current crash event.
///
/// Output:
/// - `CrashRecoveryDecision::Retry` if this is the first crash, or if the most recent previous
///   crash occurred strictly greater than 30 seconds ago.
/// - `CrashRecoveryDecision::GiveUp` if the most recent previous crash occurred within the 30-second
///   window (i.e. `now - last_crash <= 30s`).
pub fn decide_crash_recovery(previous_crashes: &[Instant], now: Instant) -> CrashRecoveryDecision {
    decide_crash_recovery_with_window(previous_crashes, now, CRASH_RETRY_WINDOW)
}

/// Parameterized crash recovery decision function.
pub fn decide_crash_recovery_with_window(
    previous_crashes: &[Instant],
    now: Instant,
    window: Duration,
) -> CrashRecoveryDecision {
    if let Some(&last_crash) = previous_crashes.last() {
        if now <= last_crash {
            return CrashRecoveryDecision::GiveUp;
        }
        if now.duration_since(last_crash) <= window {
            return CrashRecoveryDecision::GiveUp;
        }
    }

    CrashRecoveryDecision::Retry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_crash_retries() {
        let now = Instant::now();
        let history = Vec::new();
        assert_eq!(
            decide_crash_recovery(&history, now),
            CrashRecoveryDecision::Retry,
            "First crash must always retry"
        );
    }

    #[test]
    fn test_second_crash_inside_window_gives_up() {
        let t0 = Instant::now();
        let history = vec![t0];

        // 10 seconds later (< 30s)
        let t1 = t0 + Duration::from_secs(10);
        assert_eq!(
            decide_crash_recovery(&history, t1),
            CrashRecoveryDecision::GiveUp,
            "Crash occurring 10s after initial crash must give up"
        );

        // 29 seconds and 999 ms later (< 30s)
        let t2 = t0 + Duration::from_millis(29_999);
        assert_eq!(
            decide_crash_recovery(&history, t2),
            CrashRecoveryDecision::GiveUp,
            "Crash occurring 29.999s after initial crash must give up"
        );
    }

    #[test]
    fn test_second_crash_exactly_30_seconds_gives_up() {
        let t0 = Instant::now();
        let history = vec![t0];

        // Exactly 30 seconds boundary
        let t1 = t0 + Duration::from_secs(30);
        assert_eq!(
            decide_crash_recovery(&history, t1),
            CrashRecoveryDecision::GiveUp,
            "Crash occurring at exactly 30s boundary is within window and must give up"
        );
    }

    #[test]
    fn test_second_crash_outside_window_retries() {
        let t0 = Instant::now();
        let history = vec![t0];

        // 30 seconds and 1 millisecond later (> 30s)
        let t1 = t0 + Duration::from_millis(30_001);
        assert_eq!(
            decide_crash_recovery(&history, t1),
            CrashRecoveryDecision::Retry,
            "Crash occurring at 30.001s is outside 30s window and must retry"
        );

        // 60 seconds later (> 30s)
        let t2 = t0 + Duration::from_secs(60);
        assert_eq!(
            decide_crash_recovery(&history, t2),
            CrashRecoveryDecision::Retry,
            "Crash occurring 60s after initial crash must retry"
        );
    }

    #[test]
    fn test_sequence_of_crashes() {
        let t0 = Instant::now();
        let mut history = Vec::new();

        // 1. Initial crash -> Retry
        assert_eq!(
            decide_crash_recovery(&history, t0),
            CrashRecoveryDecision::Retry
        );
        history.push(t0);

        // 2. Next crash 45s later (> 30s) -> Retry
        let t1 = t0 + Duration::from_secs(45);
        assert_eq!(
            decide_crash_recovery(&history, t1),
            CrashRecoveryDecision::Retry
        );
        history.push(t1);

        // 3. Next crash 15s after t1 (< 30s) -> GiveUp
        let t2 = t1 + Duration::from_secs(15);
        assert_eq!(
            decide_crash_recovery(&history, t2),
            CrashRecoveryDecision::GiveUp
        );
    }

    #[test]
    fn test_clock_anomaly_or_equal_timestamps_gives_up() {
        let t0 = Instant::now();
        let history = vec![t0];

        // Equal timestamp
        assert_eq!(
            decide_crash_recovery(&history, t0),
            CrashRecoveryDecision::GiveUp
        );
    }
}
