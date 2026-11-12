//! A client that logs in and opens the tunnel in separate runs: one attempt across two processes,
//! the session passed on stdin and never logged, and the logout request when the client could not
//! log out itself.
#![cfg(unix)] // The fake clients are shell scripts; tests/windows.rs covers Windows.

mod common;

use std::time::Duration;

use common::*;
use fvm_contract::app_helper::{Milestone, ProcessExitStatus, StopInitiator, StopReason};
use fvm_contract::backend::SessionLaunch;

const COOKIE: &str = "SVPNCOOKIE=abc123";

/// Logs in with the password and the OTP and prints the session the way openconnect's login run
/// does. The cookie is put together when printed, so the script text, which the log shows as the
/// command, does not hold it.
const LOGIN: &str = "\
printf 'VPN account password: '
IFS= read -r pw
printf '\\nINFO:   Connected to gateway.\\nTwo-factor authentication token: '
IFS= read -r otp
printf \"COOKIE='SVPNCOOKIE=%s%s'\\nHOST='203.0.113.10'\\n\" abc 123";

/// Records the session it reads and stays up; SIGTERM makes it log out and exit, as openconnect
/// does.
const TUNNEL: &str = "\
IFS= read -r cookie
printf 'tunnel_cookie=%s\\n' \"$cookie\" >> \"$1\"
trap 'printf \"tunnel_ended=term\\n\" >> \"$1\"; exit 0' TERM
printf 'INFO:   Tunnel is up and running.\\n'
while :; do /bin/sleep 0.05; done";

/// Records what it reads and answers like the adapter's curl request.
const LOGOUT: &str = "\
IFS= read -r header
printf 'logout_input=%s\\n' \"$header\" >> \"$1\"
printf 'HTTP 200'";

fn attempt(login: &str, tunnel: &str, logout: &str, setup: Setup) -> Session {
    let record = RecordFile::new();
    let mut launch = script_launch(login, &record);
    launch.session = Some(Box::new(SessionLaunch {
        tunnel: script_launch(tunnel, &record),
        logout: script_launch(logout, &record),
    }));
    Session::start(launch, record, setup.submitting_otp())
}

fn client_starts(session: &Session) -> usize {
    session
        .delivered
        .iter()
        .filter(|delivered| matches!(delivered, Delivered::ClientStarted))
        .count()
}

fn session_never_shown(session: &Session) {
    assert!(
        session.texts().iter().all(|text| !text.contains("abc123")),
        "{:?}",
        session.texts()
    );
}

#[test]
fn the_two_runs_are_one_attempt() {
    let mut session = attempt(LOGIN, TUNNEL, LOGOUT, Setup::default());
    session.wait_for(Milestone::TunnelUp);
    assert_eq!(session.milestones(), FULL_FLOW);
    assert_eq!(client_starts(&session), 2);
    assert!(session.has_text("‹session received; its line is not shown›"));
    assert!(session.has_text("HOST='203.0.113.10'"));
    assert!(session.wait(Duration::from_secs(5), |session| {
        session.record.value("tunnel_cookie").is_some()
    }));
    assert_eq!(
        session.record.value("tunnel_cookie"),
        Some(COOKIE.to_owned())
    );

    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 0 });
    assert!(facts.up_duration.is_some());
    // It logged out itself on SIGTERM.
    assert_eq!(
        session.record.value("tunnel_ended"),
        Some("term".to_owned())
    );
    assert_eq!(session.record.value("logout_input"), None);
    session_never_shown(&session);
}

#[test]
fn a_tunnel_run_killed_after_the_grace_period_is_logged_out() {
    let tunnel = "\
IFS= read -r cookie
trap '' TERM
printf 'INFO:   Tunnel is up and running.\\n'
while :; do /bin/sleep 0.05; done";
    let setup = Setup {
        grace_period: Duration::from_millis(300),
        ..Setup::default()
    };
    let mut session = attempt(LOGIN, tunnel, LOGOUT, setup);
    session.wait_for(Milestone::TunnelUp);
    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();
    assert_eq!(facts.exit_status, ProcessExitStatus::Signaled { signal: 9 });
    assert_eq!(
        session.record.value("logout_input"),
        Some(format!("Cookie: {COOKIE}"))
    );
    let texts = session.texts();
    let logout = texts
        .iter()
        .position(|&text| text == "‹logout request sent: HTTP 200›")
        .expect("the logout line");
    let exited = texts
        .iter()
        .position(|text| text.starts_with("‹exited: "))
        .expect("the exit line");
    assert!(exited < logout, "{texts:?}");
    session_never_shown(&session);
}

#[test]
fn a_tunnel_run_that_ends_by_itself_is_not_logged_out() {
    let tunnel = "\
IFS= read -r cookie
printf 'INFO:   Tunnel is up and running.\\n'
/bin/sleep 0.2
exit 2";
    let mut session = attempt(LOGIN, tunnel, LOGOUT, Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(facts.stop_initiator, StopInitiator::Client {});
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 2 });
    assert_eq!(facts.furthest_milestone, Some(Milestone::TunnelUp));
    assert_eq!(session.record.value("logout_input"), None);
    assert!(!session.texts().iter().any(|text| text.contains("logout")));
}

#[test]
fn a_login_run_without_a_session_ends_the_attempt() {
    let login = "\
printf 'VPN account password: '
IFS= read -r pw
printf '\\nINFO:   Connected to gateway.\\nTwo-factor authentication token: '
IFS= read -r otp
exit 1";
    let mut session = attempt(login, TUNNEL, LOGOUT, Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 1 });
    assert_eq!(facts.furthest_milestone, Some(Milestone::OtpSubmitted));
    assert_eq!(client_starts(&session), 1);
    assert_eq!(session.record.value("tunnel_cookie"), None);
    assert_eq!(session.record.value("logout_input"), None);
}

#[test]
fn a_session_whose_tunnel_run_cannot_start_is_logged_out() {
    let record = RecordFile::new();
    let mut launch = script_launch(LOGIN, &record);
    launch.session = Some(Box::new(SessionLaunch {
        tunnel: shell("/nonexistent/openconnect", Vec::new()),
        logout: script_launch(LOGOUT, &record),
    }));
    let mut session = Session::start(launch, record, Setup::default().submitting_otp());
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.exit_status,
        ProcessExitStatus::SpawnFailed { errno: 2 }
    );
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
    assert_eq!(
        session.record.value("logout_input"),
        Some(format!("Cookie: {COOKIE}"))
    );
    assert!(
        session
            .texts()
            .iter()
            .any(|text| text.starts_with("‹spawn failed: /nonexistent/openconnect: "))
    );
}

#[test]
fn a_failed_logout_request_is_logged_and_the_exit_still_reported() {
    let tunnel = "\
IFS= read -r cookie
trap '' TERM
printf 'INFO:   Tunnel is up and running.\\n'
while :; do /bin/sleep 0.05; done";
    let logout = "\
IFS= read -r header
printf 'HTTP 000'
exit 7";
    let setup = Setup {
        grace_period: Duration::from_millis(300),
        ..Setup::default()
    };
    let mut session = attempt(LOGIN, tunnel, logout, setup);
    session.wait_for(Milestone::TunnelUp);
    session.run.stop(StopReason::UserDisconnect);
    session.wait_for_exit();
    assert!(
        session.has_text("‹logout request failed: HTTP 000 (code 7)›"),
        "{:?}",
        session.texts()
    );
}

#[test]
fn a_session_line_that_cannot_be_read_is_hidden_too() {
    let login = "\
printf 'VPN account password: '
IFS= read -r pw
printf '\\nINFO:   Connected to gateway.\\n'
printf \"COOKIE='SVPNCOOKIE=%s%s\\n\" abc 123
exit 0";
    let mut session = attempt(login, TUNNEL, LOGOUT, Setup::default());
    let facts = session.wait_for_exit();
    assert!(session.has_text("‹a line holding a secret is not shown›"));
    assert_eq!(client_starts(&session), 1);
    assert_eq!(
        facts.furthest_milestone,
        Some(Milestone::ConnectedToGateway)
    );
    session_never_shown(&session);
}
