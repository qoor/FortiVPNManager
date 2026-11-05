import FortiHelperCore
import FortiShared
import Foundation
import Testing

/// stdin injection rules.
@Suite struct TunnelSupervisorInputTests {
    /// Well-formed codes pass the format check and are only refused for the phase.
    @Test(arguments: ["123456", "1234567", "12345678", "000000"])
    func wellFormedOTPsReachThePhaseCheck(code: String) {
        let harness = Harness(.hang, ["spawned"])
        #expect(throws: HelperError(.wrongPhase, "openfortivpn is not running")) {
            try harness.supervisor.submitOTP(code)
        }
    }

    @Test(arguments: [
        "", "12345", "123456789", "12345a", " 123456", "123456 ", "123456\n", "12 3456",
        "１２３４５６",  // fullwidth digits
        "١٢٣٤٥٦",  // Arabic-Indic digits
        "+123456", "-123456", "12345\u{0}6",
    ])
    func malformedOTPsAreRejectedBeforeAnyPhaseCheck(code: String) throws {
        // Not even started: format is checked first.
        let harness = Harness(.hang, ["spawned"])
        #expect(throws: HelperError(.invalidRequest, "OTP must be 6 to 8 ASCII digits")) {
            try harness.supervisor.submitOTP(code)
        }
    }

    @Test func otpBeforeStartIsWrongPhase() throws {
        let harness = Harness(.hang, ["spawned"])
        #expect(throws: HelperError(.wrongPhase, "openfortivpn is not running")) {
            try harness.supervisor.submitOTP(testOTP)
        }
    }

    /// The OTP is refused before the prompt (the script stalls at that stage, so the state is stable).
    @Test(arguments: [("spawned", Milestone.spawned), ("gateway", .connectedToGateway)])
    func otpBeforeThePromptIsWrongPhase(stage: String, milestone: Milestone) async throws {
        let harness = Harness(.hang, [stage])
        defer { harness.record.remove() }
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(milestone) })
        #expect(throws: HelperError(.wrongPhase, "openfortivpn is not waiting for an OTP")) {
            try harness.supervisor.submitOTP(testOTP)
        }
        harness.supervisor.stop(reason: .userDisconnect)
        let facts = try #require(await harness.recorder.waitForExit())
        #expect(facts.furthestMilestone == milestone)
        #expect(!harness.recorder.texts.contains("‹OTP sent›"))
    }

    /// Accepted once while prompted; refused when repeated and after a stop request.
    @Test func otpIsAcceptedOnceWhilePrompted() async throws {
        let harness = Harness(.hang, ["submitted"])
        defer { harness.record.remove() }
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(.otpPrompted) })
        try harness.supervisor.submitOTP(testOTP)
        #expect(await harness.recorder.wait { $0.reached(.otpSubmitted) })
        #expect(throws: HelperError(.wrongPhase, "an OTP was already submitted for this attempt")) {
            try harness.supervisor.submitOTP("654321")
        }
        #expect(await harness.recorder.wait { _ in harness.record.values["otp_sha256"] != nil })
        harness.supervisor.stop(reason: .userDisconnect)
        #expect(throws: HelperError(.wrongPhase, "openfortivpn is not running")) {
            try harness.supervisor.submitOTP(testOTP)
        }
        let facts = try #require(await harness.recorder.waitForExit())
        #expect(facts.furthestMilestone == .otpSubmitted)
        #expect(facts.stopInitiator == .helper(.userDisconnect))
        #expect(harness.recorder.texts.filter { $0 == "‹OTP sent›" }.count == 1)
        #expect(harness.record.values["otp_sha256"] == sha256Hex(testOTP))
        expectOutputInvariants(harness)
    }

    @Test func otpWhileStoppingIsWrongPhase() async throws {
        let harness = Harness(.hang, ["otp", "ignore-term"], limits: WatchdogLimits(terminate: .milliseconds(300)))
        defer { harness.record.remove() }
        try harness.supervisor.start()
        #expect(await harness.recorder.wait { $0.reached(.otpPrompted) })
        harness.supervisor.stop(reason: .otpTimeout)
        #expect(throws: HelperError(.wrongPhase, "openfortivpn is not running")) {
            try harness.supervisor.submitOTP(testOTP)
        }
        let facts = try #require(await harness.recorder.waitForExit())
        #expect(facts.furthestMilestone == .otpPrompted)
        #expect(facts.stopInitiator == .helper(.otpTimeout))
    }

    @Test func passwordIsWrittenOnceAndOnlyBeforeTheGateway() async throws {
        let script = """
        printf 'VPN account password: '
        IFS= read -r pw
        printf 'first_len=%s\\n' "${#pw}" >> "$1"
        printf '\\nVPN account password: '
        /bin/sleep 0.2
        printf '\\nINFO:   Connected to gateway.\\nVPN account password: '
        /bin/sleep 0.2
        exit 0
        """
        let harness = Harness(script: script)
        defer { harness.record.remove() }
        _ = try await harness.run()
        let texts = harness.recorder.texts
        #expect(texts.filter { $0 == "‹password sent›" }.count == 1)
        #expect(texts.filter { $0 == "‹password prompt ignored: already answered or not expected at this stage›" }.count == 2)
        #expect(harness.record.values["first_len"] == "\(testPassword.utf8.count)")
        expectOutputInvariants(harness)
    }

    @Test func passwordIsNotWrittenAfterAStopRequest() async throws {
        let script = """
        trap '' TERM
        echo 'started=yes' >> "$1"
        /bin/sleep 0.3
        printf 'VPN account password: '
        IFS= read -r pw
        """
        let harness = Harness(script: script, limits: WatchdogLimits(terminate: .milliseconds(700)))
        defer { harness.record.remove() }
        try harness.supervisor.start()
        // SIGTERM must arrive after the trap is installed.
        #expect(await harness.recorder.wait { _ in harness.record.values["started"] == "yes" })
        harness.supervisor.stop(reason: .appQuit)
        let facts = try #require(await harness.recorder.waitForExit())
        #expect(!harness.recorder.texts.contains("‹password sent›"))
        #expect(facts.stopInitiator == .helper(.appQuit))
        #expect(facts.exitStatus == .signaled(signal: SIGKILL))
    }

    @Test func startTwiceIsWrongPhase() async throws {
        let harness = Harness(.authFailed, ["password"])
        defer { harness.record.remove() }
        try harness.supervisor.start()
        #expect(throws: HelperError(.wrongPhase, "attempt already started")) {
            try harness.supervisor.start()
        }
        _ = await harness.recorder.waitForExit()
    }

    @Test func stopBeforeStartDoesNothing() async throws {
        let harness = Harness(.hang, ["spawned"])
        harness.supervisor.stop(reason: .userDisconnect)
        try await Task.sleep(for: .milliseconds(30))
        #expect(harness.recorder.delivered.isEmpty)
        #expect(harness.supervisor.processID == nil)
    }
}
