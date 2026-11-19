//! Why a run ended without the app asking for it, judged only from the facts the Helper and the app
//! observed: how far the run got, who stopped it and why, and the code openconnect ended with.

use fvm_contract::app_helper::{EndFacts, Milestone, StopInitiator, StopReason};

use super::OtpFailure;
use super::ledger::Failure;

const EPERM: i32 = 1;
const EPIPE: i32 = 32;

pub(super) struct ExitContext<'a> {
    pub(super) facts: &'a EndFacts,
    /// The furthest milestone the manager saw; the facts may lag behind it.
    pub(super) seen_milestone: Option<Milestone>,
    /// The chain already used its one immediate retry after a certificate rejection.
    pub(super) cert_retry_used: bool,
    /// The stop the app asked for, if any.
    pub(super) app_stop: Option<StopReason>,
    pub(super) otp_failure: Option<OtpFailure>,
    pub(super) otp_in_spam: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Outcome {
    /// The gateway certificate was learned before the OTP was used: retry at once, uncounted.
    CertificateRetry,
    /// The server ended or rejected the session: reconnect shortly.
    ServerSessionEnded,
    /// Uncounted; back off.
    NetworkError,
    NetworkSetupFailed,
    /// Counted.
    AuthFailure(Failure),
    /// Stop without retrying.
    Fatal(Failure),
}

/// The first rule that matches, in order.
pub(super) fn classify(context: &ExitContext) -> Outcome {
    let facts = context.facts;
    let furthest = facts
        .furthest_milestone
        .max(context.seen_milestone.unwrap_or(Milestone::Started));

    // The app stopped the run because the Gmail session failed and no OTP could arrive.
    if let Some(failure) = context.otp_failure {
        return match failure {
            OtpFailure::Authentication => Outcome::Fatal(Failure::GmailRejected),
            OtpFailure::Network => Outcome::NetworkError,
        };
    }

    if facts.stop_initiator
        == (StopInitiator::Helper {
            reason: StopReason::NetworkSetupFailed,
        })
    {
        return Outcome::NetworkSetupFailed;
    }

    let rejected = !facts.cert_rejected_digests.is_empty();
    let rejected_at = facts.cert_rejected_at.unwrap_or(furthest);
    if rejected && rejected_at < Milestone::OtpSubmitted {
        return if context.cert_retry_used {
            Outcome::NetworkError
        } else {
            Outcome::CertificateRetry
        };
    }

    if furthest == Milestone::TunnelUp {
        return match context.app_stop {
            // The app's own deadline stop raced the tunnel coming up.
            Some(_) => Outcome::NetworkError,
            None if [-EPIPE, -EPERM].contains(&facts.end_code) => Outcome::ServerSessionEnded,
            None => Outcome::NetworkError,
        };
    }

    if facts.stop_initiator
        == (StopInitiator::Helper {
            reason: StopReason::PromptRepeated,
        })
    {
        return Outcome::AuthFailure(if furthest < Milestone::OtpPrompted {
            Failure::AuthenticationRejected
        } else {
            Failure::OtpRejected
        });
    }
    if rejected {
        return Outcome::AuthFailure(Failure::CertificateRejectedAfterLogin);
    }
    if facts.tunnel_setup_failed {
        return Outcome::AuthFailure(Failure::TunnelSetupFailed);
    }
    if context.app_stop == Some(StopReason::OtpTimeout) {
        return Outcome::AuthFailure(Failure::OtpNotReceived {
            in_spam: context.otp_in_spam,
        });
    }
    Outcome::NetworkError
}
