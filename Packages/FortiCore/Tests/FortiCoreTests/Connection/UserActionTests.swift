import Foundation
import Testing
import FortiShared
@testable import FortiCore

@Suite("Disconnect, quit and reconnect-now")
struct UserActionTests {
    @Test func disconnectWhileConnectedFinishesOnExit() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)

        #expect(h.send(.userDisconnect) == [.stopTunnel(id, .userDisconnect), .cancelTimer(.proactiveReconnect)])
        #expect(h.model.desired == .disconnected)
        #expect(h.disconnecting == DisconnectContext(attemptID: id, reason: .userDisconnect, after: .idle, connectedSince: ReducerFixture.t0 + 12))
        #expect(h.model.icon == .inProgress)
        #expect(h.model.menuActions == ConnectionMenuActions(showsConnect: true, showsDisconnect: false, reconnectNowEnabled: false))

        h.advance(2)
        #expect(h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.userDisconnect), up: 2)) == [.setActivity(false)])
        #expect(h.model.state == .idle)
        #expect(h.model.icon == .disconnected)
    }

    @Test func disconnectWhileTheTunnelIsStarting() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        h.milestone(.spawned)
        #expect(h.send(.userDisconnect) == [
            .stopTunnel(id, .userDisconnect),
            .closeOTPSession(id, checkJunk: false),
            .cancelTimer(.attemptDeadline),
        ])
        h.exit(ReducerFixture.facts(.spawned, initiator: .helper(.userDisconnect)))
        #expect(h.model.state == .idle)
        #expect(h.model.ledger.consecutiveAuthFailures == 0)
    }

    @Test func disconnectBeforeTheTunnelStartedIsImmediate() throws {
        var h = ReducerHarness()
        h.launch()
        #expect(h.send(.userDisconnect) == [.cancelTimer(.attemptDeadline), .setActivity(false)])
        #expect(h.model.state == .idle)
        // The late readiness answer does not start anything.
        #expect(h.send(.readinessChanged(.ready)) == [])
        #expect(h.model.state == .idle)
    }

    @Test func disconnectWhileWaitingCancelsTheWait() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        #expect(h.send(.userDisconnect) == [.cancelTimer(.retry), .setActivity(false)])
        #expect(h.model.state == .idle)

        var offline = ReducerHarness(path: ReducerFixture.offline)
        offline.launch()
        #expect(offline.networkWait != nil)
        #expect(offline.send(.userDisconnect) == [.setActivity(false)])
        #expect(offline.model.state == .idle)
    }

    @Test func connectDuringAUserDisconnectReconnectsAfterTheExit() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.send(.userDisconnect)
        // The manual action clears the rolling history recorded by this session.
        #expect(h.send(.userConnect) == [.persistLedger(FailureLedger(profileKey: ReducerFixture.profileKey))])
        #expect(h.disconnecting?.after == .reconnect(.user))
        #expect(h.model.desired == .connected)
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.userDisconnect), up: 30))
        #expect(h.attempt?.reason == .user)
    }

    @Test func disconnectDuringAPlannedReconnectEndsIdle() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        try h.fire(.proactiveReconnect)
        h.send(.userDisconnect)
        #expect(h.disconnecting?.after == .idle)
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.proactiveReconnect), up: 10_500))
        #expect(h.model.state == .idle)
    }

    // MARK: Quit

    @Test func quitWhileIdleTerminatesAtOnce() {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        #expect(h.send(.quitRequested) == [.terminateApp])
        #expect(h.model.isTerminated)
        #expect(h.send(.userConnect) == [])
    }

    @Test func quitWhileConnectedStopsThenTerminatesOnExit() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)
        let quit = h.send(.quitRequested)
        #expect(quit == [
            .stopTunnel(id, .appQuit),
            .cancelTimer(.proactiveReconnect),
            .scheduleTimer(.quitDeadline, at: h.now + 12, generation: 4),
        ])
        #expect(h.model.desired == .disconnected)

        h.advance(1)
        #expect(h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.appQuit), up: 20)) == [
            .cancelTimer(.quitDeadline),
            .setActivity(false),
            .terminateApp,
        ])
        #expect(h.exit(ReducerFixture.facts(.tunnelUp)) == [], "nothing after termination")
    }

    @Test func quitDeadlineTerminatesWithoutTheExit() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.send(.quitRequested)
        #expect(try h.fire(.quitDeadline) == [.setActivity(false), .terminateApp])
        #expect(h.now == ReducerFixture.t0 + 12 + 12)
    }

    @Test func quitWhileTheGmailSessionStartsClosesItAndTerminates() throws {
        var h = ReducerHarness()
        h.launch()
        h.send(.readinessChanged(.ready))
        let id = ReducerFixture.id(1)
        #expect(h.send(.quitRequested) == [
            .closeOTPSession(id, checkJunk: false),
            .cancelTimer(.attemptDeadline),
            .setActivity(false),
            .terminateApp,
        ])
    }

    @Test func quitWhileWaitingTerminatesAtOnce() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        #expect(h.send(.quitRequested) == [.cancelTimer(.retry), .setActivity(false), .terminateApp])
    }

    @Test func quitDuringAStopInFlightDoesNotStopTwice() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpPrompted)
        try h.fire(.otpDeadline)

        let quit = h.send(.quitRequested)
        #expect(quit.stopReasons.isEmpty)
        #expect(quit.scheduled(.quitDeadline) == h.now + 12)
        #expect(h.disconnecting?.after == .terminate)
        let exit = h.exit(ReducerFixture.facts(.otpPrompted, initiator: .helper(.otpTimeout)))
        #expect(exit.last == .terminateApp)
        #expect(exit.persistedLedger == nil)
    }

    @Test func quitDuringAUserDisconnectWaitsForTheSameExit() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.send(.userDisconnect)
        let quit = h.send(.quitRequested)
        #expect(quit.stopReasons.isEmpty)
        #expect(h.disconnecting?.after == .terminate)
        // A disconnect click during quit cannot cancel termination.
        h.send(.userDisconnect)
        #expect(h.disconnecting?.after == .terminate)
        #expect(h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.userDisconnect), up: 30)).last == .terminateApp)
    }

    // MARK: Reconnect now

    @Test func reconnectNowWhileConnectedIsBreakBeforeMake() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)
        #expect(h.model.menuActions.reconnectNowEnabled)
        #expect(h.send(.userReconnectNow) == [
            .stopTunnel(id, .reconnect),
            .cancelTimer(.proactiveReconnect),
            .persistLedger(FailureLedger(profileKey: ReducerFixture.profileKey)),
        ])
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.reconnect), up: 50))
        #expect(h.attempt?.reason == .manualReconnect)
    }

    @Test func reconnectNowSoonAfterAnAuthFailureWaitsUntilNinetySeconds() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .connectedToGateway)
        h.exit(ReducerFixture.facts(.connectedToGateway, authFail: true))
        let failedAt = h.now

        h.advance(20)
        let effects = h.send(.userReconnectNow)
        #expect(h.retryWait == RetryWait(until: failedAt + 90, reason: .authCooldown))
        #expect(effects.persistedLedger?.consecutiveAuthFailures == 0)
        #expect(!effects.contains(.checkReadiness))
        try h.fire(.retry)
        #expect(h.now == failedAt + 90)
        #expect(h.attempt != nil)
    }

    @Test func reconnectNowCooldownTimerDeliveredSlightlyEarlyStillStartsTheAttempt() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .connectedToGateway)
        h.exit(ReducerFixture.facts(.connectedToGateway, authFail: true))
        let failedAt = h.now
        h.advance(20)
        h.send(.userReconnectNow)
        #expect(h.retryWait == RetryWait(until: failedAt + 90, reason: .authCooldown))

        let effects = try h.fireEarly(.retry, by: 0.001)
        #expect(effects.contains(.checkReadiness))
        #expect(h.attempt?.reason == .retry(.authCooldown))
        #expect(h.attempt?.startedAt == failedAt + 90)
    }

    @Test func reconnectNowAfterTheGapStartsImmediately() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        #expect(h.retryWait?.reason == .network)
        #expect(h.send(.userReconnectNow).contains(.checkReadiness))
        #expect(h.attempt?.reason == .manualReconnect)
    }

    @Test func reconnectNowWhileConnectingIsIgnored() throws {
        var h = ReducerHarness()
        h.launch()
        #expect(h.send(.userReconnectNow) == [])
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpPrompted)
        let before = h.model
        #expect(h.send(.userReconnectNow) == [])
        #expect(h.model == before)
        #expect(!h.model.menuActions.reconnectNowEnabled)
    }

    @Test func reconnectNowIsIgnoredWhenNothingIsWanted() throws {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        #expect(h.send(.userReconnectNow) == [])
        h.send(.readinessChanged(.needsSetup([.vpnPassword])))
        #expect(h.send(.userReconnectNow) == [])

        var stopping = ReducerHarness()
        try stopping.launchConnected()
        stopping.send(.userDisconnect)
        #expect(stopping.send(.userReconnectNow) == [])
    }

    // MARK: Proactive reconnect

    @Test func proactiveTimerReconnectsRightAfterTheExit() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)
        let planned = try #require(h.connected?.plannedReconnectAt)

        #expect(try h.fire(.proactiveReconnect) == [.stopTunnel(id, .proactiveReconnect)])
        #expect(h.now == planned)
        #expect(h.disconnecting?.after == .reconnect(.proactive))

        h.advance(1)
        let effects = h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.proactiveReconnect), up: 10_500))
        #expect(effects.contains(.checkReadiness))
        #expect(h.attempt?.reason == .proactive)
    }

    @Test func proactiveTimerFiringFirstThenServerCutGivesOneReconnect() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)
        try h.fire(.proactiveReconnect)
        // The server cut the session before our SIGTERM landed.
        let effects = h.exit(ReducerFixture.facts(.tunnelUp, up: 10_500))
        #expect(effects.filter { $0 == .checkReadiness }.count == 1)
        #expect(h.attempt?.reason == .proactive)
        #expect(h.exit(ReducerFixture.facts(.tunnelUp, up: 10_500), id: id) == [], "duplicate exit is stale")
    }

    @Test func serverCutFirstMakesTheProactiveTimerStale() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let generation = try #require(h.generation(of: .proactiveReconnect))
        h.advance(10_499)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 10_499))
        #expect(h.retryWait?.reason == .serverSessionEnded)

        h.advance(1)
        #expect(h.send(.timerFired(.proactiveReconnect, generation: generation)) == [])
        #expect(h.retryWait?.reason == .serverSessionEnded)
    }

    // MARK: Settings

    @Test func settingsChangeRecomputesThePlannedReconnect() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let since = try #require(h.connected?.since)

        let shorter = h.send(.settingsChanged(ConnectionSettings(sessionMaxMinutes: 60)))
        #expect(shorter.scheduled(.proactiveReconnect) == since + 55 * 60)
        #expect(h.model.nextPlannedReconnect == since + 55 * 60)

        h.advance(400)
        let past = h.send(.settingsChanged(ConnectionSettings(sessionMaxMinutes: 10, proactiveMarginMinutes: 5)))
        #expect(past.scheduled(.proactiveReconnect) == h.now + 60, "clamped to at least a minute from now")

        #expect(h.send(.settingsChanged(ConnectionSettings(sessionMaxMinutes: 10, proactiveReconnectEnabled: false))) == [
            .cancelTimer(.proactiveReconnect),
        ])
        #expect(h.model.nextPlannedReconnect == nil)

        #expect(h.send(.settingsChanged(ConnectionSettings(sessionMaxMinutes: 10, proactiveReconnectEnabled: false, reconnectAfterSleepEnabled: true))) == [])
    }

    @Test func settingsChangeWhileNotConnectedAppliesToTheNextSession() throws {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        #expect(h.send(.settingsChanged(ConnectionSettings(sessionMaxMinutes: 30))) == [])
        h.send(.userConnect)
        try h.runToConnected()
        let connected = try #require(h.connected)
        #expect(connected.plannedReconnectAt == connected.since + 25 * 60)
    }
}

@Suite("Stale events")
struct StaleEventTests {
    @Test func helperEventsFromOtherAttemptsAreIgnored() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        let before = h.model
        #expect(h.milestone(.tunnelUp, ip: "10.9.9.9", id: ReducerFixture.stranger) == [])
        #expect(h.exit(ReducerFixture.facts(.spawned), id: ReducerFixture.stranger) == [])
        #expect(h.model == before)

        try h.runToConnectedFromTunnel()
        let connected = h.model
        #expect(h.exit(ReducerFixture.facts(.tunnelUp, up: 5), id: ReducerFixture.stranger) == [])
        #expect(h.milestone(.otpPrompted, id: ReducerFixture.stranger) == [])
        #expect(h.model == connected)
    }

    @Test func exitOfAnOldAttemptWhileWaitingRetryIsIgnored() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        let waiting = h.model
        #expect(h.exit(ReducerFixture.facts(.spawned), id: id) == [])
        #expect(h.exit(ReducerFixture.facts(.tunnelUp, up: 9_999), id: id) == [])
        #expect(h.send(.helperConnectRejected(id, HelperError(.binaryUnavailable, "late"))) == [])
        #expect(h.model == waiting)
    }

    @Test func helperEventsBeforeTheTunnelWasRequestedAreIgnored() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try #require(h.attempt?.attemptID)
        #expect(h.milestone(.spawned, id: id) == [])
        h.send(.readinessChanged(.ready))
        #expect(h.exit(ReducerFixture.facts(.spawned), id: id) == [])
        #expect(h.attempt?.stage == .preparingOTP)
    }

    @Test func eventsForTheAttemptBeforeACertificateRetryAreIgnored() throws {
        var h = ReducerHarness()
        h.launch()
        let id1 = try h.startTunnel()
        try h.runMilestones(id: id1, through: .spawned)
        let oldDeadline = try #require(h.generation(of: .attemptDeadline))
        h.exit(ReducerFixture.facts(.spawned, certs: [ReducerFixture.digestA], certAt: .spawned))
        h.send(.readinessChanged(.ready))
        let retry = h.model

        #expect(h.send(.timerFired(.attemptDeadline, generation: oldDeadline)) == [])
        #expect(h.send(.otpSessionReady(id1)) == [])
        #expect(h.send(.otpSessionFailed(id1, .authentication)) == [])
        #expect(h.send(.otpCodeFound(id1, code: "123456")) == [])
        #expect(h.send(.helperConnectRejected(id1, HelperError(.busy, "late"))) == [])
        #expect(h.milestone(.otpPrompted, id: id1) == [])
        #expect(h.model == retry)
    }

    @Test func timersFireOnlyOncePerGeneration() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        let generation = try #require(h.generation(of: .retry))
        try h.fire(.retry)
        let after = h.model
        #expect(h.send(.timerFired(.retry, generation: generation)) == [])
        #expect(h.send(.timerFired(.attemptDeadline, generation: 999)) == [])
        #expect(h.model == after)
    }
}

extension ReducerHarness {
    /// From a started tunnel to `connected`.
    mutating func runToConnectedFromTunnel(sourceLocation: SourceLocation = #_sourceLocation) throws {
        let id = try #require(attempt?.attemptID, sourceLocation: sourceLocation)
        try runMilestones(id: id, through: .tunnelUp, sourceLocation: sourceLocation)
        _ = try #require(connected, sourceLocation: sourceLocation)
    }
}
