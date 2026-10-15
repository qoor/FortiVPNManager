import Foundation
import Testing
import FortiShared
@testable import FortiCore

@Suite("Readiness gate and profile changes")
struct ReadinessProfileTests {
    @Test func missingSetupBlocksTheAttempt() throws {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        h.send(.userConnect)
        let effects = h.send(.readinessChanged(.needsSetup([.vpnPassword, .helperNotEnabled])))
        #expect(effects == [.cancelTimer(.attemptDeadline), .setActivity(false)])
        #expect(!effects.startsOTPSession)
        #expect(h.model.state == .needsSetup([.vpnPassword, .helperNotEnabled]))
        #expect(h.model.icon == .error)
        #expect(h.model.attention == .needsSetup([.vpnPassword, .helperNotEnabled]))
        #expect(h.model.menuActions == ConnectionMenuActions(showsConnect: true, showsDisconnect: true, reconnectNowEnabled: false))
    }

    @Test func setupCompletedWhileConnectionWantedConnectsAutomatically() throws {
        var h = ReducerHarness()
        h.launch()
        h.send(.readinessChanged(.needsSetup([.gmailAppPassword])))
        #expect(h.model.state == .needsSetup([.gmailAppPassword]))
        #expect(h.model.desired == .connected)

        let effects = h.send(.readinessChanged(.ready))
        let id2 = ReducerFixture.id(2)
        #expect(effects == [
            .startOTPSession(id2),
            .scheduleTimer(.attemptDeadline, at: h.now + 150, generation: 2),
            .setActivity(true),
        ])
        #expect(h.attempt?.reason == .setupCompleted)
        #expect(h.attempt?.stage == .preparingOTP)
        try h.runToConnected()
    }

    @Test func setupCompletedWhileDisconnectedGoesIdle() {
        var h = ReducerHarness()
        #expect(h.launch(autoConnect: false) == [.checkReadiness])
        h.send(.readinessChanged(.needsSetup([.vpnPassword])))
        #expect(h.model.state == .needsSetup([.vpnPassword]))
        #expect(h.model.menuActions == ConnectionMenuActions(showsConnect: true, showsDisconnect: false, reconnectNowEnabled: false))
        #expect(h.send(.readinessChanged(.ready)) == [])
        #expect(h.model.state == .idle)
    }

    @Test func unknownReadinessChangesNothing() {
        var h = ReducerHarness()
        h.launch()
        let before = h.model.state
        #expect(h.send(.readinessChanged(.unknown)) == [])
        #expect(h.model.state == before)
    }

    @Test func setupLossWhileWaitingShowsSetup() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        #expect(h.send(.readinessChanged(.needsSetup([.helperVersionMismatch]))) == [.cancelTimer(.retry), .setActivity(false)])
        #expect(h.model.state == .needsSetup([.helperVersionMismatch]))
    }

    @Test func readinessChangesDuringARunApplyToTheNextAttempt() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        #expect(h.send(.readinessChanged(.needsSetup([.vpnPassword]))) == [])
        try h.runMilestones(id: id, through: .tunnelUp)
        #expect(h.connected != nil)
        #expect(h.send(.readinessChanged(.needsSetup([.vpnPassword]))) == [])
        #expect(h.connected != nil)
    }

    @Test func usernameChangeResetsTheLedgerAndRequiresSetupWhenThePasswordIsMissing() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .connectedToGateway)
        h.exit(ReducerFixture.facts(.connectedToGateway, authFail: true))
        #expect(h.model.ledger.consecutiveAuthFailures == 1)

        let bob = VPNProfile(host: "vpn.example.com", port: 8443, username: "bob", trustedCerts: [ReducerFixture.pinOld])
        let effects = h.send(.profileChanged(bob))
        #expect(effects == [.checkReadiness, .persistLedger(FailureLedger(profileKey: "bob@vpn.example.com"))])
        #expect(h.model.readiness == .unknown)
        #expect(h.model.profile == bob)

        h.send(.readinessChanged(.needsSetup([.vpnPassword])))
        #expect(h.model.state == .needsSetup([.vpnPassword]))
        #expect(h.model.timers.isEmpty)

        // Entering the password completes setup; desired is still "connected".
        h.send(.readinessChanged(.ready))
        #expect(h.attempt?.reason == .setupCompleted)
        try h.startTunnel()
        #expect(h.effects.tunnelStarts.first?.username == "bob")
    }

    static let bob = VPNProfile(host: "vpn.example.com", port: 8443, username: "bob", trustedCerts: [ReducerFixture.pinOld])

    /// §5.7: an account change applies from the next connection; the attempt in flight keeps the account
    /// its readiness check was issued for and never starts openfortivpn for the unverified one.
    @Test func accountChangeWhileTheGmailSessionStartsAppliesToTheNextAttempt() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try #require(h.attempt?.attemptID)
        h.send(.readinessChanged(.ready))
        #expect(h.attempt?.stage == .preparingOTP)

        #expect(h.send(.profileChanged(Self.bob)).contains(.checkReadiness))
        #expect(h.send(.readinessChanged(.needsSetup([.vpnPassword]))) == [])
        let effects = h.send(.otpSessionReady(id))
        #expect(effects.tunnelStarts == [ReducerFixture.profile])
        #expect(h.model.profile == Self.bob)

        // The next attempt checks readiness for bob and stops at setup.
        h.exit(ReducerFixture.facts(.spawned), id: id)
        try h.fire(.retry)
        #expect(h.effects.contains(.checkReadiness))
        h.send(.readinessChanged(.needsSetup([.vpnPassword])))
        #expect(h.model.state == .needsSetup([.vpnPassword]))

        // Once bob's password is stored, bob is used.
        h.send(.readinessChanged(.ready))
        try h.startTunnel()
        #expect(h.effects.tunnelStarts == [Self.bob])
    }

    @Test func accountChangeWhileCheckingReadinessKeepsTheAttemptOnItsOwnAccount() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try #require(h.attempt?.attemptID)
        #expect(h.attempt?.stage == .checkingReadiness)
        h.send(.profileChanged(Self.bob))
        // The launch check (alice) answers first, then the check for bob.
        h.send(.readinessChanged(.ready))
        #expect(h.send(.readinessChanged(.needsSetup([.vpnPassword]))) == [])
        #expect(h.send(.otpSessionReady(id)).tunnelStarts == [ReducerFixture.profile])
    }

    @Test func profileChangeWhileIdleShowsSetup() {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        h.send(.readinessChanged(.ready))
        let effects = h.send(.profileChanged(VPNProfile(host: "other.example.com", port: 8443, username: "alice")))
        #expect(effects.contains(.checkReadiness))
        h.send(.readinessChanged(.needsSetup([.vpnPassword])))
        #expect(h.model.state == .needsSetup([.vpnPassword]))
        #expect(h.model.icon == .error)
    }

    @Test func profileChangeWhileConnectedKeepsTheSessionUntilItEnds() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.send(.profileChanged(VPNProfile(host: "vpn.example.com", port: 8443, username: "bob")))
        h.send(.readinessChanged(.needsSetup([.vpnPassword])))
        #expect(h.connected != nil)

        h.advance(3 * 3600)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 3 * 3600))
        try h.fire(.retry)
        #expect(h.effects.contains(.checkReadiness), "the next attempt checks readiness again")
        h.send(.readinessChanged(.needsSetup([.vpnPassword])))
        #expect(h.model.state == .needsSetup([.vpnPassword]))
    }

    @Test func profileChangeKeepsFailedLatched() throws {
        var h = ReducerHarness()
        h.launch()
        h.send(.readinessChanged(.ready))
        h.send(.otpSessionFailed(ReducerFixture.id(1), .authentication))
        #expect(h.model.state == .failed(.gmailAppPassword))
        h.send(.profileChanged(VPNProfile(host: "vpn.example.com", port: 8443, username: "bob")))
        h.send(.readinessChanged(.needsSetup([.vpnPassword])))
        #expect(h.model.state == .failed(.gmailAppPassword))
        h.send(.readinessChanged(.ready))
        #expect(h.model.state == .failed(.gmailAppPassword))
    }

    @Test func realmOnlyChangeNeedsNoRecheck() {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        var profile = ReducerFixture.profile
        profile.realm = "staff"
        #expect(h.send(.profileChanged(profile)) == [])
        #expect(h.model.profile.realm == "staff")
    }

    @Test(arguments: [
        (ReadinessInputs(configValid: true, accountComplete: true, hasVPNPassword: true, hasGmailAppPassword: true, helperEnabled: true, helperBuildMatches: true),
         Readiness.ready),
        (ReadinessInputs(configValid: false, accountComplete: true, hasVPNPassword: true, hasGmailAppPassword: true, helperEnabled: true, helperBuildMatches: true),
         .needsSetup([.invalidConfig])),
        (ReadinessInputs(configValid: true, accountComplete: false, hasVPNPassword: false, hasGmailAppPassword: false, helperEnabled: true, helperBuildMatches: true),
         .needsSetup([.accountIncomplete, .vpnPassword, .gmailAppPassword])),
        (ReadinessInputs(configValid: true, accountComplete: true, hasVPNPassword: true, hasGmailAppPassword: true, helperEnabled: false, helperBuildMatches: false),
         .needsSetup([.helperNotEnabled])),
        (ReadinessInputs(configValid: true, accountComplete: true, hasVPNPassword: true, hasGmailAppPassword: true, helperEnabled: true, helperBuildMatches: false),
         .needsSetup([.helperVersionMismatch])),
    ])
    func readinessInputsMapToSetupItems(inputs: ReadinessInputs, expected: Readiness) {
        #expect(inputs.readiness == expected)
    }

    @Test func readinessFromMissingItemsIsCanonical() {
        #expect(Readiness(missing: []) == .ready)
        #expect(Readiness(missing: [.helperNotEnabled, .vpnPassword, .vpnPassword]) == .needsSetup([.vpnPassword, .helperNotEnabled]))
    }
}

@Suite("Presentation (§5.6)")
struct PresentationTests {
    static let attempt = AttemptContext(
        attemptID: ReducerFixture.id(1), reason: .user, chainID: ReducerFixture.id(1), certRetryUsed: false,
        startedAt: ReducerFixture.t0, stage: .tunnel
    )
    static let session = ConnectedContext(
        attemptID: ReducerFixture.id(1), since: ReducerFixture.t0, tunnelIP: "10.0.0.1",
        plannedReconnectAt: ReducerFixture.t0 + 10_500, underlay: ReducerFixture.wifi
    )

    static let iconCases: [(ConnectionState, ConnectionIcon)] = [
        (.connected(session), .connected),
        (.idle, .disconnected),
        (.connecting(attempt), .inProgress),
        (.waitingRetry(RetryWait(until: ReducerFixture.t0, reason: .network)), .inProgress),
        (.waitingNetwork(NetworkWait(reason: .sleep, delayAfterPath: 0)), .inProgress),
        (.disconnecting(DisconnectContext(attemptID: ReducerFixture.id(1), reason: .userDisconnect, after: .idle)), .inProgress),
        (.failed(.tooManyReconnects), .error),
        (.needsSetup([.vpnPassword]), .error),
    ]

    @Test(arguments: iconCases)
    func iconFollowsTheTable(state: ConnectionState, icon: ConnectionIcon) {
        #expect(state.icon == icon)
    }

    static let menuCases: [(DesiredState, ConnectionState, ConnectionMenuActions)] = [
        (.disconnected, .idle, ConnectionMenuActions(showsConnect: true, showsDisconnect: false, reconnectNowEnabled: false)),
        (.connected, .connecting(attempt), ConnectionMenuActions(showsConnect: false, showsDisconnect: true, reconnectNowEnabled: false)),
        (.connected, .connected(session), ConnectionMenuActions(showsConnect: false, showsDisconnect: true, reconnectNowEnabled: true)),
        (.connected, .waitingRetry(RetryWait(until: ReducerFixture.t0, reason: .authFailure(.otpRejected))),
         ConnectionMenuActions(showsConnect: false, showsDisconnect: true, reconnectNowEnabled: true)),
        (.connected, .waitingNetwork(NetworkWait(reason: .network, delayAfterPath: 10)),
         ConnectionMenuActions(showsConnect: false, showsDisconnect: true, reconnectNowEnabled: false)),
        (.connected, .failed(.otpRejected), ConnectionMenuActions(showsConnect: true, showsDisconnect: true, reconnectNowEnabled: false)),
        (.disconnected, .needsSetup([.vpnPassword]), ConnectionMenuActions(showsConnect: true, showsDisconnect: false, reconnectNowEnabled: false)),
        (.disconnected, .disconnecting(DisconnectContext(attemptID: ReducerFixture.id(1), reason: .userDisconnect, after: .idle)),
         ConnectionMenuActions(showsConnect: true, showsDisconnect: false, reconnectNowEnabled: false)),
    ]

    @Test(arguments: menuCases)
    func menuActionsFollowTheSpec(desired: DesiredState, state: ConnectionState, expected: ConnectionMenuActions) {
        var model = ConnectionModel(profile: ReducerFixture.profile, attemptIDSeed: ReducerFixture.seed)
        model.desired = desired
        model.state = state
        #expect(model.menuActions == expected)
    }

    @Test func plannedAndRetryDatesAreExposed() {
        var model = ConnectionModel(profile: ReducerFixture.profile, attemptIDSeed: ReducerFixture.seed)
        model.state = .connected(Self.session)
        #expect(model.nextPlannedReconnect == ReducerFixture.t0 + 10_500)
        #expect(model.nextRetryAt == nil)
        #expect(model.attention == nil)
        model.state = .waitingRetry(RetryWait(until: ReducerFixture.t0 + 90, reason: .authCooldown))
        #expect(model.nextPlannedReconnect == nil)
        #expect(model.nextRetryAt == ReducerFixture.t0 + 90)
        model.state = .failed(.gmailAppPassword)
        #expect(model.attention == .failed(.gmailAppPassword))
    }
}
