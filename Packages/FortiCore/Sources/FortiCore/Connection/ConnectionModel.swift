import Foundation
import FortiShared

/// Everything the connection reducer knows. Only `ConnectionReducer.reduce` mutates it.
public struct ConnectionModel: Sendable, Equatable {
    public internal(set) var desired: DesiredState = .disconnected
    public internal(set) var state: ConnectionState = .idle
    public internal(set) var settings: ConnectionSettings
    public internal(set) var profile: VPNProfile
    public internal(set) var readiness: Readiness = .unknown
    public internal(set) var ledger: FailureLedger
    /// Most recent classified failure (shown as the one-line reason).
    public internal(set) var lastFailure: ConnectionFailureReason?
    public internal(set) var path: PathSnapshot?
    public internal(set) var isSleeping = false
    public internal(set) var lastWakeAt: Date?
    public internal(set) var quitRequestedAt: Date?
    /// `terminateApp` was emitted; every later event is ignored.
    public internal(set) var isTerminated = false

    var hasLaunched = false
    var pendingWake: PendingWake?
    var networkFailureStreak = 0
    var lastFailureAttemptID: AttemptID?
    /// Profile of the last `startTunnel`, i.e. of the openfortivpn run whose events are accepted now.
    /// Differs from `profile` when `vpn.*` changed while that run was in flight.
    var runProfile: VPNProfile?
    var timers: [ConnectionTimer: TimerSlot] = [:]
    var nextTimerGeneration: UInt64 = 0
    var activityHeld = false
    var attemptIDs: AttemptIDGenerator

    /// - Parameters:
    ///   - profile: `vpn.*` from `config.yaml`, including `trusted_certs`.
    ///   - settings: `connection.*` from `config.yaml`.
    ///   - attemptIDSeed: random per launch in the app, fixed in tests.
    public init(profile: VPNProfile, settings: ConnectionSettings = ConnectionSettings(), attemptIDSeed: UUID) {
        self.profile = profile
        self.settings = settings
        self.ledger = FailureLedger(profileKey: profile.profileKey)
        self.attemptIDs = AttemptIDGenerator(seed: attemptIDSeed)
    }

    /// Attempt whose helper events are currently accepted, if any.
    public var activeAttemptID: AttemptID? {
        switch state {
        case let .connecting(context) where context.stage == .tunnel: context.attemptID
        case let .connected(context): context.attemptID
        case let .disconnecting(context): context.attemptID
        default: nil
        }
    }

    /// Timers the current state calls for. `reconcile` diffs this against `timers`.
    func wantedTimers() -> [ConnectionTimer: Date] {
        var wanted: [ConnectionTimer: Date] = [:]
        switch state {
        case let .connecting(context) where context.stopRequested == nil:
            wanted[.attemptDeadline] = context.startedAt.addingTimeInterval(RetryPolicy.attemptTimeout)
            if context.stage == .tunnel, let prompted = context.otpPromptedAt, !context.otpSubmitted {
                wanted[.otpDeadline] = prompted.addingTimeInterval(RetryPolicy.otpDeadline)
            }
        case let .connected(context) where !isSleeping && pendingWake == nil:
            // Suspended while asleep and until the wake has been evaluated (§6.3), then re-armed.
            if let planned = context.plannedReconnectAt {
                wanted[.proactiveReconnect] = planned
            }
        case let .waitingRetry(wait):
            wanted[.retry] = wait.until
        default:
            break
        }
        if let quitRequestedAt, !isTerminated {
            wanted[.quitDeadline] = quitRequestedAt.addingTimeInterval(RetryPolicy.quitTimeout)
        }
        return wanted
    }
}

extension ConnectionState {
    /// App Nap prevention (`beginActivity`) is held whenever the reducer has work in flight or a timer
    /// that must fire on time.
    var holdsActivity: Bool {
        switch self {
        case .connecting, .connected, .waitingRetry, .waitingNetwork, .disconnecting: true
        case .idle, .needsSetup, .failed: false
        }
    }
}
