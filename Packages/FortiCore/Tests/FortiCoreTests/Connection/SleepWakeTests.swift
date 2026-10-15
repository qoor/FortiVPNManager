import Foundation
import Testing
import FortiShared
@testable import FortiCore

struct WakeCase: Sendable, CustomTestStringConvertible {
    let name: String
    var settings = ConnectionSettings()
    let sleep: TimeInterval
    let observation: WakeObservation
    /// `nil` means the tunnel is kept.
    let expected: AttemptReason?

    var testDescription: String { name }
}

private func observe(alive: Bool = true, ppp: Bool = true, underlay: Underlay? = ReducerFixture.wifi, settled: Bool = true) -> WakeObservation {
    WakeObservation(processAlive: alive, pppUp: ppp, underlay: underlay, pathSatisfied: settled)
}

@Suite("§6.3 sleep and wake")
struct SleepWakeTests {
    static let afterSleepOn = ConnectionSettings(reconnectAfterSleepEnabled: true, minSleepMinutes: 30)

    /// Connected at t0+12, planned reconnect at t0+12+175 min (10,512 s).
    static let wakeCases: [WakeCase] = [
        WakeCase(name: "1. planned time passed", sleep: 4 * 3600, observation: observe(), expected: .proactive),
        WakeCase(name: "1 beats 2: planned passed and process gone", sleep: 4 * 3600, observation: observe(alive: false), expected: .proactive),
        WakeCase(name: "2. process gone", sleep: 600, observation: observe(alive: false), expected: .tunnelLost),
        WakeCase(name: "2. ppp interface gone", sleep: 600, observation: observe(ppp: false), expected: .tunnelLost),
        WakeCase(name: "2 beats 3: process gone and underlay changed", sleep: 600,
                 observation: observe(alive: false, underlay: ReducerFixture.ethernet), expected: .tunnelLost),
        WakeCase(name: "3. underlay interface changed", sleep: 600, observation: observe(underlay: ReducerFixture.ethernet), expected: .networkChanged),
        WakeCase(name: "3. same interface, new address", sleep: 600, observation: observe(underlay: ReducerFixture.otherWifi), expected: .networkChanged),
        WakeCase(name: "3 beats 4: underlay changed after a long sleep", settings: afterSleepOn, sleep: 3600,
                 observation: observe(underlay: ReducerFixture.ethernet), expected: .networkChanged),
        WakeCase(name: "4. reconnect_after_sleep, slept 31 min", settings: afterSleepOn, sleep: 31 * 60, observation: observe(), expected: .afterSleep),
        WakeCase(name: "4. reconnect_after_sleep, slept exactly the minimum", settings: afterSleepOn, sleep: 30 * 60, observation: observe(), expected: .afterSleep),
        WakeCase(name: "5. reconnect_after_sleep, short sleep keeps", settings: afterSleepOn, sleep: 10 * 60, observation: observe(), expected: nil),
        WakeCase(name: "5. reconnect_after_sleep disabled keeps", sleep: 60 * 60, observation: observe(), expected: nil),
        WakeCase(name: "5. underlay unknown after wake keeps", sleep: 600, observation: observe(underlay: nil), expected: nil),
        WakeCase(name: "not settled within 30 s keeps even with a dead process", sleep: 600,
                 observation: observe(alive: false, ppp: false, underlay: nil, settled: false), expected: nil),
    ]

    @Test(arguments: wakeCases)
    func wakeRulesWhileConnected(_ wakeCase: WakeCase) throws {
        var h = ReducerHarness(settings: wakeCase.settings)
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)
        h.advance(100)
        #expect(h.send(.willSleep) == [.cancelTimer(.proactiveReconnect)])
        h.advance(wakeCase.sleep)
        #expect(h.send(.didWake(sleptFor: wakeCase.sleep)) == [])

        h.advance(5)
        let effects = h.send(.wakeSettled(wakeCase.observation))
        if let reason = wakeCase.expected {
            let stop: StopReason = reason == .proactive ? .proactiveReconnect : .reconnect
            #expect(effects.stopReasons == [stop])
            #expect(h.disconnecting?.after == .reconnect(reason))
            h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(stop), up: h.now.timeIntervalSince(ReducerFixture.t0)), id: id)
            #expect(h.attempt?.reason == reason)
        } else {
            #expect(effects.stopReasons.isEmpty)
            #expect(h.connected != nil)
            // The proactive timer is re-armed with a fresh generation.
            #expect(effects.scheduled(.proactiveReconnect) != nil)
        }
    }

    @Test func unsettledWakeAfterThePlannedTimeDefersTheProactiveReconnect() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.send(.willSleep)
        h.advance(4 * 3600)
        h.send(.didWake(sleptFor: 4 * 3600))
        h.advance(30)
        let effects = h.send(.wakeSettled(observe(underlay: nil, settled: false)))
        #expect(effects == [.scheduleTimer(.proactiveReconnect, at: h.now + 60, generation: 4)])
        #expect(h.connected?.plannedReconnectAt == h.now + 60)

        #expect(try h.fire(.proactiveReconnect).stopReasons == [.proactiveReconnect])
        #expect(h.disconnecting?.after == .reconnect(.proactive))
    }

    @Test func proactiveTimerIsSuspendedUntilTheWakeIsEvaluated() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let planned = try #require(h.connected?.plannedReconnectAt)
        let oldGeneration = try #require(h.generation(of: .proactiveReconnect))

        h.send(.willSleep)
        h.advance(60)
        #expect(h.send(.didWake(sleptFor: 60)) == [])
        #expect(h.model.timers[.proactiveReconnect] == nil)
        // A path change while the wake is pending is left to `wakeSettled`.
        #expect(h.send(.pathChanged(PathSnapshot(isSatisfied: true, underlay: ReducerFixture.ethernet))) == [])

        let effects = h.send(.wakeSettled(observe()))
        #expect(effects == [.scheduleTimer(.proactiveReconnect, at: planned, generation: oldGeneration + 1)])
        #expect(h.send(.timerFired(.proactiveReconnect, generation: oldGeneration)) == [])
        #expect(h.connected != nil)
    }

    @Test func sleepWhileConnectingStopsTheAttemptAndResumesAfterWake() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .otpPrompted)

        #expect(h.send(.willSleep) == [
            .stopTunnel(id, .sleep),
            .closeOTPSession(id, checkJunk: false),
            .cancelTimer(.attemptDeadline),
            .cancelTimer(.otpDeadline),
        ])
        #expect(h.disconnecting?.after == .resumeAfterSleep)

        let exit = h.exit(ReducerFixture.facts(.otpPrompted, initiator: .helper(.sleep)))
        #expect(h.networkWait == NetworkWait(reason: .sleep, delayAfterPath: 0))
        #expect(exit.persistedLedger == nil, "an attempt aborted by sleep is not an authentication failure")

        h.advance(600)
        #expect(h.send(.didWake(sleptFor: 600)) == [])
        #expect(h.networkWait != nil)
        let resumed = h.send(.wakeSettled(observe(alive: false, ppp: false)))
        #expect(resumed.contains(.checkReadiness))
        #expect(h.attempt?.reason == .afterSleep)
    }

    @Test func pathAfterWakeAlsoResumes() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.send(.willSleep)
        h.exit(ReducerFixture.facts(.spawned, initiator: .helper(.sleep)))
        // Path events during sleep are ignored.
        #expect(h.send(.pathChanged(ReducerFixture.online)) == [])
        h.send(.didWake(sleptFor: 60))
        #expect(h.send(.pathChanged(PathSnapshot(isSatisfied: true, underlay: ReducerFixture.ethernet))).contains(.checkReadiness))
        // The later `wakeSettled` is harmless.
        #expect(h.send(.wakeSettled(observe(underlay: ReducerFixture.ethernet))) == [])
    }

    /// The §6.3 rules are for a tunnel that was up across the sleep. A session that came up after the wake
    /// (path returned before `wakeSettled`) must survive a stale observation or a long sleep.
    @Test(arguments: [false, true])
    func wakeSettledLeavesASessionEstablishedAfterTheWakeAlone(afterSleepOn: Bool) throws {
        var h = ReducerHarness(settings: afterSleepOn ? Self.afterSleepOn : ConnectionSettings())
        h.launch()
        try h.startTunnel()
        h.exit(ReducerFixture.facts(.spawned))
        #expect(h.retryWait?.reason == .network)
        h.send(.willSleep)
        h.advance(3600)
        h.send(.didWake(sleptFor: 3600))
        h.advance(1)
        #expect(h.send(.pathChanged(ReducerFixture.online)).contains(.checkReadiness))
        #expect(h.attempt?.reason == .afterSleep)
        try h.runToConnected()
        let session = try #require(h.connected)
        #expect(h.model.timers[.proactiveReconnect] == nil, "suspended until the wake is evaluated")

        h.advance(2)
        // Sampled before the new tunnel came up (default settings), or accurate after a long sleep.
        let observation = afterSleepOn ? observe() : observe(alive: false, ppp: false)
        let effects = h.send(.wakeSettled(observation))
        #expect(effects.stopReasons.isEmpty)
        #expect(h.connected == session)
        let planned = try #require(session.plannedReconnectAt)
        let generation = try #require(h.generation(of: .proactiveReconnect))
        #expect(effects == [.scheduleTimer(.proactiveReconnect, at: planned, generation: generation)])
    }

    @Test func wakeWithoutNetworkKeepsWaiting() throws {
        var h = ReducerHarness()
        h.launch()
        h.send(.willSleep)
        h.send(.didWake(sleptFor: 60))
        #expect(h.send(.wakeSettled(observe(alive: false, ppp: false, underlay: nil, settled: false))) == [])
        #expect(h.networkWait?.reason == .sleep)
        #expect(h.send(.pathChanged(ReducerFixture.online)).contains(.checkReadiness))
    }

    @Test(arguments: [false, true])
    func sleepBeforeTheTunnelStartedJustWaits(sessionOpen: Bool) throws {
        var h = ReducerHarness()
        h.launch()
        if sessionOpen { h.send(.readinessChanged(.ready)) }
        let id = try #require(h.attempt?.attemptID)
        let effects = h.send(.willSleep)
        let close: [ConnectionEffect] = sessionOpen ? [.closeOTPSession(id, checkJunk: false)] : []
        #expect(effects == close + [.cancelTimer(.attemptDeadline)])
        #expect(h.networkWait == NetworkWait(reason: .sleep, delayAfterPath: 0))
        #expect(h.send(.otpSessionReady(id)) == [])
    }

    @Test func sleepWhileWaitingRetryCancelsTheTimerButKeepsTheAuthGap() throws {
        var h = ReducerHarness()
        h.launch()
        let id = try h.startTunnel()
        try h.runMilestones(id: id, through: .connectedToGateway)
        h.exit(ReducerFixture.facts(.connectedToGateway, authFail: true))
        let failedAt = h.now

        #expect(h.send(.willSleep) == [.cancelTimer(.retry)])
        #expect(h.networkWait?.reason == .sleep)
        h.advance(40)
        h.send(.didWake(sleptFor: 40))
        h.send(.wakeSettled(observe()))
        #expect(h.retryWait == RetryWait(until: failedAt + 90, reason: .authCooldown))
        try h.fire(.retry)
        #expect(h.attempt != nil)
    }

    @Test func tunnelThatDiesDuringSleepReconnectsAfterWake() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.send(.willSleep)
        h.advance(3 * 3600)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 3 * 3600))
        #expect(h.networkWait?.reason == .sleep)
        #expect(h.model.timers.isEmpty)

        h.send(.didWake(sleptFor: 3 * 3600))
        #expect(h.send(.wakeSettled(observe(alive: false, ppp: false))).contains(.checkReadiness))
    }

    @Test func sleepDuringAPlannedReconnectWaitsForWake() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)
        try h.fire(.proactiveReconnect)
        h.send(.willSleep)
        #expect(h.disconnecting?.after == .resumeAfterSleep)
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.proactiveReconnect), up: 10_500), id: id)
        #expect(h.networkWait?.reason == .sleep)
    }
}

@Suite("Network path while connected (§5.5)")
struct PathChangeTests {
    @Test func underlayChangeReconnects() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)
        let effects = h.send(.pathChanged(PathSnapshot(isSatisfied: true, underlay: ReducerFixture.ethernet)))
        #expect(effects == [.stopTunnel(id, .reconnect), .cancelTimer(.proactiveReconnect)])
        #expect(h.disconnecting?.after == .reconnect(.networkChanged))
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.reconnect), up: 500))
        #expect(h.attempt?.reason == .networkChanged)
    }

    @Test(arguments: [
        PathSnapshot(isSatisfied: true, underlay: ReducerFixture.wifi),
        PathSnapshot(isSatisfied: false, underlay: nil),
        PathSnapshot(isSatisfied: true, underlay: nil),
        PathSnapshot(isSatisfied: false, underlay: ReducerFixture.ethernet),
    ])
    func otherPathEventsKeepTheTunnel(snapshot: PathSnapshot) throws {
        var h = ReducerHarness()
        try h.launchConnected()
        #expect(h.send(.pathChanged(snapshot)) == [])
        #expect(h.connected?.underlay == ReducerFixture.wifi)
    }

    @Test func flapBackToTheSameUnderlayKeepsTheTunnel() throws {
        var h = ReducerHarness()
        try h.launchConnected()
        h.send(.pathChanged(ReducerFixture.offline))
        #expect(h.send(.pathChanged(ReducerFixture.online)) == [])
        #expect(h.connected != nil)
    }

    @Test func unknownUnderlayIsAdoptedThenWatched() throws {
        var h = ReducerHarness(path: nil)
        try h.launchConnected()
        #expect(h.connected?.underlay == nil)
        #expect(h.send(.pathChanged(ReducerFixture.online)) == [])
        #expect(h.connected?.underlay == ReducerFixture.wifi)
        #expect(h.send(.pathChanged(PathSnapshot(isSatisfied: true, underlay: ReducerFixture.ethernet))).stopReasons == [.reconnect])
    }

    @Test func pathEventsAreIgnoredWhileConnectingAndDisconnecting() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        #expect(h.send(.pathChanged(PathSnapshot(isSatisfied: true, underlay: ReducerFixture.ethernet))) == [])
        #expect(h.send(.pathChanged(ReducerFixture.offline)) == [])
        #expect(h.attempt?.pathChanged == true)
        #expect(h.attempt?.stopRequested == nil)

        var stopping = ReducerHarness()
        try stopping.launchConnected()
        stopping.send(.userDisconnect)
        #expect(stopping.send(.pathChanged(PathSnapshot(isSatisfied: true, underlay: ReducerFixture.ethernet))) == [])
        #expect(stopping.disconnecting?.after == .idle)
    }

    @Test func repeatedIdenticalSnapshotsAreNotAChange() throws {
        var h = ReducerHarness()
        h.launch()
        try h.startTunnel()
        h.send(.pathChanged(ReducerFixture.online))
        #expect(h.attempt?.pathChanged == false)
    }
}
