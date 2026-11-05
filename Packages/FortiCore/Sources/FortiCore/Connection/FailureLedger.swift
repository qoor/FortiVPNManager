import Foundation

/// Authentication failure bookkeeping for one `username@host`, persisted in `state.json` so that the
/// retry policy survives relaunches. The reducer emits `.persistLedger` on every change.
public struct FailureLedger: Sendable, Equatable, Codable {
    public var profileKey: String
    /// Counted authentication failures since the last stable session or manual action.
    public var consecutiveAuthFailures: Int
    public var lastAuthFailureAt: Date?
    public var lastAuthFailureReason: ConnectionFailureReason?
    /// When attempts reached the gateway (i.e. credentials were sent), oldest first, within the rolling window.
    public var recentAuthAttempts: [Date]

    public init(
        profileKey: String,
        consecutiveAuthFailures: Int = 0,
        lastAuthFailureAt: Date? = nil,
        lastAuthFailureReason: ConnectionFailureReason? = nil,
        recentAuthAttempts: [Date] = []
    ) {
        self.profileKey = profileKey
        self.consecutiveAuthFailures = consecutiveAuthFailures
        self.lastAuthFailureAt = lastAuthFailureAt
        self.lastAuthFailureReason = lastAuthFailureReason
        self.recentAuthAttempts = recentAuthAttempts
    }

    /// No new attempt may start before this instant.
    public var cooldownEnd: Date? {
        lastAuthFailureAt?.addingTimeInterval(RetryPolicy.authFailureRetryDelay)
    }

    /// Drops attempts that fell out of the rolling window.
    mutating func prune(asOf now: Date) {
        let start = now.addingTimeInterval(-RetryPolicy.authAttemptWindow)
        recentAuthAttempts.removeAll { $0 <= start }
    }

    mutating func recordAuthAttempt(at date: Date) {
        prune(asOf: date)
        recentAuthAttempts.append(date)
    }

    mutating func recordAuthFailure(_ reason: ConnectionFailureReason, at date: Date) {
        consecutiveAuthFailures += 1
        lastAuthFailureAt = date
        lastAuthFailureReason = reason
    }

    /// A stable session proves the credentials work.
    mutating func resetConsecutiveFailures() {
        consecutiveAuthFailures = 0
    }

    /// "Connect" / "Reconnect Now": the counter and the rolling history start over. The time of the last
    /// failure is kept so the cooldown is still honoured.
    mutating func resetForManualAction() {
        consecutiveAuthFailures = 0
        recentAuthAttempts = []
    }
}
