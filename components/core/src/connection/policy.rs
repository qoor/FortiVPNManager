//! Timing and counting rules.

use std::time::{Duration, SystemTime};

use super::ConnectionSettings;

/// The whole attempt, from the readiness check to the tunnel.
pub(super) const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(150);
/// From the OTP prompt to the code's submission.
pub(super) const OTP_DEADLINE: Duration = Duration::from_secs(50);
/// The wait after the first counted authentication failure, and the least gap after any before a
/// new attempt: FortiGate blocks the IP for a while after two failures.
pub(super) const AUTH_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(90);
/// Counted authentication failures in a row that stop the retries.
pub(super) const MAX_CONSECUTIVE_AUTH_FAILURES: u32 = 2;
pub(super) const SERVER_SESSION_END_DELAY: Duration = Duration::from_secs(5);
/// A session at least this long is stable: it clears the failure count, and a drop before it is
/// counted as an authentication failure.
pub(super) const STABLE_SESSION: Duration = Duration::from_secs(120);
const NETWORK_BACKOFF_INITIAL: Duration = Duration::from_secs(10);
const NETWORK_BACKOFF_MAX: Duration = Duration::from_secs(300);
/// Failures at login this soon after a wake are network errors.
pub(super) const WAKE_GRACE: Duration = Duration::from_secs(30);
/// How long the network may take to come back after a wake before the wake is judged anyway.
pub(super) const WAKE_SETTLE_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const WAKE_SETTLE_POLL: Duration = Duration::from_secs(1);
/// After the wake was judged without a network, how often to look for it.
pub(super) const LATE_SETTLE_POLL: Duration = Duration::from_secs(2);
/// The network path is looked at once it stayed unchanged this long.
pub(super) const PATH_SETTLE: Duration = Duration::from_secs(3);
pub(super) const LOCK_POLL: Duration = Duration::from_secs(15);
/// The window in which automatic sign-in attempts are limited, and the limit.
pub(super) const AUTH_ATTEMPT_WINDOW: Duration = Duration::from_secs(60 * 60);
pub(super) const MAX_AUTH_ATTEMPTS_PER_WINDOW: usize = 12;
/// A planned proactive reconnect is never closer than this when it is computed.
pub(super) const MINIMUM_PLANNED_LEAD: Duration = Duration::from_secs(60);
/// How long a quit waits for the tunnel to end.
pub(super) const QUIT_TIMEOUT: Duration = Duration::from_secs(12);
/// After registering the Helper or opening its approval settings, readiness is checked this
/// often for this long, to follow the approval.
pub(super) const APPROVAL_FOLLOW: Duration = Duration::from_secs(120);
pub(super) const APPROVAL_FOLLOW_POLL: Duration = Duration::from_secs(2);

/// The wait after the `failures`-th network failure in a row, doubling up to the maximum.
pub(super) fn network_backoff(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    (NETWORK_BACKOFF_INITIAL * (1 << doublings)).min(NETWORK_BACKOFF_MAX)
}

/// When to replace a session that came up at `connected_at`: the session's maximum less the
/// margin, never sooner than a minute from `now`. `None` when proactive reconnects are off.
pub(super) fn planned_reconnect(
    connected_at: SystemTime,
    settings: &ConnectionSettings,
    now: SystemTime,
) -> Option<SystemTime> {
    if !settings.proactive_reconnect {
        return None;
    }
    let lifetime_minutes =
        i64::from(settings.session_max_minutes) - i64::from(settings.proactive_margin_minutes);
    let lifetime = Duration::from_secs(lifetime_minutes.unsigned_abs() * 60);
    let planned = if lifetime_minutes >= 0 {
        connected_at + lifetime
    } else {
        connected_at - lifetime
    };
    Some(planned.max(now + MINIMUM_PLANNED_LEAD))
}

/// From `from` to `to`, nothing when `to` is earlier.
pub(super) fn elapsed(from: SystemTime, to: SystemTime) -> Duration {
    to.duration_since(from).unwrap_or(Duration::ZERO)
}
