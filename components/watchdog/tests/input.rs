//! When the password and the OTP go to the client's stdin.
#![cfg(unix)] // The fake clients are shell scripts; tests/windows.rs covers Windows.

mod common;

use std::time::Duration;

use common::*;
use fvm_contract::app_helper::{
    HelperError, HelperErrorCode, Milestone, ProcessExitStatus, StopInitiator, StopReason,
};

fn wrong_phase(message: &str) -> HelperError {
    HelperError {
        code: HelperErrorCode::WrongPhase,
        message: message.to_owned(),
    }
}

/// The script stalls at each stage, so the state is stable when the OTP arrives.
#[test]
fn otp_before_the_prompt_is_refused() {
    for (stage, milestone) in [
        ("spawned", Milestone::Spawned),
        ("gateway", Milestone::ConnectedToGateway),
    ] {
        let mut session = fixture("hang", &[stage], Setup::default());
        session.wait_for(milestone);
        assert_eq!(
            session.run.submit_otp(OTP.to_owned()),
            Err(wrong_phase("openfortivpn is not waiting for an OTP"))
        );
        session.run.stop(StopReason::UserDisconnect);
        let facts = session.wait_for_exit();
        assert_eq!(facts.furthest_milestone, Some(milestone));
        assert!(!session.has_text("‹OTP sent›"));
    }
}

/// Accepted once while prompted; refused when repeated and after a stop request.
#[test]
fn otp_is_accepted_once_while_prompted() {
    let mut session = fixture("hang", &["submitted"], Setup::default());
    session.wait_for(Milestone::OtpPrompted);
    session.run.submit_otp(OTP.to_owned()).unwrap();
    session.wait_for(Milestone::OtpSubmitted);
    assert_eq!(
        session.run.submit_otp("654321".to_owned()),
        Err(wrong_phase("an OTP was already submitted for this attempt"))
    );
    assert!(session.wait(Duration::from_secs(5), |session| {
        session.record.value("otp_sha256").is_some()
    }));
    session.run.stop(StopReason::UserDisconnect);
    assert_eq!(
        session.run.submit_otp(OTP.to_owned()),
        Err(wrong_phase("openfortivpn is not running"))
    );
    let facts = session.wait_for_exit();
    assert_eq!(facts.furthest_milestone, Some(Milestone::OtpSubmitted));
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(session.count_text("‹OTP sent›"), 1);
    assert_eq!(session.record.value("otp_sha256"), Some(sha256(OTP)));
    session.assert_finished_cleanly();
}

#[test]
fn otp_while_stopping_is_refused() {
    let setup = Setup {
        grace_period: Duration::from_millis(300),
        ..Setup::default()
    };
    let mut session = fixture("hang", &["otp", "ignore-term"], setup);
    session.wait_for(Milestone::OtpPrompted);
    session.run.stop(StopReason::OtpTimeout);
    assert_eq!(
        session.run.submit_otp(OTP.to_owned()),
        Err(wrong_phase("openfortivpn is not running"))
    );
    let facts = session.wait_for_exit();
    assert_eq!(facts.furthest_milestone, Some(Milestone::OtpPrompted));
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::OtpTimeout
        }
    );
}

#[test]
fn password_is_written_only_where_the_client_asks_for_it() {
    let script_text = "\
printf 'INFO:   Connected to gateway.\\nVPN account password: '
/bin/sleep 0.2
exit 0";
    let mut session = script(script_text, Setup::default());
    session.wait_for_exit();
    assert!(!session.has_text("‹password sent›"));
    assert_eq!(
        session.count_text("‹password prompt ignored: not expected at this stage›"),
        1
    );
    session.assert_finished_cleanly();
}

/// Asked again, the gateway rejected the login; the client would wait for an answer.
#[test]
fn a_repeated_password_prompt_ends_the_run_at_once() {
    let script_text = "\
printf 'VPN account password: '
IFS= read -r pw
printf 'first_len=%s\\n' \"${#pw}\" >> \"$1\"
printf '\\nVPN account password: '
IFS= read -r pw";
    let mut session = script(script_text, Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(session.count_text("‹password sent›"), 1);
    assert!(session.has_text("‹password asked again: the gateway rejected the login›"));
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::PromptRepeated
        }
    );
    assert_eq!(
        facts.exit_status,
        ProcessExitStatus::Signaled { signal: 15 }
    );
    // With PATH as the only variable the shell runs in the C locale, where `${#pw}` counts bytes.
    assert_eq!(
        session.record.value("first_len"),
        Some(PASSWORD.len().to_string())
    );
}

#[test]
fn a_repeated_otp_prompt_ends_the_run_at_once() {
    let script_text = "\
printf 'INFO:   Connected to gateway.\\nTwo-factor authentication token: '
IFS= read -r otp
printf '\\nTwo-factor authentication token: '
/bin/sleep 10";
    let mut session = script(script_text, Setup::default().submitting_otp());
    let facts = session.wait_for_exit();
    assert_eq!(session.count_text("‹OTP sent›"), 1);
    assert!(session.has_text("‹OTP asked again: the gateway rejected the OTP›"));
    assert_eq!(facts.furthest_milestone, Some(Milestone::OtpSubmitted));
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::PromptRepeated
        }
    );
}

#[test]
fn password_is_not_written_after_a_stop_request() {
    let script_text = "\
trap '' TERM
echo 'started=yes' >> \"$1\"
/bin/sleep 0.3
printf 'VPN account password: '
IFS= read -r pw";
    let setup = Setup {
        grace_period: Duration::from_millis(700),
        ..Setup::default()
    };
    let mut session = script(script_text, setup);
    // SIGTERM must come after the trap is set.
    assert!(session.wait(Duration::from_secs(5), |session| {
        session.record.value("started").is_some()
    }));
    session.run.stop(StopReason::AppQuit);
    let facts = session.wait_for_exit();
    assert!(!session.has_text("‹password sent›"));
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::AppQuit
        }
    );
    assert_eq!(facts.exit_status, ProcessExitStatus::Signaled { signal: 9 });
}
