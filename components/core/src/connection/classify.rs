//! Why a run ended without the app asking for it. The VPN client's exit code and
//! messages cannot tell, so the Helper's facts decide: how far the run got, who stopped it, how
//! long the tunnel stayed up.

use std::time::Duration;

use fvm_contract::app_helper::{
    ExitFacts, Milestone, ProcessExitStatus, StopInitiator, StopReason,
};

use super::OtpFailure;
use super::ledger::Failure;
use super::policy;

pub(super) struct ExitContext<'a> {
    pub(super) facts: &'a ExitFacts,
    /// The furthest milestone the manager saw; the facts may lag behind it.
    pub(super) seen_milestone: Option<Milestone>,
    /// The chain already used its one immediate retry after a certificate rejection.
    pub(super) cert_retry_used: bool,
    /// The stop the app asked for, if any.
    pub(super) app_stop: Option<StopReason>,
    pub(super) otp_failure: Option<OtpFailure>,
    pub(super) otp_in_spam: bool,
    pub(super) path_changed: bool,
    /// The attempt started within the grace period after a wake.
    pub(super) within_wake_grace: bool,
    /// Measured by the manager, for when the Helper reported no up time.
    pub(super) observed_up: Option<Duration>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Outcome {
    /// The gateway certificate was learned before the OTP was used: retry at once, uncounted.
    CertificateRetry,
    /// The server ended a stable session: reconnect shortly.
    ServerSessionEnded,
    /// Killed by someone else after the tunnel came up: reconnect at once, uncounted.
    ForcedTermination,
    /// Uncounted; back off.
    NetworkError,
    /// Counted.
    AuthFailure(Failure),
    /// Stop without retrying.
    Fatal(Failure),
}

/// The first rule that matches, in order.
pub(super) fn classify(context: &ExitContext) -> Outcome {
    let facts = context.facts;
    let furthest = facts.furthest_milestone.max(context.seen_milestone);

    // The app stopped the run because the Gmail session failed and no OTP could arrive.
    if let Some(failure) = context.otp_failure {
        return match failure {
            OtpFailure::Authentication => Outcome::Fatal(Failure::GmailRejected),
            OtpFailure::Network => Outcome::NetworkError,
        };
    }

    let rejected = !facts.cert_rejected_digests.is_empty();
    let rejected_at = facts
        .cert_rejected_at
        .or(furthest)
        .unwrap_or(Milestone::Spawned);
    if rejected && rejected_at < Milestone::OtpSubmitted {
        return if context.cert_retry_used {
            Outcome::NetworkError
        } else {
            Outcome::CertificateRetry
        };
    }

    let up = facts
        .up_duration
        .or(context.observed_up)
        .unwrap_or(Duration::ZERO);
    let unrequested =
        context.app_stop.is_none() && facts.stop_initiator == StopInitiator::Client {};
    if furthest == Some(Milestone::TunnelUp) && unrequested {
        // A signal nobody here sent is a kill, not the server or the account, whatever the up
        // time. Only the client's own death shows: when only its helper process is killed, the
        // client exits normally, like a server end.
        if matches!(facts.exit_status, ProcessExitStatus::Signaled { .. }) {
            return Outcome::ForcedTermination;
        }
        if up >= policy::STABLE_SESSION {
            return Outcome::ServerSessionEnded;
        }
    }

    let Some(furthest) = furthest.filter(|&milestone| milestone >= Milestone::ConnectedToGateway)
    else {
        return Outcome::NetworkError;
    };

    // Ended at login: counted although it may be the network, as FortiGate blocks the IP after
    // two failures.
    if furthest == Milestone::ConnectedToGateway {
        return if context.path_changed || context.within_wake_grace {
            Outcome::NetworkError
        } else {
            Outcome::AuthFailure(Failure::AuthenticationRejected)
        };
    }

    if rejected {
        return Outcome::AuthFailure(Failure::CertificateRejectedAfterLogin);
    }
    if facts.tunnel_mode_rejected {
        return Outcome::AuthFailure(Failure::TunnelModeRejected);
    }
    match furthest {
        Milestone::OtpPrompted => Outcome::AuthFailure(Failure::OtpNotReceived {
            in_spam: context.otp_in_spam,
        }),
        Milestone::OtpSubmitted => Outcome::AuthFailure(Failure::OtpRejected),
        Milestone::Authenticated => Outcome::AuthFailure(Failure::TunnelSetupFailed),
        // The app's own deadline stop raced the tunnel coming up.
        Milestone::TunnelUp if context.app_stop.is_some() => Outcome::NetworkError,
        Milestone::TunnelUp if up < policy::STABLE_SESSION => {
            Outcome::AuthFailure(Failure::SessionDroppedEarly)
        }
        // A stable session the Helper ended on its own, such as on its shutdown.
        Milestone::TunnelUp => Outcome::NetworkError,
        Milestone::Spawned | Milestone::ConnectedToGateway => {
            unreachable!("filtered above")
        }
    }
}
