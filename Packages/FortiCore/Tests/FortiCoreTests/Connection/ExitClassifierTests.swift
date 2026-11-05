import Foundation
import Testing
import FortiShared
@testable import FortiCore

struct ClassifierCase: Sendable, CustomTestStringConvertible {
    let name: String
    let input: ExitClassifier.Input
    let expected: ExitClassifier.Outcome

    var testDescription: String { name }

    init(_ name: String, _ input: ExitClassifier.Input, _ expected: ExitClassifier.Outcome) {
        self.name = name
        self.input = input
        self.expected = expected
    }
}

private typealias Input = ExitClassifier.Input
private let digest = ReducerFixture.digestA

@Suite("Exit classification")
struct ExitClassifierTests {
    static let cases: [ClassifierCase] = [
        // Certificate rejection before otpSubmitted.
        ClassifierCase("cert rejected during TLS, first in chain",
                       Input(facts: certFacts(.spawned, [digest], .spawned)), .certificateRetry),
        ClassifierCase("cert rejected, duplicate digests, first in chain",
                       Input(facts: certFacts(.spawned, [digest, digest], .spawned)), .certificateRetry),
        ClassifierCase("cert rejected, chain already retried",
                       Input(facts: certFacts(.spawned, [digest], .spawned), certRetryUsed: true), .networkError),
        ClassifierCase("cert rejected without certRejectedAt uses furthest milestone",
                       Input(facts: certFacts(.connectedToGateway, [digest], nil)), .certificateRetry),
        ClassifierCase("cert rejected but no usable digest",
                       Input(facts: certFacts(.spawned, ["not-a-digest"], .spawned)), .networkError),
        ClassifierCase("cert rejection beats the path-change rule",
                       Input(facts: certFacts(.connectedToGateway, [digest], .connectedToGateway), pathChangedDuringAttempt: true),
                       .certificateRetry),
        ClassifierCase("cert rejected at otpPrompted is still before submission",
                       Input(facts: certFacts(.otpPrompted, [digest], .otpPrompted)), .certificateRetry),

        // Stable tunnel ended by the server.
        ClassifierCase("tunnel up 3h, nobody stopped it",
                       Input(facts: ReducerFixture.facts(.tunnelUp, up: 10_800)), .serverSessionEnded),
        ClassifierCase("tunnel up exactly 120 s",
                       Input(facts: ReducerFixture.facts(.tunnelUp, up: 120)), .serverSessionEnded),
        ClassifierCase("helper did not report up-time, app measured it",
                       Input(facts: ReducerFixture.facts(.tunnelUp), observedUpDuration: 600), .serverSessionEnded),
        ClassifierCase("facts lag behind, reducer saw tunnelUp",
                       Input(facts: ReducerFixture.facts(.authenticated, up: 300), knownMilestone: .tunnelUp), .serverSessionEnded),
        ClassifierCase("stable tunnel stopped by the helper itself",
                       Input(facts: ReducerFixture.facts(.tunnelUp, initiator: .helper(.helperShutdown), up: 5_000)), .networkError),

        // Never reached the gateway.
        ClassifierCase("spawn failed", Input(facts: ReducerFixture.facts(nil)), .networkError),
        ClassifierCase("spawned only", Input(facts: ReducerFixture.facts(.spawned)), .networkError),
        ClassifierCase("spawn-to-gateway watchdog",
                       Input(facts: ReducerFixture.facts(.spawned, initiator: .helper(.watchdog))), .networkError),
        ClassifierCase("whole-attempt deadline before gateway",
                       Input(facts: ReducerFixture.facts(.spawned, initiator: .helper(.attemptTimeout)), appStop: .attemptTimeout), .networkError),

        // Gateway reached, no OTP prompt.
        ClassifierCase("rejected at the gateway",
                       Input(facts: ReducerFixture.facts(.connectedToGateway)), .authFailure(.authenticationRejected)),
        ClassifierCase("gateway failure while the path changed",
                       Input(facts: ReducerFixture.facts(.connectedToGateway), pathChangedDuringAttempt: true), .networkError),
        ClassifierCase("gateway failure right after wake",
                       Input(facts: ReducerFixture.facts(.connectedToGateway), withinWakeGrace: true), .networkError),

        // Counted failures after the prompt.
        ClassifierCase("OTP deadline",
                       Input(facts: ReducerFixture.facts(.otpPrompted, initiator: .helper(.otpTimeout)), appStop: .otpTimeout),
                       .authFailure(.otpNotReceived(inJunk: false))),
        ClassifierCase("OTP deadline, message in junk",
                       Input(facts: ReducerFixture.facts(.otpPrompted, initiator: .helper(.otpTimeout)), appStop: .otpTimeout, otpFoundInJunk: true),
                       .authFailure(.otpNotReceived(inJunk: true))),
        ClassifierCase("server login-timeout during the prompt",
                       Input(facts: ReducerFixture.facts(.otpPrompted)), .authFailure(.otpNotReceived(inJunk: false))),
        ClassifierCase("path change does not excuse failures after the prompt",
                       Input(facts: ReducerFixture.facts(.otpPrompted), pathChangedDuringAttempt: true, withinWakeGrace: true),
                       .authFailure(.otpNotReceived(inJunk: false))),
        ClassifierCase("OTP rejected",
                       Input(facts: ReducerFixture.facts(.otpSubmitted)), .authFailure(.otpRejected)),
        ClassifierCase("submit-to-auth watchdog",
                       Input(facts: ReducerFixture.facts(.otpSubmitted, initiator: .helper(.watchdog))),
                       .authFailure(.otpRejected)),
        ClassifierCase("reducer saw otpSubmitted, facts lag",
                       Input(facts: ReducerFixture.facts(.spawned), knownMilestone: .otpSubmitted), .authFailure(.otpRejected)),
        ClassifierCase("authenticated but no tunnel",
                       Input(facts: ReducerFixture.facts(.authenticated, initiator: .helper(.watchdog))),
                       .authFailure(.tunnelSetupFailed)),
        ClassifierCase("tunnel dropped after 30 s",
                       Input(facts: ReducerFixture.facts(.tunnelUp, up: 30)), .authFailure(.sessionDroppedEarly)),
        ClassifierCase("tunnel dropped after 119 s",
                       Input(facts: ReducerFixture.facts(.tunnelUp, up: 119.9)), .authFailure(.sessionDroppedEarly)),
        ClassifierCase("helper stopped a young tunnel on its own",
                       Input(facts: ReducerFixture.facts(.tunnelUp, initiator: .helper(.helperShutdown), up: 20)),
                       .authFailure(.sessionDroppedEarly)),
        ClassifierCase("cert rejected after OTP submission",
                       Input(facts: certFacts(.otpSubmitted, [digest], .otpSubmitted)), .authFailure(.certificateRejectedAfterLogin)),
        ClassifierCase("cert rejected after authentication (second TLS connection)",
                       Input(facts: certFacts(.authenticated, [digest], .authenticated), certRetryUsed: true),
                       .authFailure(.certificateRejectedAfterLogin)),
        ClassifierCase("gateway refused tunnel mode after login",
                       Input(facts: {
                           var facts = ReducerFixture.facts(.authenticated)
                           facts.tunnelModeRejected = true
                           return facts
                       }()),
                       .authFailure(.tunnelModeRejected)),
        ClassifierCase("our deadline stop raced tunnelUp",
                       Input(facts: ReducerFixture.facts(.tunnelUp, initiator: .helper(.attemptTimeout), up: 1), appStop: .attemptTimeout),
                       .networkError),

        // Gmail session failures are checked first.
        ClassifierCase("Gmail rejected the app password mid-attempt",
                       Input(facts: ReducerFixture.facts(.otpPrompted, initiator: .helper(.otpUnavailable)), appStop: .otpUnavailable, otpSessionFailure: .authentication),
                       .fatal(.gmailAppPassword)),
        ClassifierCase("Gmail connection dropped mid-attempt",
                       Input(facts: ReducerFixture.facts(.otpPrompted, initiator: .helper(.otpUnavailable)), appStop: .otpUnavailable, otpSessionFailure: .network),
                       .networkError),
    ]

    @Test(arguments: cases)
    func classify(_ testCase: ClassifierCase) {
        #expect(ExitClassifier.classify(testCase.input) == testCase.expected)
    }
}

private func certFacts(_ furthest: Milestone?, _ certs: [String], _ certAt: Milestone?) -> ExitFacts {
    ReducerFixture.facts(furthest, certs: certs, certAt: certAt)
}
