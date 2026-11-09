import Foundation
import Testing
import FortiShared
@testable import FortiCore

/// How a connected session is forcibly ended.
enum ForcedEnd: Sendable, CaseIterable {
    /// openfortivpn killed with a signal.
    case clientKilled
    /// The Helper process ended (killed or crashed).
    case helperLost
}

extension ReducerHarness {
    /// Ends the current session `up` seconds after it came up (the clock is moved there first).
    mutating func forciblyEnd(_ end: ForcedEnd, up: TimeInterval) throws {
        let context = try #require(self.connected, "not connected: \(model.state)")
        now = context.since.addingTimeInterval(up)
        switch end {
        case .clientKilled: exit(ReducerFixture.killed(up: up))
        case .helperLost: send(.helperConnectionLost)
        }
    }
}

private func scheduledHarness(at date: Date) -> ReducerHarness {
    var settings = ReducerFixture.settings
    settings.schedule = weekdayHours
    var h = ReducerHarness(settings: settings)
    h.now = date
    return h
}

@Suite("Forced termination")
struct ForcedTerminationTests {
    // MARK: Immediate reconnect

    /// Starts from one counted failure: a forced end never adds to it, and a stable session still clears it.
    @Test(arguments: ForcedEnd.allCases, [(30.0, 1), (119.0, 1), (180.0, 0), (7200.0, 0)])
    func forcedEndWhileConnectedReconnectsAtOnceUncounted(end: ForcedEnd, session: (up: TimeInterval, failuresAfter: Int)) throws {
        var h = ReducerHarness()
        h.launch(ledger: FailureLedger(
            profileKey: ReducerFixture.profileKey,
            consecutiveAuthFailures: 1,
            lastAuthFailureAt: h.now - 3600,
            lastAuthFailureReason: .otpRejected
        ))
        try h.runToConnected()

        try h.forciblyEnd(end, up: session.up)
        #expect(h.effects.contains(.checkReadiness))
        #expect(h.attempt?.reason == .retry(.forcedTermination))
        #expect(h.attempt?.startedAt == h.now)
        #expect(h.model.timers[.retry] == nil)
        #expect(h.model.ledger.consecutiveAuthFailures == session.failuresAfter)
        try h.runToConnected()
    }

    @Test func normalExitsKeepTheirRules() throws {
        var stable = ReducerHarness()
        try stable.launchConnected()
        stable.advance(180)
        stable.exit(ReducerFixture.facts(.tunnelUp, up: 180, status: .exited(code: 0)))
        #expect(stable.retryWait == RetryWait(until: stable.now + 5, reason: .serverSessionEnded))

        var early = ReducerHarness()
        try early.launchConnected()
        early.advance(30)
        early.exit(ReducerFixture.facts(.tunnelUp, up: 30, status: .exited(code: 0)))
        #expect(early.retryWait == RetryWait(until: early.now + 90, reason: .authFailure(.sessionDroppedEarly)))
        #expect(early.model.ledger.consecutiveAuthFailures == 1)
    }

    @Test func killBeforeTheTunnelCameUpKeepsTheStageRules() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .authenticated)
        h.exit(ReducerFixture.killed(.authenticated))
        #expect(h.retryWait == RetryWait(until: h.now + 90, reason: .authFailure(.tunnelSetupFailed)))
        #expect(h.model.ledger.consecutiveAuthFailures == 1)
    }

    @Test func helperStopEndedByASignalIsNotForced() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.advance(30)
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.helperShutdown), up: 30, status: .signaled(signal: 15)))
        #expect(h.retryWait?.reason == .authFailure(.sessionDroppedEarly))
        #expect(h.model.ledger.consecutiveAuthFailures == 1)
    }

    @Test func losingTheHelperAfterGmailRejectedTheLoginStillFails() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpPrompted)
        h.send(.otpSessionFailed(id, .authentication))
        h.send(.helperConnectionLost)
        #expect(h.model.state == .failed(.gmailAppPassword))
    }

    // MARK: Repeat guard

    @Test(arguments: ForcedEnd.allCases)
    func secondForcedEndBeforeAStableSessionBacksOff(end: ForcedEnd) throws {
        var h = ReducerHarness()
        try h.launchConnected()
        try h.forciblyEnd(.clientKilled, up: 600)
        #expect(h.attempt?.reason == .retry(.forcedTermination))
        try h.runToConnected()

        try h.forciblyEnd(end, up: 30)
        #expect(h.retryWait == RetryWait(until: h.now + 10, reason: .network))
        #expect(h.model.ledger.consecutiveAuthFailures == 0)

        // No session has been stable since the immediate reconnect.
        try h.fire(.retry)
        try h.runToConnected()
        try h.forciblyEnd(.clientKilled, up: 60)
        #expect(h.retryWait == RetryWait(until: h.now + 10, reason: .network))
    }

    @Test func immediateReconnectThatLosesTheHelperWhileConnectingBacksOff() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        try h.forciblyEnd(.helperLost, up: 600)
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .connectedToGateway)
        h.send(.helperConnectionLost)
        #expect(h.retryWait == RetryWait(until: h.now + 10, reason: .network))
        #expect(h.model.ledger.consecutiveAuthFailures == 0)
    }

    @Test(arguments: ForcedEnd.allCases)
    func aStableSessionMakesTheNextForcedEndImmediateAgain(end: ForcedEnd) throws {
        var h = ReducerHarness()
        try h.launchConnected()
        try h.forciblyEnd(.clientKilled, up: 600)
        try h.runToConnected()

        try h.forciblyEnd(end, up: 120)
        #expect(h.attempt?.reason == .retry(.forcedTermination))
    }

    @Test func aStableSessionTheAppReplacedAlsoCounts() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        try h.forciblyEnd(.clientKilled, up: 600)
        try h.runToConnected()
        let id = try #require(h.connected?.attemptID)

        try h.fire(.proactiveReconnect)
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.proactiveReconnect), up: 175 * 60), id: id)
        #expect(h.attempt?.reason == .proactive)
        try h.runToConnected()

        try h.forciblyEnd(.clientKilled, up: 30)
        #expect(h.attempt?.reason == .retry(.forcedTermination))
    }

    @Test(arguments: [false, true])
    func aUserConnectOrReconnectStartsFresh(reconnectNow: Bool) throws {
        var h = ReducerHarness()
        try h.launchConnected()
        try h.forciblyEnd(.clientKilled, up: 600)
        try h.runToConnected()
        try h.forciblyEnd(.clientKilled, up: 30)
        #expect(h.retryWait?.reason == .network)

        if reconnectNow {
            h.send(.userReconnectNow)
        } else {
            h.send(.userDisconnect)
            h.send(.userConnect)
        }
        try h.runToConnected()
        try h.forciblyEnd(.clientKilled, up: 30)
        #expect(h.attempt?.reason == .retry(.forcedTermination))
    }

    // MARK: Gates

    @Test func killDuringSleepWaitsForTheWake() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.advance(600)
        h.send(.willSleep)
        try h.forciblyEnd(.clientKilled, up: 1200)
        #expect(h.networkWait == NetworkWait(reason: .sleep, delayAfterPath: 0))
        #expect(!h.effects.contains(.checkReadiness))
        #expect(h.model.timers.isEmpty)

        h.advance(60)
        h.send(.didWake(sleptFor: 660))
        #expect(h.send(.wakeSettled(observe(alive: false, ppp: false))).contains(.checkReadiness))
        #expect(h.attempt?.reason == .afterSleep)
    }

    @Test func killWhileOfflineWaitsForThePathThenStartsAtOnce() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.advance(300)
        h.send(.pathChanged(ReducerFixture.offline))
        #expect(h.connected != nil)

        try h.forciblyEnd(.clientKilled, up: 600)
        #expect(h.networkWait == NetworkWait(reason: .forcedTermination, delayAfterPath: 0))
        #expect(h.model.timers.isEmpty)

        h.advance(120)
        #expect(h.send(.pathChanged(ReducerFixture.online)).contains(.checkReadiness))
        #expect(h.attempt?.reason == .retry(.forcedTermination))
    }

    @Test func killOutsideTheHoursWhileAwayHoldsTheReconnect() throws {
        var h = scheduledHarness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.advance(600)
        h.send(.presenceChanged(away: true))

        try h.forciblyEnd(.clientKilled, up: 6000)
        #expect(h.retryWait == RetryWait(until: at(18, 20, 10), reason: .userAway))
        #expect(!h.effects.contains(.checkReadiness))

        h.advance(600)
        h.send(.presenceChanged(away: false))
        #expect(h.attempt?.reason == .retry(.userAway))
    }

    @Test func killAfterReconnectsWereSuspendedStaysOff() throws {
        var h = scheduledHarness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.advance(600)
        h.send(.presenceChanged(away: true))
        try h.fire(.awayLimit)
        #expect(h.model.reconnectSuspended)

        try h.forciblyEnd(.helperLost, up: 4 * 3600)
        #expect(h.model.state == .idle)
        #expect(h.model.desired == .disconnected)
    }

    @Test func killAtTheHourlyAttemptCapFails() throws {
        var h = ReducerHarness()
        let attempts = (1...11).map { h.now - TimeInterval($0 * 60) }
        h.launch(ledger: FailureLedger(profileKey: ReducerFixture.profileKey, recentAuthAttempts: attempts.reversed()))
        try h.runToConnected()

        try h.forciblyEnd(.clientKilled, up: 600)
        #expect(h.model.state == .failed(.tooManyReconnects))
    }
}
