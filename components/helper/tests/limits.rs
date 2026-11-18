mod common;

use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use common::*;
use fvm_contract::app_helper::{
    HelperError, HelperErrorCode, Milestone, ProcessExitStatus, StopInitiator, StopReason,
};
use fvm_contract::helper_internal::NetworkOutcome;
use fvm_helper_core::session::StageLimits;
use fvm_helper_core::session::backend::Link;

const TINY: Duration = Duration::from_millis(150);

#[test]
fn a_stalled_stage_cancels_the_session() {
    type Shrink = fn(&mut StageLimits);
    type Stall = fn(&Link);
    let stalls: [(&str, Milestone, bool, Shrink, Stall); 5] = [
        (
            "spawnToGateway",
            Milestone::Spawned,
            false,
            |limits| limits.spawn_to_gateway = TINY,
            |_| {},
        ),
        (
            "gatewayToPromptOrAuth",
            Milestone::ConnectedToGateway,
            false,
            |limits| limits.gateway_to_prompt_or_auth = TINY,
            |link| trust_gateway(link),
        ),
        (
            "otpPromptToSubmit",
            Milestone::OtpPrompted,
            false,
            |limits| limits.otp_prompt_to_submit = TINY,
            |link| {
                trust_gateway(link);
                assert!(ask_otp(link).is_none(), "the watchdog ends the OTP wait");
            },
        ),
        (
            "otpSubmitToAuth",
            Milestone::OtpSubmitted,
            true,
            |limits| limits.otp_submit_to_auth = TINY,
            |link| {
                trust_gateway(link);
                assert!(ask_otp(link).is_some());
            },
        ),
        (
            "authToTunnel",
            Milestone::Authenticated,
            true,
            |limits| limits.auth_to_tunnel = TINY,
            |link| {
                trust_gateway(link);
                assert!(ask_otp(link).is_some());
                open(link);
            },
        ),
    ];
    let runs: Vec<_> = stalls
        .into_iter()
        .map(|(name, furthest, submits_otp, shrink, stall)| {
            thread::spawn(move || {
                let gateway = FakeGateway::start();
                let mut setup = Setup::default().logging_out_to(&gateway);
                if submits_otp {
                    setup = setup.submitting_otp();
                }
                shrink(&mut setup.timing.stages);
                let script = script(move |_, link| {
                    stall(link);
                    wait_for_cancel(link);
                    ended(-EIO, false)
                });
                let mut session = Session::start(script, setup);
                let facts = session.wait_for_exit();
                assert_eq!(
                    facts.stop_initiator,
                    StopInitiator::Helper {
                        reason: StopReason::Watchdog
                    },
                    "{name}"
                );
                assert_eq!(facts.furthest_milestone, Some(furthest), "{name}");
                assert_eq!(facts.up_duration, None);
                let texts = session.texts();
                let expired = format!("‹watchdog: {name} did not finish within 0.15s›");
                let fired = texts
                    .iter()
                    .position(|text| *text == expired)
                    .unwrap_or_else(|| panic!("{expired} missing from {texts:?}"));
                assert_eq!(texts[fired + 1], "‹session cancelled (reason: watchdog)›");
                session.assert_finished_cleanly();
            })
        })
        .collect();
    for run in runs {
        run.join().unwrap();
    }
}

#[test]
fn no_stage_limit_once_the_tunnel_is_up() {
    let mut setup = Setup::default().submitting_otp();
    let tiny = Duration::from_millis(300);
    setup.timing.stages = StageLimits {
        spawn_to_gateway: tiny,
        gateway_to_prompt_or_auth: tiny,
        otp_prompt_to_submit: tiny,
        otp_submit_to_auth: tiny,
        auth_to_tunnel: tiny,
    };
    let mut session = Session::start(up(settings("utun9", "10.10.0.5")), setup);
    session.bring_up(vec![gateway_route("192.0.2.1")]);
    session.settle(Duration::from_millis(500));
    assert_eq!(session.exit_facts(), None);
    let facts = session.stop_and_revert(StopReason::Sleep);
    assert!(
        !session
            .texts()
            .iter()
            .any(|text| text.starts_with("‹watchdog: "))
    );
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::Sleep
        }
    );
}

#[test]
fn a_network_setup_that_outlasts_the_tunnel_stage_ends_the_run_as_a_network_setup_failure() {
    let mut setup = Setup::default();
    setup.timing.stages.auth_to_tunnel = Duration::from_secs(1);
    let mut session = Session::start(up_without_otp(settings("utun9", "10.10.0.5")), setup);
    session
        .next_job()
        .finish(looked_up(vec![gateway_route("192.0.2.1")]));
    let apply = session.next_job();
    thread::sleep(Duration::from_millis(1200));
    apply.finish(applied(vec![gateway_route("192.0.2.1")]));
    session
        .next_job()
        .finish(NetworkOutcome::Reverted { left: Vec::new() });
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::NetworkSetupFailed
        }
    );
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
    assert_eq!(
        session.count_text("‹watchdog: authToTunnel did not finish within 1s›"),
        1
    );
}

#[test]
fn a_second_stop_keeps_the_first_reason() {
    let mut session = Session::start(up(settings("utun9", "10.10.0.5")), Setup::default());
    session.wait_for(Milestone::OtpPrompted);
    session.run.stop(StopReason::UserDisconnect);
    session.run.stop(StopReason::AppQuit);
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: -EIO });
    assert_eq!(
        session
            .texts()
            .iter()
            .filter(|text| text.starts_with("‹session cancelled"))
            .count(),
        1
    );
    session.run.stop(StopReason::UserDisconnect);
    session.settle(Duration::from_millis(50));
    assert_eq!(session.events().len(), 4);
}

#[test]
fn a_session_that_outlasts_its_cancel_is_cancelled_again_until_it_ends() {
    let repeat = Duration::from_millis(300);
    let mut setup = Setup::default();
    setup.timing.cancel_repeat = repeat;
    let writes = Arc::new(AtomicUsize::new(0));
    let (written, cancels) = mpsc::channel();
    let script = script({
        let writes = Arc::clone(&writes);
        move |_, link| {
            assert!(link.arm_cancel(move || {
                writes.fetch_add(1, Ordering::SeqCst);
                let _ = written.send(());
            }));
            trust_gateway(link);
            assert_eq!(cancels.iter().take(3).count(), 3);
            link.disarm_cancel();
            thread::sleep(repeat * 2);
            ended(-EINTR, true)
        }
    });
    let mut session = Session::start(script, setup);
    session.wait_for(Milestone::ConnectedToGateway);
    let stopped = Instant::now();
    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();
    let waited = stopped.elapsed();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(
        facts.exit_status,
        ProcessExitStatus::Exited { code: -EINTR }
    );
    assert!(waited >= repeat * 4, "{waited:?}");
    session.settle(repeat * 3);
    assert_eq!(writes.load(Ordering::SeqCst), 3);
    let again = "‹session still running: cancel sent again›";
    let texts = session.texts();
    let end = texts
        .iter()
        .position(|text| text.starts_with("‹session ended: "))
        .unwrap_or_else(|| panic!("no session end in {texts:?}"));
    assert_eq!(
        texts[..end].iter().filter(|&&text| text == again).count(),
        2,
        "{texts:?}"
    );
    assert!(!texts[end..].contains(&again), "{texts:?}");
    session.assert_finished_cleanly();
}

#[test]
fn a_session_that_returned_is_not_cancelled_again_while_it_logs_out() {
    let repeat = Duration::from_millis(100);
    let hold = repeat * 5;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut setup = Setup::default();
    setup.timing.cancel_repeat = repeat;
    setup.request.host = "127.0.0.1".to_owned();
    setup.request.port = i64::from(listener.local_addr().unwrap().port());
    let gateway = thread::spawn(move || {
        let (connection, _) = listener.accept().unwrap();
        thread::sleep(hold);
        drop(connection);
    });
    let script = script(|_, link| {
        trust_gateway(link);
        open(link);
        wait_for_cancel(link);
        ended(-EINTR, false)
    });
    let mut session = Session::start(script, setup);
    session.wait_for(Milestone::Authenticated);
    let stopped = Instant::now();
    session.run.stop(StopReason::UserDisconnect);
    session.wait_for_exit();
    let waited = stopped.elapsed();
    gateway.join().unwrap();
    assert!(waited >= hold, "{waited:?}");
    let texts = session.texts();
    assert!(
        texts
            .iter()
            .any(|text| text.starts_with("‹logout request failed: HTTP 000")),
        "{texts:?}"
    );
    assert_eq!(
        session.count_text("‹session still running: cancel sent again›"),
        0,
        "{texts:?}"
    );
    session.assert_finished_cleanly();
}

fn opened_then_returned() -> Script {
    script(|_, link| {
        trust_gateway(link);
        open(link);
        ended(-EIO, false)
    })
}

fn assert_ended_by_the_client(session: &Session) {
    let facts = session.exit_facts().unwrap();
    assert_eq!(facts.stop_initiator, StopInitiator::Client {});
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: -EIO });
    let texts = session.texts();
    assert!(
        !texts
            .iter()
            .any(|text| text.starts_with("‹watchdog: ") || text.starts_with("‹session cancelled")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.starts_with("‹logout request failed: HTTP 000")),
        "{texts:?}"
    );
    session.assert_finished_cleanly();
}

#[test]
fn a_stage_limit_does_not_expire_while_a_returned_session_logs_out() {
    let gateway = FakeGateway::holding();
    let mut setup = Setup::default().logging_out_to(&gateway);
    setup.timing.stages.auth_to_tunnel = TINY;
    let mut session = Session::start(opened_then_returned(), setup);
    gateway.wait_for_connections(1);
    session.settle(TINY * 3);
    assert_eq!(session.exit_facts(), None);
    gateway.release();
    session.wait_for_exit();
    assert_ended_by_the_client(&session);
}

#[test]
fn a_stop_while_a_returned_session_logs_out_changes_nothing() {
    let gateway = FakeGateway::holding();
    let setup = Setup::default().logging_out_to(&gateway);
    let mut session = Session::start(opened_then_returned(), setup);
    gateway.wait_for_connections(1);
    assert!(session.wait(Duration::from_secs(5), |session| {
        !session.run.session_running()
    }));
    session.run.stop(StopReason::UserDisconnect);
    session.settle(Duration::from_millis(100));
    assert_eq!(session.exit_facts(), None);
    assert!(!session.run.session_running());
    gateway.release();
    session.wait_for_exit();
    assert_ended_by_the_client(&session);
}

#[test]
fn an_otp_while_a_returned_session_logs_out_is_refused_as_for_a_run_that_is_not_running() {
    let gateway = FakeGateway::holding();
    let setup = Setup::default().logging_out_to(&gateway);
    let mut session = Session::start(opened_then_returned(), setup);
    gateway.wait_for_connections(1);
    assert!(session.wait(Duration::from_secs(5), |session| {
        !session.run.session_running()
    }));
    assert_eq!(
        session.run.submit_otp(OTP.to_owned()),
        Err(HelperError {
            code: HelperErrorCode::WrongPhase,
            message: "openconnect is not running".to_owned(),
        })
    );
    session.settle(Duration::from_millis(100));
    assert_eq!(session.exit_facts(), None);
    gateway.release();
    session.wait_for_exit();
    assert!(!session.has_text("‹OTP sent›"));
    assert!(!session.reached(Milestone::OtpSubmitted));
    assert_ended_by_the_client(&session);
}
