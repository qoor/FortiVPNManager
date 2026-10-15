import Foundation
import FortiShared

/// Where the connection is right now (§5.4).
public enum ConnectionState: Sendable, Equatable {
    case idle
    case needsSetup([SetupItem])
    case connecting(AttemptContext)
    case connected(ConnectedContext)
    case waitingRetry(RetryWait)
    case waitingNetwork(NetworkWait)
    case disconnecting(DisconnectContext)
    case failed(ConnectionFailureReason)
}

/// Steps of §6.1 before openfortivpn reports milestones.
public enum AttemptStage: Sendable, Equatable {
    /// §6.1 1-2: waiting for the app's readiness check (config, Keychain, helper status, BuildID).
    case checkingReadiness
    /// §6.1 3: Gmail session is logging in and taking its snapshot.
    case preparingOTP
    /// §6.1 4+: `startTunnel` was issued; progress is in `milestone`.
    case tunnel
}

/// One connection attempt in progress.
public struct AttemptContext: Sendable, Equatable {
    public var attemptID: AttemptID
    public var reason: AttemptReason
    /// First attempt of the certificate-retry chain this attempt belongs to.
    public var chainID: AttemptID
    /// The chain already used its one immediate retry after a certificate rejection.
    public var certRetryUsed: Bool
    public var startedAt: Date
    public var stage: AttemptStage
    /// Furthest milestone reported by the helper for this attempt.
    public var milestone: Milestone?
    public var otpPromptedAt: Date?
    /// `submitOTP` was issued.
    public var otpSubmitted = false
    /// The Gmail session is open and owned by this attempt (it closes itself after `submitOTP`).
    var otpSessionOpen = false
    var otpSessionFailure: OTPSessionFailure?
    var otpFoundInJunk = false
    /// The network path changed while this attempt was running.
    public var pathChanged = false
    /// Stop the app requested for this attempt (`otpTimeout` / `attemptTimeout`); the exit is still classified.
    public var stopRequested: StopReason?
    /// `vpn.*` when the attempt began: the account its readiness check was issued for and the one
    /// `startTunnel` uses. A profile change while the attempt runs applies from the next attempt (§5.7).
    var profile: VPNProfile?

    init(
        attemptID: AttemptID,
        reason: AttemptReason,
        chainID: AttemptID,
        certRetryUsed: Bool,
        startedAt: Date,
        stage: AttemptStage
    ) {
        self.attemptID = attemptID
        self.reason = reason
        self.chainID = chainID
        self.certRetryUsed = certRetryUsed
        self.startedAt = startedAt
        self.stage = stage
    }
}

/// A tunnel that is up.
public struct ConnectedContext: Sendable, Equatable {
    public var attemptID: AttemptID
    public var since: Date
    public var tunnelIP: String?
    /// Next proactive reconnect, or `nil` when proactive reconnect is disabled.
    public var plannedReconnectAt: Date?
    /// Underlay recorded at `tunnelUp` (or the first one reported afterwards).
    public var underlay: Underlay?
}

/// Waiting for a timer before the next attempt.
public struct RetryWait: Sendable, Equatable {
    public var until: Date
    public var reason: RetryReason
}

/// Waiting for the network path to be satisfied (or for wake) before the next attempt.
public struct NetworkWait: Sendable, Equatable {
    public var reason: RetryReason
    /// Backoff to apply once the path is satisfied again; zero means "attempt immediately".
    public var delayAfterPath: TimeInterval
}

/// What happens once the run being stopped has exited.
public enum AfterStop: Sendable, Equatable {
    case idle
    case reconnect(AttemptReason)
    case resumeAfterSleep
    case terminate
}

/// A stop the app requested; completion is the helper's `.exited` event, never the XPC reply.
public struct DisconnectContext: Sendable, Equatable {
    public var attemptID: AttemptID
    public var reason: StopReason
    public var after: AfterStop
    /// Set when the stopped run had a tunnel up.
    public var connectedSince: Date?
}

/// Bookkeeping for a reducer-owned timer.
struct TimerSlot: Sendable, Equatable {
    var date: Date
    var generation: UInt64
    var fired = false
}

/// Wake that has not been evaluated yet (`didWake` received, `wakeSettled` pending).
struct PendingWake: Sendable, Equatable {
    var sleptFor: TimeInterval
}
