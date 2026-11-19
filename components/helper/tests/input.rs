mod common;

use std::sync::mpsc;

use common::*;
use fvm_contract::app_helper::{
    HelperError, HelperErrorCode, Milestone, StopInitiator, StopReason,
};
use fvm_helper_core::session::backend::{FormKind, SessionEvent};

fn wrong_phase(message: &str) -> HelperError {
    HelperError {
        code: HelperErrorCode::WrongPhase,
        message: message.to_owned(),
    }
}

#[test]
fn an_otp_before_the_prompt_is_refused() {
    for (trusted, milestone) in [
        (false, Milestone::Spawned),
        (true, Milestone::ConnectedToGateway),
    ] {
        let script = script(move |_, link| {
            if trusted {
                trust_gateway(link);
            }
            wait_for_cancel(link);
            ended(-EIO)
        });
        let mut session = Session::start(script, Setup::default());
        session.wait_for(milestone);
        assert_eq!(
            session.run.submit_otp(OTP.to_owned()),
            Err(wrong_phase("openconnect is not waiting for an OTP"))
        );
        session.run.stop(StopReason::UserDisconnect);
        let facts = session.wait_for_exit();
        assert_eq!(facts.furthest_milestone, milestone);
        assert!(!session.has_text("‹OTP sent›"));
    }
}

#[test]
fn an_otp_is_accepted_once_while_prompted_and_reaches_the_session() {
    let (got, received) = mpsc::channel();
    let script = script(move |_, link| {
        trust_gateway(link);
        let otp = ask_otp(link);
        got.send(otp.map(|code| code.to_string())).unwrap();
        wait_for_cancel(link);
        ended(-EIO)
    });
    let mut session = Session::start(script, Setup::default());
    session.wait_for(Milestone::OtpPrompted);
    session.run.submit_otp(OTP.to_owned()).unwrap();
    session.wait_for(Milestone::OtpSubmitted);
    assert_eq!(received.recv().unwrap().as_deref(), Some(OTP));
    assert_eq!(
        session.run.submit_otp("654321".to_owned()),
        Err(wrong_phase("an OTP was already submitted for this attempt"))
    );
    session.run.stop(StopReason::UserDisconnect);
    assert_eq!(
        session.run.submit_otp(OTP.to_owned()),
        Err(wrong_phase("openconnect is not running"))
    );
    let facts = session.wait_for_exit();
    assert_eq!(facts.furthest_milestone, Milestone::OtpSubmitted);
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(session.count_text("‹OTP sent›"), 1);
    session.assert_finished_cleanly();
}

#[test]
fn an_otp_that_is_not_one_is_refused_before_the_run_sees_it() {
    let script = script(|_, link| {
        trust_gateway(link);
        let otp = ask_otp(link);
        assert!(otp.is_none(), "the session got an OTP");
        ended(-EIO)
    });
    let mut session = Session::start(script, Setup::default());
    session.wait_for(Milestone::OtpPrompted);
    let refused = session.run.submit_otp("12345".to_owned()).unwrap_err();
    assert_eq!(refused.code, HelperErrorCode::InvalidRequest);
    session.run.stop(StopReason::OtpTimeout);
    let facts = session.wait_for_exit();
    assert_eq!(facts.furthest_milestone, Milestone::OtpPrompted);
}

#[test]
fn a_stop_ends_the_otp_wait_and_refuses_a_later_otp() {
    let script = script(|_, link| {
        trust_gateway(link);
        match ask_otp(link) {
            Some(_) => ended(0),
            None => ended(-EIO),
        }
    });
    let mut session = Session::start(script, Setup::default());
    session.wait_for(Milestone::OtpPrompted);
    session.run.stop(StopReason::OtpTimeout);
    let facts = session.wait_for_exit();
    assert_eq!(
        session.run.submit_otp(OTP.to_owned()),
        Err(wrong_phase("openconnect is not running"))
    );
    assert_eq!(facts.end_code, -EIO);
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::OtpTimeout
        }
    );
    assert!(session.has_text("‹session cancelled (reason: otpTimeout)›"));
}

#[test]
fn a_repeated_login_form_ends_the_run_as_a_repeated_prompt() {
    let script = script(|_, link| {
        trust_gateway(link);
        link.report(SessionEvent::FormRepeated(FormKind::Login));
        error_line(link, "Failed to complete authentication");
        ended(-EPERM)
    });
    let mut session = Session::start(script, Setup::default());
    let facts = session.wait_for_exit();
    assert!(session.has_text("‹password asked again: the gateway rejected the login›"));
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::PromptRepeated
        }
    );
    assert_eq!(facts.furthest_milestone, Milestone::ConnectedToGateway);
    session.assert_finished_cleanly();
}

#[test]
fn a_repeated_otp_form_ends_the_run_as_a_repeated_prompt() {
    let script = script(|_, link| {
        trust_gateway(link);
        if ask_otp(link).is_some() {
            link.report(SessionEvent::FormRepeated(FormKind::Otp));
        }
        ended(-EPERM)
    });
    let mut session = Session::start(script, Setup::default().submitting_otp());
    let facts = session.wait_for_exit();
    assert_eq!(session.count_text("‹OTP sent›"), 1);
    assert!(session.has_text("‹OTP asked again: the gateway rejected the OTP›"));
    assert_eq!(facts.furthest_milestone, Milestone::OtpSubmitted);
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::PromptRepeated
        }
    );
}
