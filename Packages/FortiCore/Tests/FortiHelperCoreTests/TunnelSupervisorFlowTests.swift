import Darwin
import FortiHelperCore
import FortiShared
import Foundation
import Synchronization
import Testing

/// End-to-end runs of `TunnelSupervisor` against fake openfortivpn scripts (DESIGN §10 TunnelSupervisor).
@Suite struct TunnelSupervisorFlowTests {
    static let fullFlow: [Milestone] = [.spawned, .connectedToGateway, .otpPrompted, .otpSubmitted, .authenticated, .tunnelUp]

    @Test(arguments: ["tokeninfo", "htmlform"])
    func successThenUserDisconnect(promptPath: String) async throws {
        let repairCalls = Mutex(0)
        let harness = try Harness(.success, [promptPath], routeRepair: {
            repairCalls.withLock { $0 += 1 }
            return .repaired
        })
        defer { harness.record.remove() }
        harness.submitOTPWhenPrompted()
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(.tunnelUp) })
        #expect(harness.supervisor.processID != nil)
        harness.supervisor.stop(reason: .userDisconnect)
        let facts = try #require(await harness.recorder.waitForExit())

        let recorder = harness.recorder
        #expect(recorder.milestones == Self.fullFlow)
        #expect(recorder.events.contains { $0.kind == .milestone(.tunnelUp, tunnelIP: "10.212.134.200") })
        #expect(recorder.events.allSatisfy { event in
            if case .milestone(let milestone, let ip) = event.kind, milestone != .tunnelUp { ip == nil } else { true }
        })
        #expect(facts.furthestMilestone == .tunnelUp)
        #expect(facts.stopInitiator == .helper(.userDisconnect))
        #expect(facts.watchdogFired == nil)
        #expect(facts.certRejectedDigests.isEmpty)
        #expect(facts.certRejectedAt == nil)
        #expect(!facts.authFailLineSeen)
        #expect(try #require(facts.upDuration) >= 0)
        #expect(facts.exitStatus == .exited(code: 0))
        // "Restoring routes..." completed, so no repair.
        #expect(!facts.routeRepairPerformed)
        #expect(repairCalls.withLock { $0 } == 0)
        #expect(harness.supervisor.isFinished)
        #expect(harness.supervisor.processID == nil)

        // The script received exactly the password and the OTP, and stdin was closed after the OTP.
        let record = harness.record.values
        #expect(record["password_eof"] == "no")
        #expect(record["password_len"] == "\(testPassword.utf8.count)")
        #expect(record["password_sha256"] == sha256Hex(testPassword))
        #expect(record["otp_eof"] == "no")
        #expect(record["otp_sha256"] == sha256Hex(testOTP))
        #expect(record["stdin_closed"] == "yes")
        #expect(record["term"] == "yes")

        expectOutputInvariants(harness)
        let texts = recorder.texts
        #expect(texts.first?.hasPrefix("‹attempt started: pid ") == true)
        #expect(texts.first?.hasSuffix("/bin/sh \(Fixture.success.path) \(harness.record.path) \(promptPath)›") == true)
        let order = [
            "‹password sent›",
            "INFO:   Connected to gateway.",
            "‹OTP sent›",
            "INFO:   Authenticated.",
            "INFO:   Tunnel is up and running.",
            "‹sent SIGTERM (reason: userDisconnect)›",
            "INFO:   Restoring routes...",
            "INFO:   Logged out.",
            "‹exited: code 0›",
        ]
        let positions = order.map { texts.firstIndex(of: $0) }
        #expect(!positions.contains(nil), "missing one of \(order) in \(texts)")
        #expect(positions.compactMap { $0 } == positions.compactMap { $0 }.sorted())
        #expect(texts.last == "‹exited: code 0›")
        #expect(recorder.lines.contains { $0.source == .stderr && $0.text == "Thu Oct 15 12:12:05 2026 : Modem hangup" })
        #expect(recorder.lines.contains { $0.source == .stdout && $0.text == "add host 203.0.113.10: gateway 192.168.1.1" })
        // The prompts are part of the log (as a partial line when the input took longer than 250 ms).
        #expect(texts.contains("VPN account password: "))
        // Each milestone follows the line that produced it.
        #expect(linesBefore(recorder) { $0 == .milestone(.connectedToGateway, tunnelIP: nil) }.contains("INFO:   Connected to gateway."))
        #expect(linesBefore(recorder) { $0 == .milestone(.authenticated, tunnelIP: nil) }.contains("INFO:   Authenticated."))
        #expect(linesBefore(recorder) { if case .milestone(.tunnelUp, _) = $0 { true } else { false } }.last == "INFO:   Tunnel is up and running.")
    }

    @Test func certificateRejectedBeforeLogin() async throws {
        let harness = try Harness(.certRejected)
        defer { harness.record.remove() }
        let facts = try await harness.run()

        #expect(harness.recorder.milestones == [.spawned])
        #expect(facts == ExitFacts(
            furthestMilestone: .spawned,
            stopInitiator: .none,
            certRejectedDigests: [rejectedDigest],
            certRejectedAt: .spawned,
            exitStatus: .exited(code: 1),
            lastOutputSeq: facts.lastOutputSeq
        ))
        // The password prompt comes before TLS (main.c:732), so it was answered.
        #expect(harness.record.values["password_sha256"] == sha256Hex(testPassword))
        let texts = harness.recorder.texts
        #expect(texts.filter { $0 == "ERROR:      trusted-cert = \(rejectedDigest)" }.count == 2)
        #expect(texts.contains("INFO:   Could not log out."))
        expectOutputInvariants(harness)
    }

    @Test(arguments: [
        ("password", Milestone.connectedToGateway, [Milestone.spawned, .connectedToGateway]),
        ("otp", .otpSubmitted, [.spawned, .connectedToGateway, .otpPrompted, .otpSubmitted]),
    ])
    func authenticationFailure(mode: String, furthest: Milestone, milestones: [Milestone]) async throws {
        let harness = try Harness(.authFailed, [mode])
        defer { harness.record.remove() }
        harness.submitOTPWhenPrompted()
        let facts = try await harness.run()

        #expect(harness.recorder.milestones == milestones)
        #expect(facts == ExitFacts(
            furthestMilestone: furthest,
            stopInitiator: .none,
            authFailLineSeen: true,
            exitStatus: .exited(code: 1),
            lastOutputSeq: facts.lastOutputSeq
        ))
        if mode == "otp" {
            #expect(harness.record.values["otp_sha256"] == sha256Hex(testOTP))
        }
        expectOutputInvariants(harness)
    }

    @Test func serverEndsTheSession() async throws {
        let repairCalls = Mutex(0)
        let harness = try Harness(.serverCut, ["cut"], routeRepair: {
            repairCalls.withLock { $0 += 1 }
            return .repaired
        })
        defer { harness.record.remove() }
        harness.submitOTPWhenPrompted()
        let facts = try await harness.run()

        #expect(harness.recorder.milestones == Self.fullFlow)
        #expect(facts.furthestMilestone == .tunnelUp)
        #expect(facts.stopInitiator == .none)
        #expect(facts.watchdogFired == nil)
        #expect(!facts.authFailLineSeen)
        #expect(facts.exitStatus == .exited(code: 1))
        // The script stays up for 0.3 s.
        let upDuration = try #require(facts.upDuration)
        #expect(upDuration >= 0.15 && upDuration < 5)
        #expect(!facts.routeRepairPerformed)
        #expect(repairCalls.withLock { $0 } == 0)
        let lines = harness.recorder.lines
        #expect(lines.contains { $0.source == .stderr && $0.text == "Thu Oct 15 12:12:05 2026 : Connection terminated." })
        #expect(lines.contains { $0.source == .stdout && $0.text == "ERROR:  pppd: The link was terminated by the modem hanging up." })
        #expect(harness.recorder.texts.last == "‹exited: code 1›")
        expectOutputInvariants(harness)
    }

    /// openfortivpn killed after "Setting new routes..." never restores them: the hook runs once
    /// and its result becomes `routeRepairPerformed`.
    @Test(arguments: [true, false])
    func crashAfterRoutesWereChangedTriggersRouteRepair(repairSucceeds: Bool) async throws {
        let repairCalls = Mutex(0)
        let harness = try Harness(.serverCut, ["crash"], routeRepair: {
            repairCalls.withLock { $0 += 1 }
            return repairSucceeds ? .repaired : .failed
        })
        defer { harness.record.remove() }
        harness.submitOTPWhenPrompted()
        let facts = try await harness.run()

        #expect(repairCalls.withLock { $0 } == 1)
        #expect(facts.routeRepairPerformed == repairSucceeds)
        #expect(facts.exitStatus == .signaled(signal: SIGKILL))
        #expect(facts.furthestMilestone == .tunnelUp)
        #expect(facts.stopInitiator == .none)
        let texts = harness.recorder.texts
        #expect(Array(texts.suffix(2)) == ["‹exited: signal 9 (SIGKILL)›", repairSucceeds ? "‹routes repaired›" : "‹route repair failed — see the Helper log›"])
        expectOutputInvariants(harness)
    }

    @Test func crashWithoutRouteRepairHook() async throws {
        let harness = try Harness(.serverCut, ["crash"])
        defer { harness.record.remove() }
        harness.submitOTPWhenPrompted()
        let facts = try await harness.run()
        #expect(!facts.routeRepairPerformed)
        #expect(facts.exitStatus == .signaled(signal: SIGKILL))
    }

    @Test func routeRepairIsNotCalledWhenRoutesWereNeverTouched() async throws {
        let repairCalls = Mutex(0)
        let harness = try Harness(.authFailed, ["password"], routeRepair: {
            repairCalls.withLock { $0 += 1 }
            return .repaired
        })
        defer { harness.record.remove() }
        let facts = try await harness.run()
        #expect(repairCalls.withLock { $0 } == 0)
        #expect(!facts.routeRepairPerformed)
    }

    /// The child closes stdin and exits right after printing the prompt: the password write
    /// fails with EPIPE, which is logged and otherwise treated as the child exiting.
    @Test func passwordWriteAfterChildClosedStdin() async throws {
        let harness = try Harness(.earlyExit)
        defer { harness.record.remove() }
        let facts = try await harness.run()

        #expect(facts == ExitFacts(furthestMilestone: .spawned, stopInitiator: .none, exitStatus: .exited(code: 1), lastOutputSeq: facts.lastOutputSeq))
        let texts = harness.recorder.texts
        #expect(texts.contains("‹input failed: openfortivpn exited before reading it (EPIPE)›"))
        #expect(!texts.contains("‹password sent›"))
        #expect(harness.record.values["exited"] == "yes")
        expectOutputInvariants(harness)
        // No OTP can be submitted to a finished run.
        #expect(throws: HelperError(.wrongPhase, "openfortivpn is not running")) { try harness.supervisor.submitOTP(testOTP) }
    }

    @Test func immediateExitBeforeAnyPrompt() async throws {
        let harness = try Harness(script: "echo 'ERROR:  Specify a valid host:port couple.'; exit 1")
        defer { harness.record.remove() }
        let facts = try await harness.run()
        #expect(harness.recorder.milestones == [.spawned])
        #expect(facts == ExitFacts(furthestMilestone: .spawned, stopInitiator: .none, exitStatus: .exited(code: 1), lastOutputSeq: facts.lastOutputSeq))
        #expect(harness.recorder.texts.contains("ERROR:  Specify a valid host:port couple."))
        expectOutputInvariants(harness)
    }

    @Test func spawnFailureIsReportedAsExited() async throws {
        let harness = try Harness(executable: ExecutableSpec(path: "/nonexistent/openfortivpn", arguments: ["openfortivpn", "-c", "/x"], environment: fixturePATH))
        try harness.supervisor.start()
        let facts = try #require(await harness.recorder.waitForExit())

        #expect(facts == ExitFacts(furthestMilestone: nil, stopInitiator: .none, exitStatus: .spawnFailed(errno: ENOENT), lastOutputSeq: 1))
        #expect(harness.recorder.milestones.isEmpty)
        #expect(harness.recorder.lines.count == 1)
        #expect(harness.recorder.texts.first?.hasPrefix("‹spawn failed: /nonexistent/openfortivpn: ") == true)
        #expect(harness.recorder.lines.first?.source == .helper)
        #expect(harness.supervisor.isFinished)
        #expect(harness.supervisor.furthestMilestone == nil)
        // Stop after the end is a no-op.
        harness.supervisor.stop(reason: .userDisconnect)
        try await Task.sleep(for: .milliseconds(30))
        #expect(harness.recorder.events.count == 1)
    }

    /// pppd keeps openfortivpn's stderr (its `logfd 2`) and lives in its own session, so the
    /// pipe may stay open after openfortivpn exits: output is still collected for at most
    /// `pipeDrainLimit`, then `.exited` is sent anyway.
    @Test func pipeHeldOpenByAnotherProcessGroup() async throws {
        let script = """
        set -m
        ( /bin/sleep 0.1; echo 'Thu Oct 15 12:12:06 2026 : late pppd line' >&2; exec /bin/sleep 3 ) </dev/null >/dev/null &
        set +m
        echo 'INFO:   bye'
        exit 0
        """
        let timing = SupervisorTiming(outputBatchInterval: .milliseconds(20), pipeDrainLimit: .milliseconds(700))
        let harness = try Harness(script: script, timing: timing)
        defer { harness.record.remove() }
        let started = ContinuousClock.now
        let facts = try await harness.run()
        let elapsed = ContinuousClock.now - started

        #expect(facts.exitStatus == .exited(code: 0))
        #expect(elapsed >= .milliseconds(650))
        #expect(elapsed < .milliseconds(2_500))
        let lines = harness.recorder.lines
        #expect(lines.contains { $0.source == .stderr && $0.text == "Thu Oct 15 12:12:06 2026 : late pppd line" })
        #expect(harness.recorder.texts.contains("‹output pipes still open after 0.7s; not waiting any longer›"))
        expectOutputInvariants(harness)
    }

    @Test func outputSequenceContinuesFromFirstOutputSeq() async throws {
        let harness = try Harness(.authFailed, ["password"], firstOutputSeq: 1_000)
        defer { harness.record.remove() }
        let facts = try await harness.run()
        #expect(harness.recorder.lines.first?.seq == 1_000)
        #expect(facts.lastOutputSeq == 1_000 + UInt64(harness.recorder.lines.count) - 1)
        expectOutputInvariants(harness, firstSeq: 1_000)
    }

    /// Output is delivered in several batches (timer and before each event), never after `.exited`.
    @Test func outputIsBatchedAndFlushedBeforeEvents() async throws {
        let harness = try Harness(.serverCut, ["cut"])
        defer { harness.record.remove() }
        harness.submitOTPWhenPrompted()
        _ = try await harness.run()
        let recorder = harness.recorder
        // Every event except `.otpPrompted` (reported from the unfinished prompt line) follows at
        // least one new line, which is delivered first.
        #expect(recorder.batches.count >= Self.fullFlow.count)
        var sawOutput = false
        for entry in recorder.delivered {
            switch entry {
            case .output: sawOutput = true
            case .event(let event):
                if event.kind != .milestone(.otpPrompted, tunnelIP: nil) {
                    #expect(sawOutput, "no output delivered right before \(event.kind)")
                }
                sawOutput = false
            }
        }
        // Lines arrive in seq order across batches.
        let seqs = recorder.batches.flatMap { $0.map(\.seq) }
        #expect(seqs == seqs.sorted())
        expectOutputInvariants(harness)
    }

    @Test func linesWithoutNewlineAreFlushedAsPartialAtExit() async throws {
        let harness = try Harness(script: "printf 'INFO:   complete\\nunfinished tail'; printf 'err tail' >&2; exit 0")
        defer { harness.record.remove() }
        _ = try await harness.run()
        let lines = harness.recorder.lines.filter { $0.source != .helper }
        #expect(lines.map(\.text).sorted() == ["INFO:   complete", "err tail", "unfinished tail"])
        #expect(lines.first { $0.text == "INFO:   complete" }?.isPartial == false)
        #expect(lines.first { $0.text == "unfinished tail" }?.isPartial == true)
        #expect(lines.first { $0.text == "err tail" }?.isPartial == true)
        expectOutputInvariants(harness)
    }
}
