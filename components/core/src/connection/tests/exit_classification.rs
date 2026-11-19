//! Why a run ended without the app asking.

use fvm_contract::app_helper::{EndFacts, Milestone, StopReason};

use super::super::classify::{ExitContext, Outcome, classify};
use super::super::{Failure, OtpFailure};
use super::harness::*;

#[derive(Default)]
struct Case {
    seen: Option<Milestone>,
    cert_retry_used: bool,
    app_stop: Option<StopReason>,
    otp_failure: Option<OtpFailure>,
    otp_in_spam: bool,
}

fn outcome(facts: EndFacts, case: Case) -> Outcome {
    classify(&ExitContext {
        facts: &facts,
        seen_milestone: case.seen,
        cert_retry_used: case.cert_retry_used,
        app_stop: case.app_stop,
        otp_failure: case.otp_failure,
        otp_in_spam: case.otp_in_spam,
    })
}

fn plain(facts: EndFacts) -> Outcome {
    outcome(facts, Case::default())
}

fn cert(furthest: Milestone, certs: &[&str], at: Option<Milestone>) -> EndFacts {
    let certs: Vec<String> = certs.iter().map(|cert| cert.to_string()).collect();
    ended(furthest).certs(&certs, at).facts()
}

fn auth(failure: Failure) -> Outcome {
    Outcome::AuthFailure(failure)
}

const BEFORE_THE_TUNNEL: [Milestone; 5] = [
    Milestone::Started,
    Milestone::ConnectedToGateway,
    Milestone::OtpPrompted,
    Milestone::OtpSubmitted,
    Milestone::Authenticated,
];

#[test]
fn certificate_rejection_before_the_otp_was_submitted() {
    let a = digest('a');
    let a = a.as_str();
    assert_eq!(
        plain(cert(Milestone::Started, &[a], Some(Milestone::Started))),
        Outcome::CertificateRetry
    );
    assert_eq!(
        plain(cert(Milestone::Started, &[a, a], Some(Milestone::Started))),
        Outcome::CertificateRetry
    );
    assert_eq!(
        outcome(
            cert(Milestone::Started, &[a], Some(Milestone::Started)),
            Case {
                cert_retry_used: true,
                ..Case::default()
            }
        ),
        Outcome::NetworkError,
        "the chain already retried"
    );
    assert_eq!(
        plain(cert(Milestone::ConnectedToGateway, &[a], None)),
        Outcome::CertificateRetry,
        "no rejection milestone"
    );
    assert_eq!(
        plain(cert(
            Milestone::OtpPrompted,
            &[a],
            Some(Milestone::OtpPrompted)
        )),
        Outcome::CertificateRetry
    );
}

#[test]
fn the_server_ending_or_rejecting_the_session_after_the_tunnel() {
    for code in [-EPIPE, -EPERM] {
        assert_eq!(
            plain(ended(Milestone::TunnelUp).code(code).facts()),
            Outcome::ServerSessionEnded,
            "{code}"
        );
        assert_eq!(
            outcome(
                ended(Milestone::Authenticated).code(code).facts(),
                Case {
                    seen: Some(Milestone::TunnelUp),
                    ..Case::default()
                }
            ),
            Outcome::ServerSessionEnded,
            "the facts lag behind: {code}"
        );
    }
}

#[test]
fn any_other_end_after_the_tunnel_is_a_network_error() {
    for code in [-EIO, -EINTR] {
        assert_eq!(
            plain(ended(Milestone::TunnelUp).code(code).facts()),
            Outcome::NetworkError,
            "{code}"
        );
    }
    assert_eq!(
        plain(
            ended(Milestone::TunnelUp)
                .by_helper(StopReason::HelperShutdown)
                .code(-EINTR)
                .facts()
        ),
        Outcome::NetworkError,
        "stopped by the Helper itself"
    );
    assert_eq!(
        plain(
            ended(Milestone::TunnelUp)
                .by_helper(StopReason::StageLimit)
                .code(-EPIPE)
                .facts()
        ),
        Outcome::ServerSessionEnded,
        "the code decides, whoever stopped the run"
    );
    assert_eq!(
        outcome(
            ended(Milestone::TunnelUp)
                .by_helper(StopReason::AttemptTimeout)
                .code(-EPIPE)
                .facts(),
            Case {
                app_stop: Some(StopReason::AttemptTimeout),
                ..Case::default()
            }
        ),
        Outcome::NetworkError,
        "our deadline stop raced the tunnel"
    );
    let a = digest('a');
    assert_eq!(
        plain(cert(
            Milestone::TunnelUp,
            &[a.as_str()],
            Some(Milestone::TunnelUp)
        )),
        Outcome::NetworkError,
        "a certificate change on the tunnel's reconnect"
    );
}

/// The Helper ends a run whose client asks again for what it was given.
#[test]
fn a_repeated_prompt_is_a_rejected_login_or_otp() {
    assert_eq!(
        plain(
            ended(Milestone::ConnectedToGateway)
                .by_helper(StopReason::PromptRepeated)
                .facts()
        ),
        auth(Failure::AuthenticationRejected)
    );
    for furthest in [Milestone::OtpSubmitted, Milestone::Authenticated] {
        assert_eq!(
            plain(
                ended(furthest)
                    .by_helper(StopReason::PromptRepeated)
                    .facts()
            ),
            auth(Failure::OtpRejected),
            "{furthest:?}"
        );
    }
    assert_eq!(
        outcome(
            ended(Milestone::ConnectedToGateway)
                .by_helper(StopReason::PromptRepeated)
                .facts(),
            Case {
                seen: Some(Milestone::OtpSubmitted),
                ..Case::default()
            }
        ),
        auth(Failure::OtpRejected),
        "the facts lag behind"
    );
}

#[test]
fn a_certificate_rejected_after_the_login_is_counted() {
    let a = digest('a');
    assert_eq!(
        plain(cert(
            Milestone::OtpSubmitted,
            &[a.as_str()],
            Some(Milestone::OtpSubmitted)
        )),
        auth(Failure::CertificateRejectedAfterLogin)
    );
    assert_eq!(
        outcome(
            cert(
                Milestone::Authenticated,
                &[a.as_str()],
                Some(Milestone::Authenticated)
            ),
            Case {
                cert_retry_used: true,
                ..Case::default()
            }
        ),
        auth(Failure::CertificateRejectedAfterLogin)
    );
}

#[test]
fn a_tunnel_device_that_could_not_be_set_up_is_counted() {
    assert_eq!(
        plain(
            ended(Milestone::Authenticated)
                .tunnel_setup_failed()
                .code(-EINTR)
                .facts()
        ),
        auth(Failure::TunnelSetupFailed)
    );
}

#[test]
fn an_otp_not_received_by_the_apps_deadline_is_counted() {
    assert_eq!(
        outcome(
            ended(Milestone::OtpPrompted)
                .by_helper(StopReason::OtpTimeout)
                .code(-EINTR)
                .facts(),
            Case {
                app_stop: Some(StopReason::OtpTimeout),
                ..Case::default()
            }
        ),
        auth(Failure::OtpNotReceived { in_spam: false })
    );
    assert_eq!(
        outcome(
            ended(Milestone::OtpPrompted)
                .by_helper(StopReason::OtpTimeout)
                .code(-EINTR)
                .facts(),
            Case {
                app_stop: Some(StopReason::OtpTimeout),
                otp_in_spam: true,
                ..Case::default()
            }
        ),
        auth(Failure::OtpNotReceived { in_spam: true })
    );
}

#[test]
fn a_network_setup_the_helper_gave_up_on_is_its_own_uncounted_outcome() {
    for furthest in [Milestone::Authenticated, Milestone::TunnelUp] {
        for code in [-EINTR, -EPIPE] {
            let facts = ended(furthest)
                .by_helper(StopReason::NetworkSetupFailed)
                .code(code)
                .facts();
            assert_eq!(plain(facts), Outcome::NetworkSetupFailed, "{furthest:?}");
        }
    }
}

#[test]
fn any_other_end_before_the_tunnel_is_a_network_error() {
    for furthest in BEFORE_THE_TUNNEL {
        for code in [-EIO, -EINTR, -EPERM, -EPIPE] {
            assert_eq!(
                plain(ended(furthest).code(code).facts()),
                Outcome::NetworkError,
                "{furthest:?} {code}"
            );
        }
        assert_eq!(
            plain(ended(furthest).by_helper(StopReason::StageLimit).facts()),
            Outcome::NetworkError,
            "{furthest:?}"
        );
        assert_eq!(
            outcome(
                ended(furthest)
                    .by_helper(StopReason::AttemptTimeout)
                    .code(-EINTR)
                    .facts(),
                Case {
                    app_stop: Some(StopReason::AttemptTimeout),
                    ..Case::default()
                }
            ),
            Outcome::NetworkError,
            "{furthest:?}"
        );
    }
    assert_eq!(
        outcome(
            ended(Milestone::Started).facts(),
            Case {
                seen: Some(Milestone::OtpSubmitted),
                ..Case::default()
            }
        ),
        Outcome::NetworkError,
        "the facts lag behind"
    );
}

#[test]
fn gmail_session_failures_come_first() {
    let facts = || {
        ended(Milestone::OtpPrompted)
            .by_helper(StopReason::OtpUnavailable)
            .facts()
    };
    assert_eq!(
        outcome(
            facts(),
            Case {
                app_stop: Some(StopReason::OtpUnavailable),
                otp_failure: Some(OtpFailure::Authentication),
                ..Case::default()
            }
        ),
        Outcome::Fatal(Failure::GmailRejected)
    );
    assert_eq!(
        outcome(
            facts(),
            Case {
                app_stop: Some(StopReason::OtpUnavailable),
                otp_failure: Some(OtpFailure::Network),
                ..Case::default()
            }
        ),
        Outcome::NetworkError
    );
}
