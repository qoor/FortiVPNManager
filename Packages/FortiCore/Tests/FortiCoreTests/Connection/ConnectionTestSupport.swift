import Foundation
import Testing
import FortiShared
@testable import FortiCore

enum ReducerFixture {
    static let t0 = Date(timeIntervalSince1970: 1_900_000_000)
    static let seed = UUID(uuid: (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    /// An attempt ID the reducer never issued.
    static let stranger = UUID(uuid: (0xde, 0xad, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff))

    static let pinOld = String(repeating: "0", count: 64)
    static let digestA = String(repeating: "a", count: 64)
    static let digestB = String(repeating: "b", count: 64)
    static let digestC = String(repeating: "c", count: 64)
    static let digestD = String(repeating: "d", count: 64)

    static let wifi = Underlay(interfaceName: "en0", sourceAddress: "192.168.0.10")
    static let otherWifi = Underlay(interfaceName: "en0", sourceAddress: "10.1.2.3")
    static let ethernet = Underlay(interfaceName: "en7", sourceAddress: "192.168.1.20")
    static let online = PathSnapshot(isSatisfied: true, underlay: wifi)
    static let offline = PathSnapshot(isSatisfied: false, underlay: nil)

    static let profile = VPNProfile(host: "vpn.example.com", port: 8443, username: "alice", trustedCerts: [pinOld])
    static let profileKey = "alice@vpn.example.com"

    /// The `n`-th attempt ID (1-based) issued by a model seeded with `seed`.
    static func id(_ n: Int) -> AttemptID {
        var generator = AttemptIDGenerator(seed: seed)
        var result = generator.next()
        for _ in 1..<n { result = generator.next() }
        return result
    }

    static func facts(
        _ furthest: Milestone?,
        initiator: StopInitiator = .none,
        up: TimeInterval? = nil,
        certs: [String] = [],
        certAt: Milestone? = nil,
        watchdog: WatchdogPhase? = nil,
        authFail: Bool = false
    ) -> ExitFacts {
        ExitFacts(
            furthestMilestone: furthest,
            stopInitiator: initiator,
            watchdogFired: watchdog,
            certRejectedDigests: certs,
            certRejectedAt: certAt,
            authFailLineSeen: authFail,
            upDuration: up,
            exitStatus: furthest == nil ? .spawnFailed(errno: 2) : .exited(code: 1)
        )
    }
}

/// Drives the reducer with a controllable clock, as `ConnectionController` would.
struct ReducerHarness {
    var model: ConnectionModel
    var now: Date = ReducerFixture.t0
    /// Effects of the last event.
    private(set) var effects: [ConnectionEffect] = []

    init(
        profile: VPNProfile = ReducerFixture.profile,
        settings: ConnectionSettings = ConnectionSettings(),
        path: PathSnapshot? = ReducerFixture.online
    ) {
        model = ConnectionModel(profile: profile, settings: settings, attemptIDSeed: ReducerFixture.seed)
        if let path { send(.pathChanged(path)) }
    }

    @discardableResult
    mutating func send(_ event: ConnectionEvent) -> [ConnectionEffect] {
        effects = ConnectionReducer.reduce(state: &model, event: event, now: now)
        return effects
    }

    mutating func advance(_ seconds: TimeInterval) {
        now = now.addingTimeInterval(seconds)
    }

    /// Moves the clock to the timer's due date (if later) and fires it with its current generation.
    @discardableResult
    mutating func fire(_ timer: ConnectionTimer, sourceLocation: SourceLocation = #_sourceLocation) throws -> [ConnectionEffect] {
        let slot = try #require(model.timers[timer], "timer \(timer) is not armed", sourceLocation: sourceLocation)
        if slot.date > now { now = slot.date }
        return send(.timerFired(timer, generation: slot.generation))
    }

    /// Fires the timer's current generation `seconds` before its due date and leaves the clock there,
    /// like an app timer slept on the monotonic clock while the wall clock is slewed.
    @discardableResult
    mutating func fireEarly(
        _ timer: ConnectionTimer,
        by seconds: TimeInterval,
        sourceLocation: SourceLocation = #_sourceLocation
    ) throws -> [ConnectionEffect] {
        let slot = try #require(model.timers[timer], "timer \(timer) is not armed", sourceLocation: sourceLocation)
        now = slot.date.addingTimeInterval(-seconds)
        return send(.timerFired(timer, generation: slot.generation))
    }

    func generation(of timer: ConnectionTimer) -> UInt64? {
        model.timers[timer]?.generation
    }

    var attempt: AttemptContext? {
        if case let .connecting(context) = model.state { return context }
        return nil
    }

    var connected: ConnectedContext? {
        if case let .connected(context) = model.state { return context }
        return nil
    }

    var disconnecting: DisconnectContext? {
        if case let .disconnecting(context) = model.state { return context }
        return nil
    }

    var retryWait: RetryWait? {
        if case let .waitingRetry(wait) = model.state { return wait }
        return nil
    }

    var networkWait: NetworkWait? {
        if case let .waitingNetwork(wait) = model.state { return wait }
        return nil
    }

    var currentID: AttemptID? {
        attempt?.attemptID ?? model.activeAttemptID
    }

    @discardableResult
    mutating func helper(_ kind: HelperEvent.Kind, id: AttemptID? = nil) -> [ConnectionEffect] {
        send(.helperEvent(HelperEvent(attemptID: id ?? currentID ?? ReducerFixture.stranger, date: now, kind: kind)))
    }

    @discardableResult
    mutating func milestone(_ milestone: Milestone, ip: String? = nil, id: AttemptID? = nil) -> [ConnectionEffect] {
        helper(.milestone(milestone, tunnelIP: ip), id: id)
    }

    @discardableResult
    mutating func exit(_ facts: ExitFacts, id: AttemptID? = nil) -> [ConnectionEffect] {
        helper(.exited(facts), id: id)
    }

    @discardableResult
    mutating func launch(autoConnect: Bool = true, ledger: FailureLedger? = nil) -> [ConnectionEffect] {
        send(.appLaunched(autoConnect: autoConnect, restoredLedger: ledger))
    }

    /// Answers the readiness check (if pending) and the Gmail session start; returns the attempt.
    @discardableResult
    mutating func startTunnel(sourceLocation: SourceLocation = #_sourceLocation) throws -> AttemptID {
        let context = try #require(attempt, "not connecting: \(model.state)", sourceLocation: sourceLocation)
        if context.stage == .checkingReadiness {
            send(.readinessChanged(.ready))
        }
        let id = context.attemptID
        send(.otpSessionReady(id))
        #expect(effects.tunnelStarts.count == 1, sourceLocation: sourceLocation)
        return id
    }

    /// Runs the helper milestones and the OTP exchange up to `tunnelUp` (12 s of clock time).
    mutating func runToConnected(sourceLocation: SourceLocation = #_sourceLocation) throws {
        let id = try startTunnel(sourceLocation: sourceLocation)
        try runMilestones(id: id, through: .tunnelUp, sourceLocation: sourceLocation)
        _ = try #require(connected, "not connected: \(model.state)", sourceLocation: sourceLocation)
    }

    /// Reports milestones (and submits an OTP when prompted) up to and including `last`.
    mutating func runMilestones(id: AttemptID, through last: Milestone, sourceLocation: SourceLocation = #_sourceLocation) throws {
        for milestone in Milestone.allCases where milestone <= last {
            switch milestone {
            case .spawned, .connectedToGateway, .otpPrompted:
                advance(1)
                self.milestone(milestone, id: id)
            case .otpSubmitted:
                advance(5)
                send(.otpCodeFound(id, code: "123456"))
                #expect(effects.contains(.submitOTP(id, code: "123456")), sourceLocation: sourceLocation)
                advance(1)
                self.milestone(.otpSubmitted, id: id)
            case .authenticated:
                advance(1)
                self.milestone(.authenticated, id: id)
            case .tunnelUp:
                advance(2)
                self.milestone(.tunnelUp, ip: "10.212.134.5", id: id)
            }
        }
    }

    /// Launch with auto-connect and bring the tunnel up.
    mutating func launchConnected(sourceLocation: SourceLocation = #_sourceLocation) throws {
        launch()
        try runToConnected(sourceLocation: sourceLocation)
    }
}

extension Array where Element == ConnectionEffect {
    func scheduled(_ timer: ConnectionTimer) -> Date? {
        for case let .scheduleTimer(scheduledTimer, date, _) in self where scheduledTimer == timer {
            return date
        }
        return nil
    }

    func cancels(_ timer: ConnectionTimer) -> Bool {
        contains(.cancelTimer(timer))
    }

    var tunnelStarts: [VPNProfile] {
        compactMap { if case let .startTunnel(_, profile) = $0 { profile } else { nil } }
    }

    var stopReasons: [StopReason] {
        compactMap { if case let .stopTunnel(_, reason) = $0 { reason } else { nil } }
    }

    var submitsOTP: Bool {
        contains { if case .submitOTP = $0 { true } else { false } }
    }

    var startsOTPSession: Bool {
        contains { if case .startOTPSession = $0 { true } else { false } }
    }

    var persistedLedger: FailureLedger? {
        compactMap { if case let .persistLedger(ledger) = $0 { ledger } else { nil } }.last
    }

    var persistedCerts: [String]? {
        compactMap { if case let .persistTrustedCerts(certs) = $0 { certs } else { nil } }.last
    }
}
