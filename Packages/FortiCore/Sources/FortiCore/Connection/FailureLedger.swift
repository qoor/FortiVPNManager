import Foundation

/// Authentication failure bookkeeping for one `username@host`, persisted in `state.json` so that the
/// retry policy survives relaunches (§5.7, §6.2). The reducer emits `.persistLedger` on every change.
public struct FailureLedger: Sendable, Equatable, Codable {
    public var profileKey: String
    /// Counted authentication failures since the last stable session or manual action.
    public var consecutiveAuthFailures: Int
    public var lastAuthFailureAt: Date?
    public var lastAuthFailureReason: ConnectionFailureReason?
    /// When attempts reached the gateway (i.e. credentials were sent), oldest first, within the last hour.
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

    /// No new attempt may start before this instant (90 s after the last authentication failure).
    public var cooldownEnd: Date? {
        lastAuthFailureAt?.addingTimeInterval(RetryPolicy.authFailureRetryDelay)
    }

    /// Number of authentication attempts within the rolling window ending at `now`.
    public func authAttempts(asOf now: Date) -> Int {
        let start = now.addingTimeInterval(-RetryPolicy.authAttemptWindow)
        return recentAuthAttempts.lazy.filter { $0 > start }.count
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

    /// A stable session (>= 2 minutes) proves the credentials work.
    mutating func resetConsecutiveFailures() {
        consecutiveAuthFailures = 0
    }

    /// "Connect" / "Reconnect Now": the counter and the rolling history start over. The time of the last
    /// failure is kept so the 90 s gap is still honoured.
    mutating func resetForManualAction() {
        consecutiveAuthFailures = 0
        recentAuthAttempts = []
    }
}
