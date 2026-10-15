import Foundation
import Testing
import FortiShared
@testable import FortiCore

@Suite("Retry policy and value types")
struct RetryPolicyTests {
    @Test(arguments: [(1, 10.0), (2, 20.0), (3, 40.0), (4, 80.0), (5, 160.0), (6, 300.0), (7, 300.0), (60, 300.0), (0, 10.0)])
    func networkBackoffDoublesUpToFiveMinutes(failures: Int, expected: TimeInterval) {
        #expect(RetryPolicy.networkBackoff(afterFailures: failures) == expected)
    }

    @Test func plannedReconnectIsSessionMaxMinusMargin() {
        let connectedAt = ReducerFixture.t0
        let planned = RetryPolicy.plannedReconnect(connectedAt: connectedAt, settings: ConnectionSettings(), now: connectedAt)
        #expect(planned == connectedAt + 175 * 60)
    }

    @Test func plannedReconnectIsNilWhenDisabled() {
        let settings = ConnectionSettings(proactiveReconnectEnabled: false)
        #expect(RetryPolicy.plannedReconnect(connectedAt: ReducerFixture.t0, settings: settings, now: ReducerFixture.t0) == nil)
    }

    @Test func plannedReconnectIsClampedToAMinuteFromNow() {
        let settings = ConnectionSettings(sessionMaxMinutes: 10, proactiveMarginMinutes: 5)
        let now = ReducerFixture.t0 + 20 * 60
        #expect(RetryPolicy.plannedReconnect(connectedAt: ReducerFixture.t0, settings: settings, now: now) == now + 60)
        // A margin larger than the session cannot produce a past date either.
        let silly = ConnectionSettings(sessionMaxMinutes: 5, proactiveMarginMinutes: 10)
        #expect(RetryPolicy.plannedReconnect(connectedAt: ReducerFixture.t0, settings: silly, now: ReducerFixture.t0) == ReducerFixture.t0 + 60)
    }

    // MARK: FailureLedger

    @Test func rollingWindowExcludesAttemptsAnHourOld() {
        let now = ReducerFixture.t0 + 7_200
        let ledger = FailureLedger(profileKey: "k", recentAuthAttempts: [
            now - 3_600, // exactly one hour old: out
            now - 3_599, // in
            now - 10, // in
        ])
        #expect(ledger.authAttempts(asOf: now) == 2)
        var pruned = ledger
        pruned.prune(asOf: now)
        #expect(pruned.recentAuthAttempts == [now - 3_599, now - 10])
    }

    @Test func cooldownEndIsNinetySecondsAfterTheLastAuthFailure() {
        var ledger = FailureLedger(profileKey: "k")
        #expect(ledger.cooldownEnd == nil)
        ledger.recordAuthFailure(.otpRejected, at: ReducerFixture.t0)
        #expect(ledger.cooldownEnd == ReducerFixture.t0 + 90)
        #expect(ledger.consecutiveAuthFailures == 1)
        #expect(ledger.lastAuthFailureReason == .otpRejected)
    }

    @Test func manualResetKeepsTheCooldown() {
        var ledger = FailureLedger(
            profileKey: "k",
            consecutiveAuthFailures: 2,
            lastAuthFailureAt: ReducerFixture.t0,
            lastAuthFailureReason: .otpRejected,
            recentAuthAttempts: [ReducerFixture.t0]
        )
        ledger.resetForManualAction()
        #expect(ledger.consecutiveAuthFailures == 0)
        #expect(ledger.recentAuthAttempts.isEmpty)
        #expect(ledger.cooldownEnd == ReducerFixture.t0 + 90)
    }

    @Test func ledgerRoundTripsThroughJSON() throws {
        let ledger = FailureLedger(
            profileKey: ReducerFixture.profileKey,
            consecutiveAuthFailures: 1,
            lastAuthFailureAt: ReducerFixture.t0,
            lastAuthFailureReason: .otpNotReceived(inJunk: true),
            recentAuthAttempts: [ReducerFixture.t0 - 60, ReducerFixture.t0]
        )
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        let decoded = try decoder.decode(FailureLedger.self, from: encoder.encode(ledger))
        #expect(decoded == ledger)

        let helperReason = ConnectionFailureReason.helper(.binaryUnavailable)
        #expect(try decoder.decode(ConnectionFailureReason.self, from: encoder.encode(helperReason)) == helperReason)
    }

    // MARK: TrustedCertPins

    @Test(arguments: [
        (String(repeating: "a", count: 64), true),
        (String(repeating: "0", count: 64), true),
        (String(repeating: "A", count: 64), false),
        (String(repeating: "a", count: 63), false),
        (String(repeating: "a", count: 65), false),
        (String(repeating: "g", count: 64), false),
        (" " + String(repeating: "a", count: 63), false),
        ("", false),
    ])
    func digestValidation(digest: String, valid: Bool) {
        #expect(TrustedCertPins.isValidDigest(digest) == valid)
    }

    @Test func normalizationLowercasesDedupesAndDropsInvalid() {
        let upper = String(repeating: "A", count: 64)
        #expect(TrustedCertPins.normalized([upper, ReducerFixture.digestA, "junk", ReducerFixture.digestB]) == [ReducerFixture.digestA, ReducerFixture.digestB])
    }

    @Test func mergingPutsNewestFirstAndKeepsFour() {
        let existing = [ReducerFixture.digestA, ReducerFixture.digestB, ReducerFixture.digestC, ReducerFixture.digestD]
        let merged = TrustedCertPins.merging([ReducerFixture.pinOld], into: existing)
        #expect(merged == [ReducerFixture.pinOld, ReducerFixture.digestA, ReducerFixture.digestB, ReducerFixture.digestC])
        // Re-learning a pin moves it to the front instead of duplicating it.
        #expect(TrustedCertPins.merging([ReducerFixture.digestC], into: existing) == [ReducerFixture.digestC, ReducerFixture.digestA, ReducerFixture.digestB, ReducerFixture.digestD])
        #expect(TrustedCertPins.merging([], into: []) == [])
    }

    // MARK: AttemptIDGenerator / VPNProfile

    @Test func attemptIDsAreDeterministicPerSeed() {
        var first = AttemptIDGenerator(seed: ReducerFixture.seed)
        var second = AttemptIDGenerator(seed: ReducerFixture.seed)
        let a = (first.next(), first.next(), first.next())
        let b = (second.next(), second.next(), second.next())
        #expect(a == b)
        #expect(Set([a.0, a.1, a.2]).count == 3)
        #expect(a.0.uuidString == "00000000-0000-0000-0000-000000000001")

        var other = AttemptIDGenerator(seed: ReducerFixture.stranger)
        #expect(other.next() != a.0)
    }

    @Test func connectRequestCarriesThePins() {
        var profile = ReducerFixture.profile
        profile.realm = ""
        profile.trustedCerts = [ReducerFixture.digestA, ReducerFixture.pinOld]
        let request = profile.connectRequest(attemptID: ReducerFixture.id(1), password: "pw")
        #expect(request == ConnectRequest(
            attemptID: ReducerFixture.id(1),
            host: "vpn.example.com",
            port: 8443,
            username: "alice",
            realm: nil,
            trustedCerts: [ReducerFixture.digestA, ReducerFixture.pinOld],
            password: "pw"
        ))
        profile.realm = "staff"
        #expect(profile.connectRequest(attemptID: ReducerFixture.id(1), password: "pw").realm == "staff")
        #expect(profile.profileKey == "alice@vpn.example.com")
    }
}
