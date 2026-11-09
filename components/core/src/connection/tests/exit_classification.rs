//! Why a run ended without the app asking (SPEC F-CM-03).

use std::time::Duration;

use fvm_contract::app_helper::{ExitFacts, Milestone, ProcessExitStatus, StopReason};

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
    path_changed: bool,
    within_wake_grace: bool,
    observed_up: Option<f64>,
}

fn outcome(facts: ExitFacts, case: Case) -> Outcome {
    classify(&ExitContext {
        facts: &facts,
        seen_milestone: case.seen,
        cert_retry_used: case.cert_retry_used,
        app_stop: case.app_stop,
        otp_failure: case.otp_failure,
        otp_in_spam: case.otp_in_spam,
        path_changed: case.path_changed,
        within_wake_grace: case.within_wake_grace,
        observed_up: case.observed_up.map(Duration::from_secs_f64),
    })
}

fn plain(facts: ExitFacts) -> Outcome {
    outcome(facts, Case::default())
}

fn cert(furthest: Milestone, certs: &[&str], at: Option<Milestone>) -> ExitFacts {
    let certs: Vec<String> = certs.iter().map(|cert| cert.to_string()).collect();
    ended(furthest).certs(&certs, at).facts()
}

fn auth(failure: Failure) -> Outcome {
    Outcome::AuthFailure(failure)
}

#[test]
fn certificate_rejection_before_the_otp_was_submitted() {
    let a = digest('a');
    let a = a.as_str();
    assert_eq!(
        plain(cert(Milestone::Spawned, &[a], Some(Milestone::Spawned))),
        Outcome::CertificateRetry
    );
    assert_eq!(
        plain(cert(Milestone::Spawned, &[a, a], Some(Milestone::Spawned))),
        Outcome::CertificateRetry
    );
    assert_eq!(
        outcome(
            cert(Milestone::Spawned, &[a], Some(Milestone::Spawned)),
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
    // A rejection without a usable value cannot arrive: the Helper reports only values the client
    // accepts (backend adapter output).
    assert_eq!(
        outcome(
            cert(
                Milestone::ConnectedToGateway,
                &[a],
                Some(Milestone::ConnectedToGateway)
            ),
            Case {
                path_changed: true,
                ..Case::default()
            }
        ),
        Outcome::CertificateRetry,
        "beats the path-change rule"
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
fn a_stable_tunnel_ended_by_the_server() {
    assert_eq!(
        plain(ended(Milestone::TunnelUp).up(10_800.0).facts()),
        Outcome::ServerSessionEnded
    );
    assert_eq!(
        plain(ended(Milestone::TunnelUp).up(120.0).facts()),
        Outcome::ServerSessionEnded
    );
    assert_eq!(
        outcome(
            ended(Milestone::TunnelUp).facts(),
            Case {
                observed_up: Some(600.0),
                ..Case::default()
            }
        ),
        Outcome::ServerSessionEnded,
        "the Helper did not report the up-time; the app measured it"
    );
    assert_eq!(
        outcome(
            ended(Milestone::Authenticated).up(300.0).facts(),
            Case {
                seen: Some(Milestone::TunnelUp),
                ..Case::default()
            }
        ),
        Outcome::ServerSessionEnded,
        "the facts lag behind"
    );
    assert_eq!(
        plain(
            ended(Milestone::TunnelUp)
                .by_helper(StopReason::HelperShutdown)
                .up(5_000.0)
                .facts()
        ),
        Outcome::NetworkError,
        "stopped by the Helper itself"
    );
}

#[test]
fn killed_after_the_tunnel_came_up() {
    assert_eq!(
        plain(killed(Milestone::TunnelUp).up(180.0).facts()),
        Outcome::ForcedTermination
    );
    assert_eq!(
        plain(killed(Milestone::TunnelUp).up(30.0).facts()),
        Outcome::ForcedTermination,
        "not an early drop"
    );
    assert_eq!(
        outcome(
            killed(Milestone::TunnelUp).facts(),
            Case {
                observed_up: Some(5.0),
                ..Case::default()
            }
        ),
        Outcome::ForcedTermination
    );
    assert_eq!(
        outcome(
            killed(Milestone::Authenticated).up(2.0).facts(),
            Case {
                seen: Some(Milestone::TunnelUp),
                ..Case::default()
            }
        ),
        Outcome::ForcedTermination
    );
    assert_eq!(
        outcome(
            killed(Milestone::TunnelUp).up(1.0).facts(),
            Case {
                app_stop: Some(StopReason::AttemptTimeout),
                ..Case::default()
            }
        ),
        Outcome::NetworkError,
        "our deadline stop raced the tunnel"
    );
    let signaled = |signal| ProcessExitStatus::Signaled { signal };
    assert_eq!(
        plain(
            ended(Milestone::TunnelUp)
                .by_helper(StopReason::HelperShutdown)
                .up(20.0)
                .status(signaled(15))
                .facts()
        ),
        auth(Failure::SessionDroppedEarly)
    );
    assert_eq!(
        plain(
            ended(Milestone::TunnelUp)
                .by_helper(StopReason::HelperShutdown)
                .up(5_000.0)
                .status(signaled(9))
                .facts()
        ),
        Outcome::NetworkError
    );
}

#[test]
fn killed_before_the_tunnel_came_up_keeps_the_stage_rules() {
    assert_eq!(
        plain(killed(Milestone::Spawned).facts()),
        Outcome::NetworkError
    );
    assert_eq!(
        plain(killed(Milestone::ConnectedToGateway).facts()),
        auth(Failure::AuthenticationRejected)
    );
    assert_eq!(
        plain(killed(Milestone::OtpPrompted).facts()),
        auth(Failure::OtpNotReceived { in_spam: false })
    );
    assert_eq!(
        plain(killed(Milestone::Authenticated).facts()),
        auth(Failure::TunnelSetupFailed)
    );
}

#[test]
fn never_reached_the_gateway() {
    assert_eq!(plain(spawn_failed().facts()), Outcome::NetworkError);
    assert_eq!(
        plain(ended(Milestone::Spawned).facts()),
        Outcome::NetworkError
    );
    assert_eq!(
        plain(
            ended(Milestone::Spawned)
                .by_helper(StopReason::Watchdog)
                .facts()
        ),
        Outcome::NetworkError
    );
    assert_eq!(
        outcome(
            ended(Milestone::Spawned)
                .by_helper(StopReason::AttemptTimeout)
                .facts(),
            Case {
                app_stop: Some(StopReason::AttemptTimeout),
                ..Case::default()
            }
        ),
        Outcome::NetworkError
    );
}

#[test]
fn ended_at_the_gateway() {
    assert_eq!(
        plain(ended(Milestone::ConnectedToGateway).facts()),
        auth(Failure::AuthenticationRejected)
    );
    assert_eq!(
        outcome(
            ended(Milestone::ConnectedToGateway).facts(),
            Case {
                path_changed: true,
                ..Case::default()
            }
        ),
        Outcome::NetworkError
    );
    assert_eq!(
        outcome(
            ended(Milestone::ConnectedToGateway).facts(),
            Case {
                within_wake_grace: true,
                ..Case::default()
            }
        ),
        Outcome::NetworkError
    );
}

#[test]
fn counted_failures_after_the_prompt() {
    let otp_stop = Case {
        app_stop: Some(StopReason::OtpTimeout),
        ..Case::default()
    };
    assert_eq!(
        outcome(
            ended(Milestone::OtpPrompted)
                .by_helper(StopReason::OtpTimeout)
                .facts(),
            otp_stop
        ),
        auth(Failure::OtpNotReceived { in_spam: false })
    );
    assert_eq!(
        outcome(
            ended(Milestone::OtpPrompted)
                .by_helper(StopReason::OtpTimeout)
                .facts(),
            Case {
                app_stop: Some(StopReason::OtpTimeout),
                otp_in_spam: true,
                ..Case::default()
            }
        ),
        auth(Failure::OtpNotReceived { in_spam: true })
    );
    assert_eq!(
        plain(ended(Milestone::OtpPrompted).facts()),
        auth(Failure::OtpNotReceived { in_spam: false })
    );
    assert_eq!(
        outcome(
            ended(Milestone::OtpPrompted).facts(),
            Case {
                path_changed: true,
                within_wake_grace: true,
                ..Case::default()
            }
        ),
        auth(Failure::OtpNotReceived { in_spam: false }),
        "a path change does not excuse failures after the prompt"
    );
    assert_eq!(
        plain(ended(Milestone::OtpSubmitted).facts()),
        auth(Failure::OtpRejected)
    );
    assert_eq!(
        plain(
            ended(Milestone::OtpSubmitted)
                .by_helper(StopReason::Watchdog)
                .facts()
        ),
        auth(Failure::OtpRejected)
    );
    assert_eq!(
        outcome(
            ended(Milestone::Spawned).facts(),
            Case {
                seen: Some(Milestone::OtpSubmitted),
                ..Case::default()
            }
        ),
        auth(Failure::OtpRejected)
    );
    assert_eq!(
        plain(
            ended(Milestone::Authenticated)
                .by_helper(StopReason::Watchdog)
                .facts()
        ),
        auth(Failure::TunnelSetupFailed)
    );
    assert_eq!(
        plain(ended(Milestone::TunnelUp).up(30.0).facts()),
        auth(Failure::SessionDroppedEarly)
    );
    assert_eq!(
        plain(ended(Milestone::TunnelUp).up(119.9).facts()),
        auth(Failure::SessionDroppedEarly)
    );
    assert_eq!(
        plain(
            ended(Milestone::TunnelUp)
                .by_helper(StopReason::HelperShutdown)
                .up(20.0)
                .facts()
        ),
        auth(Failure::SessionDroppedEarly)
    );
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
    assert_eq!(
        plain(
            ended(Milestone::Authenticated)
                .tunnel_mode_rejected()
                .facts()
        ),
        auth(Failure::TunnelModeRejected)
    );
    assert_eq!(
        outcome(
            ended(Milestone::TunnelUp)
                .by_helper(StopReason::AttemptTimeout)
                .up(1.0)
                .facts(),
            Case {
                app_stop: Some(StopReason::AttemptTimeout),
                ..Case::default()
            }
        ),
        Outcome::NetworkError
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
