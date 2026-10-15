import Foundation
import Testing
import FortiShared
@testable import FortiCore

@Suite("§6.1 connect sequence")
struct ConnectSequenceTests {
    @Test func launchRunsTheWholeSequenceInOrder() throws {
        var h = ReducerHarness()
        let t0 = h.now
        let id1 = ReducerFixture.id(1)

        #expect(h.launch() == [
            .checkReadiness,
            .scheduleTimer(.attemptDeadline, at: t0 + 150, generation: 1),
            .setActivity(true),
        ])
        #expect(h.attempt?.stage == .checkingReadiness)
        #expect(h.model.desired == .connected)

        #expect(h.send(.readinessChanged(.ready)) == [.startOTPSession(id1)])
        #expect(h.attempt?.stage == .preparingOTP)

        #expect(h.send(.otpSessionReady(id1)) == [.startTunnel(id1, ReducerFixture.profile)])
        #expect(h.attempt?.stage == .tunnel)

        h.advance(1)
        #expect(h.milestone(.spawned) == [])
        h.advance(1)
        #expect(h.milestone(.connectedToGateway) == [
            .persistLedger(FailureLedger(profileKey: ReducerFixture.profileKey, recentAuthAttempts: [t0 + 2])),
        ])
        h.advance(1)
        #expect(h.milestone(.otpPrompted) == [
            .beginOTPSearch(id1, promptedAt: t0 + 3),
            .scheduleTimer(.otpDeadline, at: t0 + 53, generation: 2),
        ])
        h.advance(5)
        #expect(h.send(.otpCodeFound(id1, code: "123456")) == [
            .submitOTP(id1, code: "123456"),
            .cancelTimer(.otpDeadline),
        ])
        h.advance(1)
        #expect(h.milestone(.otpSubmitted) == [])
        h.advance(1)
        #expect(h.milestone(.authenticated) == [])
        h.advance(2)
        let planned = t0 + 12 + (180 - 5) * 60
        #expect(h.milestone(.tunnelUp, ip: "10.212.134.5") == [
            .cancelTimer(.attemptDeadline),
            .scheduleTimer(.proactiveReconnect, at: planned, generation: 3),
        ])

        let connected = try #require(h.connected)
        #expect(connected.attemptID == id1)
        #expect(connected.since == t0 + 12)
        #expect(connected.tunnelIP == "10.212.134.5")
        #expect(connected.underlay == ReducerFixture.wifi)
        #expect(connected.plannedReconnectAt == planned)
        #expect(h.model.nextPlannedReconnect == planned)
        #expect(h.model.icon == .connected)
    }

    @Test func proactiveReconnectDisabledArmsNoTimer() throws {
        var h = ReducerHarness(settings: ConnectionSettings(proactiveReconnectEnabled: false))
        try h.launchConnected()
        #expect(h.effects.scheduled(.proactiveReconnect) == nil)
        #expect(h.connected?.plannedReconnectAt == nil)
        #expect(h.model.timers[.proactiveReconnect] == nil)
    }

    @Test func customSessionSettingsDrivePlannedReconnect() throws {
        var h = ReducerHarness(settings: ConnectionSettings(sessionMaxMinutes: 60, proactiveMarginMinutes: 10))
        try h.launchConnected()
        let connected = try #require(h.connected)
        #expect(connected.plannedReconnectAt == connected.since + 50 * 60)
    }

    @Test func connectFromIdleRequiresUserAction() throws {
        var h = ReducerHarness()
        #expect(h.launch(autoConnect: false) == [.checkReadiness])
        #expect(h.model.state == .idle)
        #expect(h.model.desired == .disconnected)
        // An unsolicited "ready" does not start anything while the user wants to stay disconnected.
        #expect(h.send(.readinessChanged(.ready)) == [])
        #expect(h.model.state == .idle)

        let effects = h.send(.userConnect)
        #expect(effects.first == .checkReadiness)
        #expect(effects.contains(.setActivity(true)))
        #expect(h.attempt?.reason == .user)
        try h.runToConnected()
    }

    @Test func otpDeadlineStopsWithoutSubmittingAndCountsOnce() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpPrompted)
        let promptedAt = h.now

        let effects = try h.fire(.otpDeadline)
        #expect(h.now == promptedAt + 50)
        #expect(effects == [
            .stopTunnel(id, .otpTimeout),
            .closeOTPSession(id, checkJunk: true),
            .cancelTimer(.attemptDeadline),
        ])
        #expect(!effects.submitsOTP)

        // A code arriving after the deadline is not submitted.
        #expect(h.send(.otpCodeFound(id, code: "654321")) == [])

        h.advance(1)
        let exit = h.exit(ReducerFixture.facts(.otpPrompted, initiator: .helper(.otpTimeout), watchdog: nil))
        let wait = try #require(h.retryWait)
        #expect(wait.reason == .authFailure(.otpNotReceived(inJunk: false)))
        #expect(wait.until == h.now + 90)
        #expect(exit.scheduled(.retry) == h.now + 90)
        #expect(exit.persistedLedger?.consecutiveAuthFailures == 1)
        #expect(exit.persistedLedger?.lastAuthFailureAt == h.now)
    }

    @Test func junkHintUpgradesTheReasonBeforeOrAfterExit() throws {
        // Reported before the exit.
        var before = ReducerHarness()
        before.launch()
        let id = try before.startTunnel()
        try before.runMilestones(id: id, through: .otpPrompted)
        try before.fire(.otpDeadline)
        #expect(before.send(.otpFoundInJunk(id)) == [])
        before.exit(ReducerFixture.facts(.otpPrompted, initiator: .helper(.otpTimeout)))
        #expect(before.retryWait?.reason == .authFailure(.otpNotReceived(inJunk: true)))
        #expect(before.model.ledger.lastAuthFailureReason == .otpNotReceived(inJunk: true))

        // Reported after the exit.
        var after = ReducerHarness()
        after.launch()
        let id2 = try after.startTunnel()
        try after.runMilestones(id: id2, through: .otpPrompted)
        try after.fire(.otpDeadline)
        after.exit(ReducerFixture.facts(.otpPrompted, initiator: .helper(.otpTimeout)))
        let effects = after.send(.otpFoundInJunk(id2))
        #expect(after.retryWait?.reason == .authFailure(.otpNotReceived(inJunk: true)))
        #expect(effects.persistedLedger?.lastAuthFailureReason == .otpNotReceived(inJunk: true))
        // A hint for an unrelated attempt changes nothing.
        #expect(after.send(.otpFoundInJunk(ReducerFixture.stranger)) == [])
    }

    @Test func attemptDeadlineStopsARunningTunnel() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        h.advance(1)
        h.milestone(.spawned)

        let effects = try h.fire(.attemptDeadline)
        #expect(h.now == ReducerFixture.t0 + 150)
        #expect(effects == [.stopTunnel(id, .attemptTimeout), .closeOTPSession(id, checkJunk: false)])
        #expect(h.attempt?.stopRequested == .attemptTimeout)

        h.exit(ReducerFixture.facts(.spawned, initiator: .helper(.attemptTimeout)))
        #expect(h.retryWait == RetryWait(until: h.now + 10, reason: .network))
        #expect(h.model.ledger.consecutiveAuthFailures == 0)
    }

    @Test func attemptDeadlineWhileGmailLoginHangsIsANetworkError() throws {
        var h = ReducerHarness()
        h.launch()
        h.send(.readinessChanged(.ready))
        let id = try #require(h.attempt?.attemptID)

        let effects = try h.fire(.attemptDeadline)
        #expect(effects.first == .closeOTPSession(id, checkJunk: false))
        #expect(effects.stopReasons.isEmpty)
        #expect(effects.tunnelStarts.isEmpty)
        #expect(h.retryWait?.reason == .network)
        // The late session result is stale now.
        #expect(h.send(.otpSessionReady(id)) == [])
    }

    @Test func gatewayWithoutOTPClosesTheSessionAtTunnelUp() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        h.milestone(.spawned)
        h.milestone(.connectedToGateway)
        h.milestone(.authenticated)
        let effects = h.milestone(.tunnelUp, ip: "10.0.0.1")
        #expect(effects.first == .closeOTPSession(id, checkJunk: false))
        #expect(h.connected != nil)
    }

    @Test(arguments: ["12345", "123456789", "12a456", "", " 123456", "１２３４５６"])
    func implausibleCodesAreNotSubmitted(code: String) throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpPrompted)
        #expect(h.send(.otpCodeFound(id, code: code)) == [])
        #expect(h.attempt?.otpSubmitted == false)
    }

    @Test(arguments: ["123456", "1234567", "12345678"])
    func plausibleCodesAreSubmittedOnce(code: String) throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpPrompted)
        #expect(h.send(.otpCodeFound(id, code: code)).contains(.submitOTP(id, code: code)))
        #expect(h.send(.otpCodeFound(id, code: code)) == [])
    }

    @Test func codeBeforeThePromptIsIgnored() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        h.milestone(.spawned)
        h.milestone(.connectedToGateway)
        #expect(h.send(.otpCodeFound(id, code: "123456")) == [])
    }

    @Test func outOfOrderAndDuplicateMilestonesAreIgnored() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpPrompted)
        #expect(h.milestone(.otpPrompted) == [])
        #expect(h.milestone(.spawned) == [])
        #expect(h.attempt?.milestone == .otpPrompted)
    }

    @Test func authAttemptIsRecordedOncePerAttemptEvenWhenMilestonesSkip() throws {
        var h = ReducerHarness()
        h.launch()
        _ = try h.startTunnel()
        h.milestone(.spawned)
        // `connectedToGateway` was never reported, `otpPrompted` implies it.
        let effects = h.milestone(.otpPrompted)
        #expect(effects.persistedLedger?.recentAuthAttempts == [h.now])
        h.milestone(.otpSubmitted)
        #expect(h.model.ledger.recentAuthAttempts.count == 1)
    }

    @Test func attemptIDsAreDeterministicAndFresh() throws {
        var h = ReducerHarness()
        h.launch()
        #expect(h.attempt?.attemptID == ReducerFixture.id(1))
        _ = try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        try h.fire(.retry)
        #expect(h.attempt?.attemptID == ReducerFixture.id(2))
        #expect(ReducerFixture.id(1) != ReducerFixture.id(2))
    }
}
