mod common;

use std::sync::{Arc, Mutex, mpsc};

use common::*;
use fvm_contract::app_helper::{
    EndFacts, HelperEventKind, Milestone, OutputSource, StopInitiator, StopReason,
};
use fvm_contract::helper_internal::NetworkOutcome;
use fvm_helper_core::session::backend::SessionEvent;

const EPIPE: i32 = 32;

fn session_end(furthest: Milestone, code: i32) -> EndFacts {
    EndFacts {
        furthest_milestone: furthest,
        stop_initiator: StopInitiator::Session {},
        cert_rejected_digests: Vec::new(),
        cert_rejected_at: None,
        tunnel_setup_failed: false,
        tunnel_address_in_use: false,
        end_code: code,
    }
}

#[test]
fn a_session_goes_through_every_milestone_and_ends_on_the_users_stop() {
    let received = Arc::new(Mutex::new(None));
    let script = {
        let received = Arc::clone(&received);
        script(move |request, link| {
            trust_gateway(link);
            let otp = ask_otp(link).unwrap();
            *received.lock().unwrap() = Some((
                request.username.clone(),
                request.password.to_string(),
                otp.to_string(),
                request.pins.clone(),
            ));
            open(link);
            error_line(
                link,
                "DTLS handshake failed: Resource temporarily unavailable",
            );
            tunnel_ready(link, settings("utun9", "10.10.0.5"));
            wait_for_cancel(link);
            line(link, "Logged out");
            ended(-EINTR)
        })
    };
    let mut session = Session::start(script, Setup::default().submitting_otp());
    session.bring_up(vec![gateway_route("192.0.2.1")]);
    assert!(session.run.session_running());
    let facts = session.stop_and_revert(StopReason::UserDisconnect);

    assert_eq!(session.milestones(), FULL_FLOW);
    for kind in session.events() {
        if let HelperEventKind::Milestone {
            milestone,
            tunnel_ip,
        } = kind
        {
            let expected =
                (*milestone == Milestone::TunnelUp).then(|| "10.10.0.5".parse().unwrap());
            assert_eq!(*tunnel_ip, expected, "{milestone:?}");
        }
    }
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(facts.furthest_milestone, Milestone::TunnelUp);
    assert_eq!(facts.end_code, -EINTR);
    assert_eq!(
        *received.lock().unwrap(),
        Some((
            "alice".to_owned(),
            PASSWORD.to_owned(),
            OTP.to_owned(),
            vec![PIN.to_owned()]
        ))
    );

    let texts = session.texts();
    assert_eq!(texts[0], "‹VPN client: openconnect 9.21›");
    let order = [
        "Connected to HTTPS on 203.0.113.10 with ciphersuite (TLS1.3)",
        "‹OTP sent›",
        "Connected as 10.10.0.5, using SSL",
        "‹network settings applied›",
        "‹session cancelled (reason: userDisconnect)›",
        "Logged out",
        "‹session ended: code -4›",
        "‹network settings reverted›",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|text| {
            texts
                .iter()
                .position(|shown| shown == text)
                .unwrap_or_else(|| panic!("{text} missing from {texts:?}"))
        })
        .collect();
    assert!(positions.is_sorted(), "{texts:?}");
    assert!(session.has_line(OutputSource::Info, "Logged out"));
    assert!(session.has_line(
        OutputSource::Error,
        "DTLS handshake failed: Resource temporarily unavailable"
    ));
    assert!(session.has_line(OutputSource::Helper, "‹OTP sent›"));
    let before = |milestone: Milestone| {
        session.texts_before(|kind| {
            matches!(kind, HelperEventKind::Milestone { milestone: reached, .. } if *reached == milestone)
        })
    };
    assert_eq!(
        before(Milestone::ConnectedToGateway).last(),
        Some(&"Connected to HTTPS on 203.0.113.10 with ciphersuite (TLS1.3)")
    );
    assert_eq!(
        before(Milestone::TunnelUp).last(),
        Some(&"‹network settings applied›")
    );
    session.assert_finished_cleanly();
}

#[test]
fn a_rejected_certificate_is_told_in_the_exit_once() {
    let script = script(|_, link| {
        link.report(SessionEvent::CertificateRejected(OTHER_PIN.to_owned()));
        link.report(SessionEvent::CertificateRejected(OTHER_PIN.to_owned()));
        error_line(link, "SSL connection failure");
        ended(-EIO)
    });
    let mut session = Session::start(script, Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(session.milestones(), [Milestone::Started]);
    assert_eq!(
        facts,
        EndFacts {
            cert_rejected_digests: vec![OTHER_PIN.to_owned()],
            cert_rejected_at: Some(Milestone::Started),
            ..session_end(Milestone::Started, -EIO)
        }
    );
    assert_eq!(
        session.count_text(&format!("‹gateway certificate not trusted: {OTHER_PIN}›")),
        2
    );
    session.assert_finished_cleanly();
}

#[test]
fn a_certificate_rejected_after_the_gateway_was_trusted_is_told_where_it_came() {
    let script = script(|_, link| {
        trust_gateway(link);
        link.report(SessionEvent::CertificateRejected(OTHER_PIN.to_owned()));
        ended(-EIO)
    });
    let mut session = Session::start(script, Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(facts.cert_rejected_at, Some(Milestone::ConnectedToGateway));
    assert_eq!(facts.furthest_milestone, Milestone::ConnectedToGateway);
}

#[test]
fn a_session_that_fails_before_the_first_form_ends_at_started() {
    let script = script(|_, link| {
        error_line(
            link,
            "SSL connection failure: The TLS connection was non-properly terminated.",
        );
        ended(-EIO)
    });
    let mut session = Session::start(script, Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(facts, session_end(Milestone::Started, -EIO));
    assert_eq!(session.texts().last(), Some(&"‹session ended: code -5›"));
    session.assert_finished_cleanly();
    let error = session.run.submit_otp(OTP.to_owned()).unwrap_err();
    assert_eq!(error.message, "openconnect is not running");
}

#[test]
fn a_tunnel_that_cannot_be_set_up_ends_after_authentication() {
    let script = script(|_, link| {
        trust_gateway(link);
        open(link);
        link.report(SessionEvent::TunnelFailed(
            Some("10.10.0.5".parse().unwrap()),
            Some("Operation not permitted (os error 1)".to_owned()),
        ));
        ended(-EIO)
    });
    let mut session = Session::start(script, Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(
        facts,
        EndFacts {
            tunnel_setup_failed: true,
            ..session_end(Milestone::Authenticated, -EIO)
        }
    );
    assert!(
        session.has_text(
            "‹the tunnel device could not be set up: Operation not permitted (os error 1)›"
        )
    );
    assert!(session.jobs.is_empty());
}

#[test]
fn the_gateway_ending_a_session_is_the_clients_end() {
    let (cut, cut_wait) = mpsc::channel::<()>();
    let script = script(move |_, link| {
        trust_gateway(link);
        open(link);
        tunnel_ready(link, settings("utun9", "10.10.0.5"));
        let _ = cut_wait.recv();
        error_line(link, "Server closed connection");
        ended(-EPIPE)
    });
    let mut session = Session::start(script, Setup::default());
    session.bring_up(vec![gateway_route("192.0.2.1")]);
    cut.send(()).unwrap();
    session
        .next_job()
        .finish(NetworkOutcome::Reverted { left: Vec::new() });
    let facts = session.wait_for_exit();
    assert_eq!(facts.stop_initiator, StopInitiator::Session {});
    assert_eq!(facts.furthest_milestone, Milestone::TunnelUp);
    assert_eq!(facts.end_code, -EPIPE);
    assert!(session.has_line(OutputSource::Error, "Server closed connection"));
    session.assert_finished_cleanly();
}

#[test]
fn output_goes_out_in_batches_before_each_event() {
    let mut session = Session::start(
        up(settings("utun9", "10.10.0.5")),
        Setup::default().submitting_otp(),
    );
    session.bring_up(vec![gateway_route("192.0.2.1")]);
    session.stop_and_revert(StopReason::UserDisconnect);
    let mut output_since_event = false;
    for delivered in &session.delivered {
        match delivered {
            Delivered::Output(lines) => {
                assert_ne!(lines.as_slice(), []);
                output_since_event = true;
            }
            Delivered::Log(..) | Delivered::Stuck => {}
            Delivered::Event(kind) => {
                let silent = matches!(
                    kind,
                    HelperEventKind::Milestone {
                        milestone: Milestone::OtpPrompted | Milestone::Authenticated,
                        ..
                    }
                );
                assert!(
                    silent || output_since_event,
                    "no output right before {kind:?}"
                );
                output_since_event = false;
            }
        }
    }
}
