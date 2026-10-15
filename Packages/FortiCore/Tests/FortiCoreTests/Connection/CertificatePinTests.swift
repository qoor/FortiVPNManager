import Foundation
import Testing
import FortiShared
@testable import FortiCore

@Suite("Gateway certificate auto-trust (§5.4, §6.2 row 1)")
struct CertificatePinTests {
    @Test func rejectionLearnsThePinAndRetriesImmediatelyWithoutCounting() throws {
        var h = ReducerHarness()
        h.launch()
        let id1 = try h.startTunnel()
        #expect(h.effects.tunnelStarts == [ReducerFixture.profile])
        h.advance(1)
        h.milestone(.spawned)

        let effects = h.exit(ReducerFixture.facts(.spawned, certs: [ReducerFixture.digestA, ReducerFixture.digestA], certAt: .spawned))
        let id2 = ReducerFixture.id(2)
        #expect(effects == [
            .persistTrustedCerts([ReducerFixture.digestA, ReducerFixture.pinOld]),
            .closeOTPSession(id1, checkJunk: false),
            .checkReadiness,
            .scheduleTimer(.attemptDeadline, at: h.now + 150, generation: 2),
        ])
        let retry = try #require(h.attempt)
        #expect(retry.attemptID == id2)
        #expect(retry.reason == .certificateRetry)
        #expect(retry.chainID == id1)
        #expect(retry.certRetryUsed)
        #expect(h.model.ledger == FailureLedger(profileKey: ReducerFixture.profileKey))

        // The learned pin travels with the very next connect request (one openfortivpn run from now on).
        try h.startTunnel()
        #expect(h.effects.tunnelStarts.first?.trustedCerts == [ReducerFixture.digestA, ReducerFixture.pinOld])
        try h.runMilestones(id: id2, through: .tunnelUp)
        #expect(h.connected != nil)

        // ...and the request after that, too.
        h.send(.userReconnectNow)
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.proactiveReconnect), up: 30))
        try h.startTunnel()
        #expect(h.effects.tunnelStarts.first?.trustedCerts == [ReducerFixture.digestA, ReducerFixture.pinOld])
    }

    @Test func secondRejectionInTheSameChainIsANetworkError() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned, certs: [ReducerFixture.digestA], certAt: .spawned))
        try h.startTunnel()

        let effects = h.exit(ReducerFixture.facts(.spawned, certs: [ReducerFixture.digestB], certAt: .spawned))
        #expect(effects.persistedCerts == [ReducerFixture.digestB, ReducerFixture.digestA, ReducerFixture.pinOld])
        #expect(h.retryWait == RetryWait(until: h.now + 10, reason: .network))
        #expect(h.model.ledger.consecutiveAuthFailures == 0)

        // A new chain gets its own single immediate retry.
        try h.fire(.retry)
        let chainStart = try #require(h.attempt)
        #expect(!chainStart.certRetryUsed)
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned, certs: [ReducerFixture.digestC], certAt: .spawned))
        #expect(h.attempt?.reason == .certificateRetry)
        #expect(h.attempt?.chainID == chainStart.attemptID)
    }

    @Test func rejectionAfterOTPSubmissionCountsAndStillPins() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpSubmitted)
        let effects = h.exit(ReducerFixture.facts(.otpSubmitted, certs: [ReducerFixture.digestA], certAt: .otpSubmitted))
        #expect(effects.persistedCerts == [ReducerFixture.digestA, ReducerFixture.pinOld])
        #expect(h.retryWait?.reason == .authFailure(.certificateRejectedAfterLogin))
        #expect(effects.persistedLedger?.consecutiveAuthFailures == 1)
    }

    @Test func pinsAreCappedAtFourNewestFirst() throws {
        var profile = ReducerFixture.profile
        profile.trustedCerts = [ReducerFixture.digestA, ReducerFixture.digestB, ReducerFixture.digestC, ReducerFixture.digestD]
        var h = ReducerHarness(profile: profile)
        h.launch()
        try h.startTunnel()
        let effects = h.exit(ReducerFixture.facts(.spawned, certs: [ReducerFixture.pinOld], certAt: .spawned))
        let expected = [ReducerFixture.pinOld, ReducerFixture.digestA, ReducerFixture.digestB, ReducerFixture.digestC]
        #expect(effects.persistedCerts == expected)
        #expect(h.model.profile.trustedCerts == expected)
    }

    @Test func digestsAreNormalizedAndUnusableOnesDoNotRetry() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        let upper = ReducerFixture.digestA.uppercased()
        h.exit(ReducerFixture.facts(.spawned, certs: [upper], certAt: .spawned))
        #expect(h.model.profile.trustedCerts == [ReducerFixture.digestA, ReducerFixture.pinOld])
        #expect(h.attempt?.reason == .certificateRetry)

        var bad = ReducerHarness()
        bad.launch()
        try bad.startTunnel()
        let effects = bad.exit(ReducerFixture.facts(.spawned, certs: ["zz"], certAt: .spawned))
        #expect(effects.persistedCerts == nil)
        #expect(bad.retryWait?.reason == .network)
    }

    @Test func pinsLearnedFromAStoppedRunAreKept() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        h.send(.userDisconnect)
        let effects = h.exit(ReducerFixture.facts(.spawned, initiator: .helper(.userDisconnect), certs: [ReducerFixture.digestA], certAt: .spawned), id: id)
        #expect(effects.persistedCerts == [ReducerFixture.digestA, ReducerFixture.pinOld])
        #expect(h.model.state == .idle)
    }

    @Test(arguments: [
        ("host", VPNProfile(host: "vpn2.example.com", port: 8443, username: "alice", trustedCerts: [ReducerFixture.pinOld]), true),
        ("port", VPNProfile(host: "vpn.example.com", port: 443, username: "alice", trustedCerts: [ReducerFixture.pinOld]), true),
        ("username", VPNProfile(host: "vpn.example.com", port: 8443, username: "bob", trustedCerts: [ReducerFixture.pinOld]), false),
        ("realm", VPNProfile(host: "vpn.example.com", port: 8443, username: "alice", realm: "staff", trustedCerts: [ReducerFixture.pinOld]), false),
    ])
    func gatewayChangeClearsThePins(field: String, profile: VPNProfile, clears: Bool) {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        let effects = h.send(.profileChanged(profile))
        if clears {
            #expect(effects.first == .persistTrustedCerts([]), "\(field)")
            #expect(h.model.profile.trustedCerts == [], "\(field)")
        } else {
            #expect(effects.persistedCerts == nil, "\(field)")
            #expect(h.model.profile.trustedCerts == [ReducerFixture.pinOld], "\(field)")
        }
    }

    @Test func gatewayChangeKeepsPinsTheUserSuppliedForTheNewGateway() {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        let supplied = VPNProfile(host: "vpn2.example.com", port: 8443, username: "alice", trustedCerts: [ReducerFixture.digestB])
        let effects = h.send(.profileChanged(supplied))
        #expect(effects.persistedCerts == nil)
        #expect(h.model.profile == supplied)

        // Nothing to clear: no effect.
        var empty = ReducerHarness(profile: VPNProfile(host: "vpn.example.com", port: 8443, username: "alice"))
        empty.launch(autoConnect: false)
        #expect(empty.send(.profileChanged(VPNProfile(host: "vpn3.example.com", port: 8443, username: "alice"))).persistedCerts == nil)
    }

    /// A run that was started for the old gateway must not plant its certificate in the new gateway's
    /// pins, and its rejection does not use up the new gateway's one immediate certificate retry.
    @Test(arguments: [false, true])
    func rejectionFromTheOldGatewayIsNotPinnedForTheNewOne(tunnelStarted: Bool) throws {
        var h = ReducerHarness()
        h.launch()
        h.send(.readinessChanged(.ready))
        let id = try #require(h.attempt?.attemptID)
        if tunnelStarted { h.send(.otpSessionReady(id)) }

        let moved = VPNProfile(host: "vpn2.example.com", port: 8443, username: "alice", trustedCerts: [ReducerFixture.pinOld])
        #expect(h.send(.profileChanged(moved)).persistedCerts == [])
        if !tunnelStarted {
            #expect(h.send(.otpSessionReady(id)).tunnelStarts.map(\.host) == ["vpn.example.com"])
        }

        let effects = h.exit(ReducerFixture.facts(.spawned, certs: [ReducerFixture.digestA], certAt: .spawned), id: id)
        #expect(effects.persistedCerts == nil)
        #expect(h.model.profile.trustedCerts == [])
        let retry = try #require(h.attempt)
        #expect(retry.reason == .certificateRetry)
        #expect(!retry.certRetryUsed, "the new gateway still gets its own immediate retry")

        // The new gateway's own rejection is learned and retried once.
        let id2 = try h.startTunnel()
        #expect(h.effects.tunnelStarts.map(\.host) == ["vpn2.example.com"])
        let second = h.exit(ReducerFixture.facts(.spawned, certs: [ReducerFixture.digestB], certAt: .spawned), id: id2)
        #expect(second.persistedCerts == [ReducerFixture.digestB])
        #expect(h.attempt?.reason == .certificateRetry)
        try h.startTunnel()
        #expect(h.effects.tunnelStarts.first?.trustedCerts == [ReducerFixture.digestB])
    }

    @Test func handEditedPinsAreAdoptedForTheNextConnect() throws {
        var h = ReducerHarness()
        h.launch(autoConnect: false)
        var edited = ReducerFixture.profile
        edited.trustedCerts = [ReducerFixture.digestC]
        #expect(h.send(.profileChanged(edited)) == [])
        h.send(.userConnect)
        try h.startTunnel()
        #expect(h.effects.tunnelStarts.first?.trustedCerts == [ReducerFixture.digestC])
    }
}
