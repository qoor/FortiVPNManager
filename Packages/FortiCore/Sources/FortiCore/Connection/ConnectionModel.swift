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
    /// Calendar (time zone) the schedule's times are read in.
    public internal(set) var calendar: Calendar
    /// Since when the user has been away (screen locked, display or system asleep); `nil` while present.
    public internal(set) var awaySince: Date?
    /// Outside the scheduled hours the user was away for `schedule.awayMinutes`: no automatic
    /// reconnect until "Connect", "Reconnect Now" or the next scheduled start. The current
    /// connection is kept.
    public internal(set) var reconnectSuspended = false

    var hasLaunched = false
    /// Latest schedule start or end already acted on; the `.schedule` timer waits for the next one.
    var scheduleBoundary: Date?
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
    ///   - calendar: the user's calendar and time zone for the schedule (fixed in tests).
    public init(
        profile: VPNProfile,
        settings: ConnectionSettings = ConnectionSettings(),
        attemptIDSeed: UUID,
        calendar: Calendar = .current
    ) {
        self.profile = profile
        self.settings = settings
        self.ledger = FailureLedger(profileKey: profile.profileKey)
        self.attemptIDs = AttemptIDGenerator(seed: attemptIDSeed)
        self.calendar = calendar
    }

    /// `true` inside the scheduled hours, and always when there is no schedule.
    public func isWithinSchedule(at now: Date) -> Bool {
        settings.schedule?.isActive(at: now, calendar: calendar) ?? true
    }

    /// Whether the app may reconnect on its own right now: always inside the scheduled hours;
    /// outside them only while the user is present and reconnects were not suspended.
    public func automaticReconnectAllowed(at now: Date) -> Bool {
        isWithinSchedule(at: now) || (!reconnectSuspended && awaySince == nil)
    }

    /// Outside the scheduled hours, the user has been away for `schedule.awayMinutes`.
    func awayLimitReached(at now: Date) -> Bool {
        guard let schedule = settings.schedule, let awaySince else { return false }
        return !schedule.isActive(at: now, calendar: calendar) && now >= awaySince.addingTimeInterval(schedule.awayLimit)
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
    func wantedTimers(now: Date) -> [ConnectionTimer: Date] {
        var wanted: [ConnectionTimer: Date] = [:]
        switch state {
        case let .connecting(context) where context.stopRequested == nil:
            wanted[.attemptDeadline] = context.startedAt.addingTimeInterval(RetryPolicy.attemptTimeout)
            if context.stage == .tunnel, let prompted = context.otpPromptedAt, !context.otpSubmitted {
                wanted[.otpDeadline] = prompted.addingTimeInterval(RetryPolicy.otpDeadline)
            }
        case let .connected(context) where !isSleeping && pendingWake == nil:
            // Suspended while asleep and until the wake has been evaluated (§6.3), then re-armed.
            // Outside the scheduled hours it also waits for an away user to return.
            if let planned = context.plannedReconnectAt, automaticReconnectAllowed(at: now) {
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
        if let schedule = settings.schedule, quitRequestedAt == nil {
            if let scheduleBoundary, let next = schedule.nextBoundary(after: scheduleBoundary, calendar: calendar) {
                wanted[.schedule] = next.date
            }
            if let awaySince, !reconnectSuspended {
                wanted[.awayLimit] = awaySince.addingTimeInterval(schedule.awayLimit)
            }
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
