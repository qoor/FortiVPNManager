import Foundation

/// Timing and counting rules of §6 in one place. All durations are in seconds.
public enum RetryPolicy {
    /// Whole-attempt deadline, owned by the app (§5.2 watchdog table).
    public static let attemptTimeout: TimeInterval = 150
    /// OTP prompt to submission deadline, owned by the app.
    public static let otpDeadline: TimeInterval = 50
    /// Wait after the first counted authentication failure; also the minimum gap after any
    /// authentication failure before a new attempt (FortiGate blocks for 60 s after two failures).
    public static let authFailureRetryDelay: TimeInterval = 90
    /// Consecutive counted authentication failures that end in `failed`.
    public static let maxConsecutiveAuthFailures = 2
    /// Delay before reconnecting after the server ended a long session.
    public static let serverSessionEndDelay: TimeInterval = 5
    /// A session at least this long is a "real" session: it resets the failure counter, and a drop
    /// before this counts as an authentication failure.
    public static let stableSessionDuration: TimeInterval = 120
    /// First network backoff step; doubles per consecutive network failure.
    public static let networkBackoffInitial: TimeInterval = 10
    /// Upper bound of the network backoff.
    public static let networkBackoffMax: TimeInterval = 300
    /// Failures this soon after a wake are treated as network errors.
    public static let wakeGracePeriod: TimeInterval = 30
    /// How long the app waits for the path and Router after a wake before sending `wakeSettled`.
    public static let wakeSettleTimeout: TimeInterval = 30
    /// Rolling window for the authentication attempt cap.
    public static let authAttemptWindow: TimeInterval = 60 * 60
    /// Maximum authentication attempts allowed within `authAttemptWindow`.
    public static let maxAuthAttemptsPerWindow = 12
    /// `plannedReconnectAt` is never closer than this to "now" when it is (re)computed.
    public static let minimumPlannedLeadTime: TimeInterval = 60
    /// Upper bound for a graceful quit before the app terminates anyway.
    public static let quitTimeout: TimeInterval = 12

    /// Backoff for the `failures`-th consecutive network failure (1-based): 10, 20, 40, ... capped at 300.
    public static func networkBackoff(afterFailures failures: Int) -> TimeInterval {
        guard failures > 1 else { return networkBackoffInitial }
        let exponent = min(failures - 1, 16)
        return min(networkBackoffInitial * Double(1 << exponent), networkBackoffMax)
    }

    /// `connectedAt + session_max − margin`, never earlier than `now + minimumPlannedLeadTime`.
    /// `nil` when proactive reconnect is disabled.
    public static func plannedReconnect(connectedAt: Date, settings: ConnectionSettings, now: Date) -> Date? {
        guard settings.proactiveReconnectEnabled else { return nil }
        let lifetime = TimeInterval(settings.sessionMaxMinutes - settings.proactiveMarginMinutes) * 60
        let planned = connectedAt.addingTimeInterval(lifetime)
        return max(planned, now.addingTimeInterval(minimumPlannedLeadTime))
    }
}
