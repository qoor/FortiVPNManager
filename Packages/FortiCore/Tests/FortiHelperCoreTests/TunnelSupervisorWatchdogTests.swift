import Darwin
@testable import FortiHelperCore
import FortiShared
import Foundation
import Synchronization
import Testing

/// Watchdogs, stop escalation and signal handling (DESIGN §3.7, §5.2 watchdog, §10).
@Suite struct TunnelSupervisorWatchdogTests {
    @Test func defaultLimitsMatchTheDesignTable() {
        let limits = WatchdogLimits()
        #expect(limits.spawnToGateway == .seconds(30))
        #expect(limits.gatewayToPromptOrAuth == .seconds(30))
        #expect(limits.otpPromptToSubmit == .seconds(65))
        #expect(limits.otpSubmitToAuth == .seconds(20))
        #expect(limits.authToTunnel == .seconds(40))
        #expect(limits.terminate == .seconds(10))
        let timing = SupervisorTiming()
        #expect(timing.outputBatchInterval == .milliseconds(100))
        #expect(timing.pipeDrainLimit == .seconds(1))
        #expect(timing.lineIdleFlush == .milliseconds(250))
    }

    @Test(arguments: [
        (Milestone.spawned, WatchdogPhase.spawnToGateway, Duration.seconds(30)),
        (.connectedToGateway, .gatewayToPromptOrAuth, .seconds(30)),
        (.otpPrompted, .otpPromptToSubmit, .seconds(65)),
        (.otpSubmitted, .otpSubmitToAuth, .seconds(20)),
        (.authenticated, .authToTunnel, .seconds(40)),
    ])
    func watchdogAfterMilestone(milestone: Milestone, phase: WatchdogPhase, limit: Duration) {
        let watchdog = WatchdogLimits().watchdog(after: milestone)
        #expect(watchdog?.phase == phase)
        #expect(watchdog?.limit == limit)
    }

    @Test func noWatchdogOnceTheTunnelIsUp() {
        #expect(WatchdogLimits().watchdog(after: .tunnelUp) == nil)
    }

    struct StallCase: Sendable, CustomTestStringConvertible {
        var stage: String
        var phase: WatchdogPhase
        var furthest: Milestone
        var submitsOTP: Bool
        var testDescription: String { stage }
    }

    @Test(arguments: [
        StallCase(stage: "spawned", phase: .spawnToGateway, furthest: .spawned, submitsOTP: false),
        StallCase(stage: "gateway", phase: .gatewayToPromptOrAuth, furthest: .connectedToGateway, submitsOTP: false),
        StallCase(stage: "otp", phase: .otpPromptToSubmit, furthest: .otpPrompted, submitsOTP: false),
        StallCase(stage: "submitted", phase: .otpSubmitToAuth, furthest: .otpSubmitted, submitsOTP: true),
        StallCase(stage: "authenticated", phase: .authToTunnel, furthest: .authenticated, submitsOTP: true),
    ])
    func watchdogStopsAStalledPhase(_ stall: StallCase) async throws {
        var limits = testLimits
        let tiny = Duration.milliseconds(150)
        switch stall.phase {
        case .spawnToGateway: limits.spawnToGateway = tiny
        case .gatewayToPromptOrAuth: limits.gatewayToPromptOrAuth = tiny
        case .otpPromptToSubmit: limits.otpPromptToSubmit = tiny
        case .otpSubmitToAuth: limits.otpSubmitToAuth = tiny
        case .authToTunnel: limits.authToTunnel = tiny
        case .terminate: Issue.record("not a phase watchdog")
        }
        let harness = try Harness(.hang, [stall.stage], limits: limits)
        defer { harness.record.remove() }
        if stall.submitsOTP {
            harness.submitOTPWhenPrompted()
        }
        let facts = try await harness.run()

        #expect(facts.watchdogFired == stall.phase)
        #expect(facts.stopInitiator == .helper(.watchdog))
        #expect(facts.furthestMilestone == stall.furthest)
        #expect(facts.exitStatus == .signaled(signal: SIGTERM))
        #expect(facts.upDuration == nil)
        let texts = harness.recorder.texts
        let fired = try #require(texts.firstIndex(of: "‹watchdog: \(stall.phase.rawValue) did not finish within 0.15s›"))
        #expect(texts[fired + 1] == "‹sent SIGTERM (reason: watchdog)›")
        #expect(texts.last == "‹exited: signal 15 (SIGTERM)›")
        expectOutputInvariants(harness)
    }

    @Test func noWatchdogOnceTheTunnelIsUpWhileRunning() async throws {
        let tiny = Duration.milliseconds(300)
        let limits = WatchdogLimits(spawnToGateway: tiny, gatewayToPromptOrAuth: tiny, otpPromptToSubmit: tiny, otpSubmitToAuth: tiny, authToTunnel: tiny, terminate: .seconds(2))
        let harness = try Harness(.hang, ["tunnel"], limits: limits)
        defer { harness.record.remove() }
        harness.submitOTPWhenPrompted()
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(.tunnelUp) })
        try await Task.sleep(for: .milliseconds(500))
        #expect(harness.recorder.exitFacts == nil)
        #expect(harness.supervisor.processID != nil)
        harness.supervisor.stop(reason: .sleep)
        let facts = try #require(await harness.recorder.waitForExit())
        #expect(facts.watchdogFired == nil)
        #expect(facts.stopInitiator == .helper(.sleep))
        #expect(facts.furthestMilestone == .tunnelUp)
    }

    /// The watchdog's SIGTERM is ignored, so SIGKILL follows after `terminate`. The first cause
    /// stays in `watchdogFired`.
    @Test func sigkillWhenSIGTERMIsIgnoredAfterAWatchdog() async throws {
        let limits = WatchdogLimits(spawnToGateway: .milliseconds(300), terminate: .milliseconds(200))
        let harness = try Harness(.hang, ["spawned", "ignore-term"], limits: limits)
        defer { harness.record.remove() }
        let facts = try await harness.run()

        #expect(facts.watchdogFired == .spawnToGateway)
        #expect(facts.stopInitiator == .helper(.watchdog))
        #expect(facts.exitStatus == .signaled(signal: SIGKILL))
        let texts = harness.recorder.texts
        #expect(texts.contains("‹sent SIGTERM (reason: watchdog)›"))
        #expect(texts.contains("‹sent SIGKILL: still running 0.2s after SIGTERM›"))
        #expect(texts.last == "‹exited: signal 9 (SIGKILL)›")
        expectOutputInvariants(harness)
    }

    /// A user stop that needs SIGKILL reports `.terminate`; routes left changed are repaired.
    @Test func sigkillAfterUserStopOnAnUpTunnel() async throws {
        let repairCalls = Mutex(0)
        let harness = try Harness(.hang, ["tunnel", "ignore-term"], limits: WatchdogLimits(terminate: .milliseconds(200)), routeRepair: {
            repairCalls.withLock { $0 += 1 }
            return .repaired
        })
        defer { harness.record.remove() }
        harness.submitOTPWhenPrompted()
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(.tunnelUp) })
        harness.supervisor.stop(reason: .userDisconnect)
        // A second stop does not change the initiator.
        harness.supervisor.stop(reason: .appQuit)
        let facts = try #require(await harness.recorder.waitForExit())

        #expect(facts.watchdogFired == .terminate)
        #expect(facts.stopInitiator == .helper(.userDisconnect))
        #expect(facts.exitStatus == .signaled(signal: SIGKILL))
        #expect(facts.routeRepairPerformed)
        #expect(repairCalls.withLock { $0 } == 1)
        #expect(harness.recorder.texts.filter { $0.hasPrefix("‹sent SIGTERM") }.count == 1)
        expectOutputInvariants(harness)
    }

    /// DESIGN §10: spawned from a background queue while the parent ignores SIGTERM, the child
    /// still ends within a second of `stop`.
    @Test func sigtermStopsAChildSpawnedFromABackgroundQueueWhileTheParentIgnoresSIGTERM() async throws {
        let harness = try Harness(.hang, ["spawned"])
        defer { harness.record.remove() }
        try await withSIGTERMIgnored {
            let startError = Mutex<HelperError?>(nil)
            await onBackgroundQueueBlockingSIGTERM {
                do throws(HelperError) {
                    try harness.supervisor.start()
                } catch {
                    startError.withLock { $0 = error }
                }
            }
            #expect(startError.withLock { $0 } == nil)
            #expect(await harness.recorder.wait { $0.reached(.spawned) })
            let stoppedAt = ContinuousClock.now
            harness.supervisor.stop(reason: .userDisconnect)
            let facts = try #require(await harness.recorder.waitForExit(timeout: .seconds(1)))
            #expect(ContinuousClock.now - stoppedAt < .seconds(1))
            #expect(facts.exitStatus == .signaled(signal: SIGTERM))
            #expect(facts.watchdogFired == nil)
            #expect(facts.stopInitiator == .helper(.userDisconnect))
        }
    }

    // MARK: - Injected clock

    @Test func watchdogFollowsTheInjectedClock() async throws {
        let clock = ManualClock()
        let harness = try Harness(.hang, ["spawned"], limits: WatchdogLimits(), clock: clock)
        defer { harness.record.remove() }
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(.spawned) })
        clock.advance(by: .milliseconds(29_900))
        try await Task.sleep(for: .milliseconds(100))
        #expect(harness.recorder.exitFacts == nil)
        #expect(harness.supervisor.processID != nil)
        clock.advance(by: .milliseconds(100))
        let facts = try #require(await harness.recorder.waitForExit())

        #expect(facts.watchdogFired == .spawnToGateway)
        #expect(facts.exitStatus == .signaled(signal: SIGTERM))
        #expect(harness.recorder.texts.contains("‹watchdog: spawnToGateway did not finish within 30s›"))
        // Dates come from the injected clock too.
        let origin = Date(timeIntervalSince1970: 1_800_000_000)
        #expect(harness.recorder.events.first?.date == origin)
        #expect(harness.recorder.events.last?.date == origin.addingTimeInterval(30))
    }

    @Test func sigkillEscalationFollowsTheInjectedClock() async throws {
        let clock = ManualClock()
        let harness = try Harness(.hang, ["spawned", "ignore-term"], limits: WatchdogLimits(), clock: clock)
        defer { harness.record.remove() }
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(.spawned) })
        // Let the script install its trap before SIGTERM arrives.
        #expect(await harness.recorder.wait { _ in harness.record.values["started"] == "yes" })
        harness.supervisor.stop(reason: .helperShutdown)
        clock.advance(by: .milliseconds(9_900))
        try await Task.sleep(for: .milliseconds(100))
        #expect(harness.recorder.exitFacts == nil)
        clock.advance(by: .milliseconds(100))
        let facts = try #require(await harness.recorder.waitForExit())

        #expect(facts.watchdogFired == .terminate)
        #expect(facts.stopInitiator == .helper(.helperShutdown))
        #expect(facts.exitStatus == .signaled(signal: SIGKILL))
        #expect(harness.recorder.texts.contains("‹sent SIGKILL: still running 10s after SIGTERM›"))
    }

    @Test func upDurationIsMeasuredWithTheInjectedClock() async throws {
        let clock = ManualClock()
        let harness = try Harness(.success, [], limits: WatchdogLimits(), clock: clock)
        defer { harness.record.remove() }
        harness.submitOTPWhenPrompted()
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(.tunnelUp) })
        clock.advance(by: .seconds(42))
        harness.supervisor.stop(reason: .proactiveReconnect)
        let facts = try #require(await harness.recorder.waitForExit())

        #expect(facts.upDuration == 42)
        #expect(facts.stopInitiator == .helper(.proactiveReconnect))
        #expect(facts.exitStatus == .exited(code: 0))
        expectOutputInvariants(harness)
    }

    /// Batches go out on the batch timer and unfinished lines on the idle-flush timer, both on
    /// the injected clock.
    @Test func batchingAndIdleFlushFollowTheInjectedClock() async throws {
        let clock = ManualClock()
        let timing = SupervisorTiming(outputBatchInterval: .milliseconds(20), lineIdleFlush: .milliseconds(250))
        let harness = try Harness(
            executable: ExecutableSpec(path: "/bin/sh", arguments: ["/bin/sh", "-c", "echo 'INFO:   first'; printf 'partial tail'; exec /bin/sleep 30"], environment: fixturePATH),
            limits: WatchdogLimits(),
            timing: timing,
            clock: clock
        )
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(.spawned) })
        // watchdog + output batch + line flush once both chunks were read.
        #expect(await harness.recorder.wait { _ in clock.pendingTimerCount == 3 })
        try await Task.sleep(for: .milliseconds(50))
        #expect(!harness.recorder.texts.contains("INFO:   first"))

        clock.advance(by: .milliseconds(20))
        #expect(await harness.recorder.wait { $0.texts.contains("INFO:   first") })
        #expect(!harness.recorder.texts.contains("partial tail"))
        #expect(harness.recorder.lines.last?.isPartial == false)
        #expect(harness.recorder.lines.last?.date == Date(timeIntervalSince1970: 1_800_000_000))

        clock.advance(by: .milliseconds(230))
        // The idle flush ran and armed a new batch timer: watchdog + output batch.
        #expect(await harness.recorder.wait { _ in clock.pendingTimerCount == 2 })
        #expect(!harness.recorder.texts.contains("partial tail"))
        clock.advance(by: .milliseconds(20))
        #expect(await harness.recorder.wait { $0.texts.contains("partial tail") })
        let partial = try #require(harness.recorder.lines.last)
        #expect(partial.isPartial)
        #expect(partial.date == Date(timeIntervalSince1970: 1_800_000_000.25))
        #expect(harness.recorder.events.count == 1)

        harness.supervisor.stop(reason: .userDisconnect)
        let facts = try #require(await harness.recorder.waitForExit())
        #expect(facts.exitStatus == .signaled(signal: SIGTERM))
        expectOutputInvariants(harness)
    }

    // MARK: - Formatting

    @Test(arguments: [
        (Duration.seconds(30), "30s"),
        (.milliseconds(150), "0.15s"),
        (.milliseconds(1_500), "1.5s"),
        (.milliseconds(250), "0.25s"),
        (.zero, "0s"),
    ])
    func durationFormatting(duration: Duration, expected: String) {
        #expect(TunnelSupervisor.format(duration) == expected)
    }

    @Test(arguments: [
        (ProcessExitStatus.exited(code: 0), "code 0"),
        (.exited(code: 254), "code 254"),
        (.signaled(signal: SIGTERM), "signal 15 (SIGTERM)"),
        (.signaled(signal: SIGKILL), "signal 9 (SIGKILL)"),
        (.signaled(signal: SIGUSR1), "signal 30"),
        (.spawnFailed(errno: ENOENT), "spawn failed (No such file or directory (errno 2))"),
    ])
    func exitStatusDescription(status: ProcessExitStatus, expected: String) {
        #expect(TunnelSupervisor.describe(status) == expected)
    }
}
