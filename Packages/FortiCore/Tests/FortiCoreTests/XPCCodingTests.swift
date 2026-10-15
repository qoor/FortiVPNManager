import Foundation
import Testing
import FortiShared

@Test func helperEventRoundTrips() throws {
    let id = AttemptID()
    let facts = ExitFacts(
        furthestMilestone: .otpPrompted,
        stopInitiator: .helper(.otpTimeout),
        certRejectedDigests: [String(repeating: "a", count: 64)],
        exitStatus: .signaled(signal: 15)
    )
    let event = HelperEvent(attemptID: id, date: Date(timeIntervalSince1970: 1_000), kind: .exited(facts))
    let decoded = try XPCCoding.decode(HelperEvent.self, from: XPCCoding.encode(event))
    #expect(decoded == event)
}

@Test func milestonesAreOrdered() {
    #expect(Milestone.spawned < .connectedToGateway)
    #expect(Milestone.otpSubmitted < .tunnelUp)
}
