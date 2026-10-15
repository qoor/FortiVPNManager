import Foundation
import FortiShared

/// Classifies how an unrequested (or deadline-stopped) run ended, following the §6.2 table top to bottom.
/// Runs the app stopped on purpose (user disconnect, proactive reconnect, sleep, quit) are not classified.
enum ExitClassifier {
    struct Input: Equatable {
        var facts: ExitFacts
        /// Furthest milestone the reducer itself saw; combined with `facts.furthestMilestone`.
        var knownMilestone: Milestone?
        var certRetryUsed: Bool
        /// Stop the app requested for this attempt (`otpTimeout` / `attemptTimeout`), if any.
        var appStop: StopReason?
        var otpSessionFailure: OTPSessionFailure?
        var otpFoundInJunk: Bool
        var pathChangedDuringAttempt: Bool
        /// The attempt started before `lastWakeAt + 30 s`.
        var withinWakeGrace: Bool
        /// Up-time measured by the app, used when the helper did not report one.
        var observedUpDuration: TimeInterval?

        init(
            facts: ExitFacts,
            knownMilestone: Milestone? = nil,
            certRetryUsed: Bool = false,
            appStop: StopReason? = nil,
            otpSessionFailure: OTPSessionFailure? = nil,
            otpFoundInJunk: Bool = false,
            pathChangedDuringAttempt: Bool = false,
            withinWakeGrace: Bool = false,
            observedUpDuration: TimeInterval? = nil
        ) {
            self.facts = facts
            self.knownMilestone = knownMilestone
            self.certRetryUsed = certRetryUsed
            self.appStop = appStop
            self.otpSessionFailure = otpSessionFailure
            self.otpFoundInJunk = otpFoundInJunk
            self.pathChangedDuringAttempt = pathChangedDuringAttempt
            self.withinWakeGrace = withinWakeGrace
            self.observedUpDuration = observedUpDuration
        }

        var furthestMilestone: Milestone? {
            switch (facts.furthestMilestone, knownMilestone) {
            case let (lhs?, rhs?): max(lhs, rhs)
            case let (lhs, rhs): lhs ?? rhs
            }
        }

        var upDuration: TimeInterval {
            facts.upDuration ?? observedUpDuration ?? 0
        }
    }

    enum Outcome: Equatable {
        /// Learned the gateway certificate before any credentials were used: retry immediately, uncounted.
        case certificateRetry
        /// The server ended a stable session: reconnect after 5 s and reset the counter.
        case serverSessionEnded
        /// Not counted; back off and wait for the network.
        case networkError
        /// Counted authentication failure.
        case authFailure(ConnectionFailureReason)
        /// Give up without retrying.
        case fatal(ConnectionFailureReason)
    }

    static func classify(_ input: Input) -> Outcome {
        let facts = input.facts
        let furthest = input.furthestMilestone

        // The Gmail session failed and the app stopped the run because no OTP could arrive.
        if let failure = input.otpSessionFailure {
            return failure == .authentication ? .fatal(.gmailAppPassword) : .networkError
        }

        // Row 1: certificate rejected before the OTP was used.
        let rejected = !facts.certRejectedDigests.isEmpty
        if rejected, (facts.certRejectedAt ?? furthest ?? .spawned) < .otpSubmitted {
            let learnable = !TrustedCertPins.normalized(facts.certRejectedDigests).isEmpty
            return (input.certRetryUsed || !learnable) ? .networkError : .certificateRetry
        }

        // Row 2: a stable tunnel ended without anyone on our side asking for it.
        if furthest == .tunnelUp, input.appStop == nil, facts.stopInitiator == .none,
           input.upDuration >= RetryPolicy.stableSessionDuration {
            return .serverSessionEnded
        }

        // Row 3 (proactive timer) never reaches the classifier: it is a requested stop.

        // Row 4: never reached the gateway.
        guard let furthest, furthest >= .connectedToGateway else { return .networkError }

        // Row 5: reached the gateway, no OTP prompt.
        if furthest == .connectedToGateway {
            return (input.pathChangedDuringAttempt || input.withinWakeGrace)
                ? .networkError
                : .authFailure(.authenticationRejected)
        }

        // Row 6: counted failures after the OTP prompt.
        if rejected { return .authFailure(.certificateRejectedAfterLogin) }
        if facts.tunnelModeRejected { return .authFailure(.tunnelModeRejected) }
        switch furthest {
        case .otpPrompted:
            return .authFailure(.otpNotReceived(inJunk: input.otpFoundInJunk))
        case .otpSubmitted:
            return .authFailure(.otpRejected)
        case .authenticated:
            return .authFailure(.tunnelSetupFailed)
        case .tunnelUp:
            // Our own deadline stop raced the tunnel coming up: not the server's or the account's fault.
            if input.appStop != nil { return .networkError }
            if input.upDuration < RetryPolicy.stableSessionDuration { return .authFailure(.sessionDroppedEarly) }
            // Stable session ended by the helper on its own (e.g. helper shutdown).
            return .networkError
        case .spawned, .connectedToGateway:
            return .networkError
        }
    }
}
