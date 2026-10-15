import Foundation
import Testing
import FortiShared
@testable import FortiCore

@Suite("§6.2 retry policy in the reducer")
struct FailureHandlingTests {
    /// Fails one attempt at the gateway (wrong password) and returns its ID.
    @discardableResult
    private func failAtGateway(_ h: inout ReducerHarness) throws -> AttemptID {
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .connectedToGateway)
        h.exit(ReducerFixture.facts(.connectedToGateway, authFail: true))
        return id
    }

    @Test func serverEndReconnectsAfterFiveSecondsAndResetsTheCounter() throws {
        var h = ReducerHarness()
        h.launch()
        try failAtGateway(&h)
        #expect(h.model.ledger.consecutiveAuthFailures == 1)
        try h.fire(.retry)
        try h.runToConnected()

        h.advance(3 * 3600)
        let effects = h.exit(ReducerFixture.facts(.tunnelUp, up: 3 * 3600))
        #expect(h.retryWait == RetryWait(until: h.now + 5, reason: .serverSessionEnded))
        #expect(effects.cancels(.proactiveReconnect))
        #expect(effects.scheduled(.retry) == h.now + 5)
        #expect(effects.persistedLedger?.consecutiveAuthFailures == 0)

        try h.fire(.retry)
        #expect(h.attempt?.reason == .retry(.serverSessionEnded))
        try h.runToConnected()
    }

    @Test func oneAuthFailureRetriesAfterNinetySecondsAndTheSecondFails() throws {
        var h = ReducerHarness()
        h.launch()
        try failAtGateway(&h)
        let firstFailure = h.now
        #expect(h.retryWait == RetryWait(until: firstFailure + 90, reason: .authFailure(.authenticationRejected)))
        #expect(h.effects.persistedLedger?.consecutiveAuthFailures == 1)
        #expect(h.model.icon == .inProgress)
        #expect(h.model.menuActions.reconnectNowEnabled)

        try h.fire(.retry)
        #expect(h.now == firstFailure + 90)
        #expect(h.attempt?.reason == .retry(.authFailure(.authenticationRejected)))
        try failAtGateway(&h)

        #expect(h.model.state == .failed(.authenticationRejected))
        #expect(h.effects.persistedLedger?.consecutiveAuthFailures == 2)
        #expect(h.effects.contains(.setActivity(false)))
        #expect(h.model.timers.isEmpty)
        #expect(h.model.icon == .error)
        #expect(h.model.attention == .failed(.authenticationRejected))
        #expect(h.model.menuActions.showsConnect)
    }

    /// The retry date equals the ledger's cooldown end; a fire delivered a hair early must not be
    /// sent back to the same (already fired) wait and stall forever.
    @Test(arguments: [0.001, 0.005, 2.0])
    func authRetryDeliveredSlightlyEarlyStillStartsTheAttempt(early: TimeInterval) throws {
        var h = ReducerHarness()
        h.launch()
        try failAtGateway(&h)
        let wait = try #require(h.retryWait)
        #expect(h.model.ledger.cooldownEnd == wait.until)

        let effects = try h.fireEarly(.retry, by: early)
        #expect(effects.contains(.checkReadiness))
        #expect(h.attempt?.reason == .retry(.authFailure(.authenticationRejected)))
        #expect(h.attempt?.startedAt == wait.until, "the attempt is evaluated at the timer's date")
        #expect(h.model.timers[.retry] == nil)
        try h.runToConnected()
    }

    @Test func networkRetryDeliveredSlightlyEarlyStartsTheAttempt() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        #expect(h.retryWait?.reason == .network)
        #expect(try h.fireEarly(.retry, by: 0.005).contains(.checkReadiness))
        #expect(h.attempt?.reason == .retry(.network))
    }

    @Test func quickDropLoopCountsAndFails() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.advance(30)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 30))
        #expect(h.retryWait?.reason == .authFailure(.sessionDroppedEarly))
        #expect(h.model.ledger.consecutiveAuthFailures == 1)

        try h.fire(.retry)
        try h.runToConnected()
        h.advance(45)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 45))
        #expect(h.model.state == .failed(.sessionDroppedEarly))
    }

    @Test func networkFailuresBackOffWithoutCounting() throws {
        var h = ReducerHarness()
        h.launch()
        for expected in [10.0, 20, 40, 80, 160, 300, 300] {
            try h.startTunnel()
            h.milestone(.spawned)
            let effects = h.exit(ReducerFixture.facts(.spawned, initiator: .helper(.watchdog), watchdog: .spawnToGateway))
            #expect(h.retryWait == RetryWait(until: h.now + expected, reason: .network))
            #expect(effects.persistedLedger == nil)
            try h.fire(.retry)
        }
        #expect(h.model.ledger == FailureLedger(profileKey: ReducerFixture.profileKey))

        // A tunnel coming up resets the backoff.
        try h.runToConnected()
        h.advance(600)
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.helperShutdown), up: 600))
        #expect(h.retryWait == RetryWait(until: h.now + 10, reason: .network))
    }

    @Test func networkFailureWaitsForThePathThenBacksOff() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        h.milestone(.spawned)
        #expect(h.send(.pathChanged(ReducerFixture.offline)) == [])
        #expect(h.attempt?.pathChanged == true)

        h.exit(ReducerFixture.facts(.spawned))
        #expect(h.networkWait == NetworkWait(reason: .network, delayAfterPath: 10))
        #expect(h.model.timers.isEmpty)
        #expect(h.model.icon == .inProgress)

        h.advance(300)
        #expect(h.send(.pathChanged(ReducerFixture.online)).scheduled(.retry) == h.now + 10)
        #expect(h.retryWait == RetryWait(until: h.now + 10, reason: .network))
        try h.fire(.retry)
        #expect(h.attempt != nil)
        #expect(h.attempt?.attemptID != id)
    }

    @Test func retryDueWhileOfflineWaitsForThePathThenStartsImmediately() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        h.send(.pathChanged(ReducerFixture.offline))
        #expect(h.retryWait != nil, "waitingRetry ignores path changes until it is due")

        try h.fire(.retry)
        #expect(h.networkWait == NetworkWait(reason: .network, delayAfterPath: 0))
        let effects = h.send(.pathChanged(ReducerFixture.online))
        #expect(effects.contains(.checkReadiness))
        #expect(h.attempt?.reason == .retry(.network))
    }

    @Test(arguments: [(10.0, true), (30.0, true), (31.0, false)])
    func gatewayFailureRightAfterWakeIsANetworkError(secondsAfterWake: TimeInterval, isNetwork: Bool) throws {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        h.send(.willSleep)
        h.advance(600)
        h.send(.didWake(sleptFor: 600))
        h.send(.wakeSettled(WakeObservation(processAlive: false, pppUp: false, underlay: ReducerFixture.wifi, pathSatisfied: true)))
        #expect(h.model.state == .idle)

        h.advance(secondsAfterWake)
        h.send(.userConnect)
        try failAtGateway(&h)
        if isNetwork {
            #expect(h.retryWait?.reason == .network)
            #expect(h.model.ledger.consecutiveAuthFailures == 0)
        } else {
            #expect(h.retryWait?.reason == .authFailure(.authenticationRejected))
            #expect(h.model.ledger.consecutiveAuthFailures == 1)
        }
    }

    @Test func gatewayFailureAfterAPathChangeIsANetworkError() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .connectedToGateway)
        #expect(h.send(.pathChanged(PathSnapshot(isSatisfied: true, underlay: ReducerFixture.otherWifi))) == [])
        h.exit(ReducerFixture.facts(.connectedToGateway, authFail: true))
        #expect(h.retryWait?.reason == .network)
        #expect(h.model.ledger.consecutiveAuthFailures == 0)
    }

    @Test func gmailAuthFailureFailsWithoutStartingOpenfortivpn() throws {
        var h = ReducerHarness()
        h.launch()
        h.send(.readinessChanged(.ready))
        let id = try #require(h.attempt?.attemptID)
        let effects = h.send(.otpSessionFailed(id, .authentication))
        #expect(h.model.state == .failed(.gmailAppPassword))
        #expect(effects.tunnelStarts.isEmpty)
        #expect(effects == [.cancelTimer(.attemptDeadline), .setActivity(false)])
        #expect(h.send(.otpSessionReady(id)) == [])
        #expect(h.model.ledger.consecutiveAuthFailures == 0)
    }

    @Test func gmailNetworkFailureBacksOff() throws {
        var h = ReducerHarness()
        h.launch()
        h.send(.readinessChanged(.ready))
        let id = try #require(h.attempt?.attemptID)
        h.send(.otpSessionFailed(id, .network))
        #expect(h.retryWait == RetryWait(until: h.now + 10, reason: .network))
    }

    @Test(arguments: [OTPSessionFailure.authentication, .network])
    func gmailFailureWhileTheTunnelRunsStopsIt(failure: OTPSessionFailure) throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpPrompted)
        #expect(h.send(.otpSessionFailed(id, failure)) == [
            .stopTunnel(id, .otpUnavailable),
            .cancelTimer(.attemptDeadline),
            .cancelTimer(.otpDeadline),
        ])
        h.exit(ReducerFixture.facts(.otpPrompted, initiator: .helper(.otpUnavailable)))
        switch failure {
        case .authentication: #expect(h.model.state == .failed(.gmailAppPassword))
        case .network: #expect(h.retryWait?.reason == .network)
        }
        #expect(h.model.ledger.consecutiveAuthFailures == 0)
        #expect(h.model.ledger.recentAuthAttempts.count == 1)
    }

    @Test func gmailWrapUpFailureAfterSubmissionIsIgnored() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpSubmitted)
        #expect(h.send(.otpSessionFailed(id, .network)) == [])
        h.milestone(.authenticated)
        h.milestone(.tunnelUp, ip: "10.0.0.2")
        #expect(h.connected != nil)
    }

    @Test func rollingCapStopsAReconnectLoop() throws {
        var h = ReducerHarness()
        h.launch()
        for attempt in 1...12 {
            try h.runToConnected()
            h.advance(130)
            h.exit(ReducerFixture.facts(.tunnelUp, up: 130))
            #expect(h.retryWait?.reason == .serverSessionEnded)
            try h.fire(.retry)
            if attempt < 12 {
                #expect(h.attempt != nil, "attempt \(attempt + 1) should start")
            }
        }
        #expect(h.model.state == .failed(.tooManyReconnects))
        #expect(h.model.ledger.authAttempts(asOf: h.now) == 12)

        // Manual action clears the history and is allowed through.
        let effects = h.send(.userConnect)
        #expect(effects.contains(.checkReadiness))
        #expect(effects.persistedLedger?.recentAuthAttempts == [])
    }

    @Test func rollingCapWindowSlides() throws {
        var h = ReducerHarness()
        h.launch()
        for _ in 1...20 {
            try h.runToConnected()
            h.advance(400)
            h.exit(ReducerFixture.facts(.tunnelUp, up: 400))
            try h.fire(.retry)
            #expect(h.attempt != nil)
        }
        #expect(h.model.ledger.authAttempts(asOf: h.now) < 12)
    }

    @Test func failedOnlyClearsOnUserAction() throws {
        var h = ReducerHarness()
        h.launch()
        try failAtGateway(&h)
        try h.fire(.retry)
        try failAtGateway(&h)
        let failed = ConnectionState.failed(.authenticationRejected)
        #expect(h.model.state == failed)

        h.advance(3600)
        let nonUserEvents: [ConnectionEvent] = [
            .pathChanged(ReducerFixture.offline),
            .pathChanged(ReducerFixture.online),
            .readinessChanged(.ready),
            .readinessChanged(.needsSetup([.helperNotEnabled])),
            .willSleep,
            .didWake(sleptFor: 4000),
            .wakeSettled(WakeObservation(processAlive: false, pppUp: false, underlay: ReducerFixture.wifi, pathSatisfied: true)),
            .settingsChanged(ConnectionSettings(sessionMaxMinutes: 60)),
            .timerFired(.retry, generation: 1),
            .helperConnectionLost,
            .helperEvent(HelperEvent(attemptID: ReducerFixture.id(2), date: h.now, kind: .exited(ReducerFixture.facts(.spawned)))),
        ]
        for event in nonUserEvents {
            h.send(event)
            #expect(h.model.state == failed, "\(event) must not clear failed")
        }

        let effects = h.send(.userConnect)
        #expect(effects.contains(.checkReadiness))
        #expect(h.model.ledger.consecutiveAuthFailures == 0)
        #expect(h.model.ledger.recentAuthAttempts.isEmpty)
        #expect(h.model.desired == .connected)
    }

    @Test func connectRightAfterFailureWaitsOutTheNinetySeconds() throws {
        var h = ReducerHarness()
        h.launch()
        try failAtGateway(&h)
        try h.fire(.retry)
        try failAtGateway(&h)
        let lastFailure = h.now

        h.advance(10)
        let effects = h.send(.userConnect)
        #expect(h.retryWait == RetryWait(until: lastFailure + 90, reason: .authCooldown))
        #expect(effects.persistedLedger?.consecutiveAuthFailures == 0)
        #expect(!effects.contains(.checkReadiness))
        try h.fire(.retry)
        #expect(h.attempt?.reason == .retry(.authCooldown))
    }

    @Test func reconnectNowClearsFailed() throws {
        var h = ReducerHarness()
        h.launch()
        h.send(.readinessChanged(.ready))
        let id = try #require(h.currentID)
        h.send(.otpSessionFailed(id, .authentication))
        #expect(h.model.state == .failed(.gmailAppPassword))
        #expect(!h.model.menuActions.reconnectNowEnabled)

        h.send(.userReconnectNow)
        #expect(h.attempt?.reason == .manualReconnect)
    }

    @Test func stableSessionResetsTheCounterEvenOnUserDisconnect() throws {
        var h = ReducerHarness()
        h.launch()
        try failAtGateway(&h)
        try h.fire(.retry)
        try h.runToConnected()
        #expect(h.model.ledger.consecutiveAuthFailures == 1)

        h.advance(200)
        #expect(h.send(.userDisconnect).stopReasons == [.userDisconnect])
        let effects = h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.userDisconnect), up: 200))
        #expect(effects.persistedLedger?.consecutiveAuthFailures == 0)
        #expect(h.model.state == .idle)
    }

    @Test func shortSessionEndedByTheUserKeepsTheCounter() throws {
        var h = ReducerHarness()
        h.launch()
        try failAtGateway(&h)
        try h.fire(.retry)
        try h.runToConnected()
        h.advance(60)
        h.send(.userDisconnect)
        let effects = h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.userDisconnect), up: 60))
        #expect(effects.persistedLedger == nil)
        #expect(h.model.ledger.consecutiveAuthFailures == 1)
    }

    @Test func everyLedgerChangeIsPersisted() throws {
        var h = ReducerHarness()
        var persistedCount = 0
        func step(_ h: inout ReducerHarness, _ event: ConnectionEvent) {
            let before = h.model.ledger
            let effects = h.send(event)
            if h.model.ledger != before {
                #expect(effects.persistedLedger == h.model.ledger, "\(event)")
                persistedCount += 1
            } else {
                #expect(effects.persistedLedger == nil, "\(event)")
            }
        }
        step(&h, .appLaunched(autoConnect: true, restoredLedger: nil))
        step(&h, .readinessChanged(.ready))
        let id = ReducerFixture.id(1)
        step(&h, .otpSessionReady(id))
        step(&h, .helperEvent(HelperEvent(attemptID: id, date: h.now, kind: .milestone(.connectedToGateway, tunnelIP: nil))))
        step(&h, .helperEvent(HelperEvent(attemptID: id, date: h.now, kind: .exited(ReducerFixture.facts(.connectedToGateway)))))
        h.advance(90)
        let retryGeneration = try #require(h.generation(of: .retry))
        step(&h, .timerFired(.retry, generation: retryGeneration))
        step(&h, .userReconnectNow) // ignored while connecting
        step(&h, .userDisconnect)
        step(&h, .userConnect) // manual reset
        step(&h, .profileChanged(VPNProfile(host: "vpn.example.com", port: 8443, username: "bob")))
        #expect(persistedCount == 4)
    }

    @Test func launchHonoursARestoredCooldown() throws {
        var h = ReducerHarness()
        let ledger = FailureLedger(
            profileKey: ReducerFixture.profileKey,
            consecutiveAuthFailures: 1,
            lastAuthFailureAt: h.now - 30,
            lastAuthFailureReason: .otpRejected,
            recentAuthAttempts: [h.now - 40]
        )
        let effects = h.launch(ledger: ledger)
        #expect(h.retryWait == RetryWait(until: h.now + 60, reason: .authCooldown))
        #expect(effects == [.scheduleTimer(.retry, at: h.now + 60, generation: 1), .setActivity(true)])
        #expect(h.model.ledger == ledger)
    }

    @Test func launchStaysFailedWhenTheRestoredCounterIsExhausted() {
        var h = ReducerHarness()
        let ledger = FailureLedger(
            profileKey: ReducerFixture.profileKey,
            consecutiveAuthFailures: 2,
            lastAuthFailureAt: h.now - 86_400,
            lastAuthFailureReason: .otpNotReceived(inJunk: true)
        )
        let effects = h.launch(ledger: ledger)
        #expect(h.model.state == .failed(.otpNotReceived(inJunk: true)))
        #expect(effects.isEmpty)
        #expect(h.model.menuActions.showsConnect)
    }

    @Test func launchEnforcesTheRestoredRollingCap() {
        var h = ReducerHarness()
        let attempts = (1...12).map { h.now - TimeInterval($0 * 60) }
        let effects = h.launch(ledger: FailureLedger(profileKey: ReducerFixture.profileKey, recentAuthAttempts: attempts.reversed()))
        #expect(h.model.state == .failed(.tooManyReconnects))
        #expect(effects.isEmpty)
    }

    @Test func launchDiscardsALedgerOfAnotherProfileAndPrunesOldAttempts() {
        var other = ReducerHarness()
        other.launch(ledger: FailureLedger(profileKey: "bob@vpn.example.com", consecutiveAuthFailures: 2))
        #expect(other.model.ledger == FailureLedger(profileKey: ReducerFixture.profileKey))
        #expect(other.attempt != nil)

        var stale = ReducerHarness()
        let effects = stale.launch(ledger: FailureLedger(profileKey: ReducerFixture.profileKey, recentAuthAttempts: [stale.now - 7200]))
        #expect(effects.persistedLedger == FailureLedger(profileKey: ReducerFixture.profileKey))
    }

    @Test(arguments: [
        (HelperError.Code.busy, nil as ConnectionFailureReason?),
        (.wrongPhase, nil),
        (.unknownAttempt, nil),
        (.internalError, nil),
        (.binaryUnavailable, .helper(.binaryUnavailable)),
        (.spawnFailed, .helper(.spawnFailed)),
        (.invalidRequest, .helper(.invalidRequest)),
        (.notAuthorized, .helper(.notAuthorized)),
    ])
    func connectRejectionsAreClassifiedByCode(code: HelperError.Code, failure: ConnectionFailureReason?) throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        let effects = h.send(.helperConnectRejected(id, HelperError(code, "nope")))
        #expect(effects.first == .closeOTPSession(id, checkJunk: false))
        if let failure {
            #expect(h.model.state == .failed(failure))
        } else {
            #expect(h.retryWait?.reason == .network)
        }
    }

    @Test func losingTheHelperConnectionEndsTheRun() throws {
        var connected = ReducerHarness()
        try connected.launchConnected()
        connected.send(.helperConnectionLost)
        #expect(connected.retryWait?.reason == .network)

        var connecting = ReducerHarness()
        connecting.launch()
        let id = try connecting.startTunnel()
        #expect(connecting.send(.helperConnectionLost).first == .closeOTPSession(id, checkJunk: false))
        #expect(connecting.retryWait?.reason == .network)

        var stopping = ReducerHarness()
        try stopping.launchConnected()
        stopping.send(.userDisconnect)
        stopping.send(.helperConnectionLost)
        #expect(stopping.model.state == .idle)

        var idle = ReducerHarness()
        #expect(idle.send(.helperConnectionLost) == [])
    }
}
