mod common;

use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use common::*;
use fvm_contract::app_helper::{
    HelperError, HelperErrorCode, Milestone, ProcessExitStatus, StopInitiator, StopReason,
};
use fvm_contract::helper_internal::{
    AppRequest, FromEntry, LogCategory, LogLevel, NetworkOutcome, ToEntry,
};
use fvm_helper_core::entry;
use fvm_helper_core::session::backend::{Command, Link};
use fvm_helper_core::session::{SessionManager, StageLimits};

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
                let mut setup = Setup::default();
                if submits_otp {
                    setup = setup.submitting_otp();
                }
                shrink(&mut setup.timing.stages);
                let script = script(move |_, link| {
                    stall(link);
                    wait_for_cancel(link);
                    ended(-EIO)
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
fn a_session_that_outlasts_its_cancel_is_detached_once() {
    let detach_after = Duration::from_millis(300);
    let mut setup = Setup::default();
    setup.timing.detach_after = detach_after;
    let writes = Arc::new(Mutex::new(Vec::new()));
    let script = script({
        let writes = Arc::clone(&writes);
        move |_, link| {
            let (written, wait) = mpsc::channel();
            assert!(link.arm_cancel(move |command| {
                writes.lock().unwrap().push((command, Instant::now()));
                let _ = written.send(());
            }));
            trust_gateway(link);
            assert_eq!(wait.iter().take(2).count(), 2);
            thread::sleep(detach_after * 3);
            link.disarm_cancel();
            ended(-EINTR)
        }
    });
    let mut session = Session::start(script, setup);
    session.wait_for(Milestone::ConnectedToGateway);
    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    let writes = writes.lock().unwrap().clone();
    let commands: Vec<_> = writes.iter().map(|(command, _)| *command).collect();
    assert_eq!(commands, [Command::Cancel, Command::Detach]);
    let waited = writes[1].1 - writes[0].1;
    assert!(waited >= detach_after, "{waited:?}");
    let texts = session.texts();
    let end = texts
        .iter()
        .position(|text| text.starts_with("‹session ended: "))
        .unwrap_or_else(|| panic!("no session end in {texts:?}"));
    let detached = "‹session still running: detach sent›";
    assert_eq!(
        texts[..end]
            .iter()
            .filter(|&&text| text == detached)
            .count(),
        1,
        "{texts:?}"
    );
    assert!(!texts[end..].contains(&detached), "{texts:?}");
    session.assert_finished_cleanly();
}

#[test]
fn a_session_that_returned_is_neither_detached_nor_stuck_while_its_secret_is_deleted() {
    let detach_after = Duration::from_millis(100);
    let mut setup = Setup::default();
    setup.timing.detach_after = detach_after;
    setup.timing.stuck_after = detach_after * 2;
    let release = setup.secrets.hold_deletes();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let script = script({
        let commands = Arc::clone(&commands);
        move |_, link| {
            trust_gateway(link);
            open(link);
            let (written, wait) = mpsc::channel();
            assert!(link.arm_cancel(move |command| {
                commands.lock().unwrap().push(command);
                let _ = written.send(());
            }));
            wait.recv().unwrap();
            ended(-EINTR)
        }
    });
    let mut session = Session::start(script, setup);
    session.wait_for(Milestone::Authenticated);
    session.run.stop(StopReason::UserDisconnect);
    assert!(session.wait(Duration::from_secs(5), |session| {
        !session.run.session_running()
    }));
    session.settle(detach_after * 5);
    assert_eq!(session.exit_facts(), None);
    release.send(()).unwrap();
    session.wait_for_exit();
    assert_eq!(*commands.lock().unwrap(), [Command::Cancel]);
    assert_eq!(
        session.count_text("‹session still running: detach sent›"),
        0,
        "{:?}",
        session.texts()
    );
    assert!(
        !session
            .delivered
            .iter()
            .any(|delivered| matches!(delivered, Delivered::Stuck))
    );
    session.assert_finished_cleanly();
}

#[test]
fn a_session_still_running_after_its_cancel_is_reported_stuck_after_its_lines() {
    let stuck_after = Duration::from_millis(300);
    let mut setup = Setup::default();
    setup.timing.detach_after = Duration::from_millis(100);
    setup.timing.stuck_after = stuck_after;
    let (release, held) = mpsc::channel::<()>();
    let script = script(move |_, link| {
        assert!(link.arm_cancel(|_| {}));
        trust_gateway(link);
        let _ = held.recv();
        link.disarm_cancel();
        ended(-EINTR)
    });
    let mut session = Session::start(script, setup);
    session.wait_for(Milestone::ConnectedToGateway);
    let stopped = Instant::now();
    session.run.stop(StopReason::UserDisconnect);
    let stuck = |session: &Session| {
        session
            .delivered
            .iter()
            .filter(|delivered| matches!(delivered, Delivered::Stuck))
            .count()
    };
    assert!(
        session.wait(Duration::from_secs(5), |session| stuck(session) == 1),
        "{:?}",
        session.texts()
    );
    let waited = stopped.elapsed();
    assert!(waited >= stuck_after, "{waited:?}");
    assert!(session.run.session_running());
    assert_eq!(session.exit_facts(), None);
    let line = "‹session did not end within 0.3s after the cancel: restarting the Helper›";
    let log = format!(
        "0277070470 run {ATTEMPT}: session did not end within 0.3s after the cancel; aborting so the OS restarts the Helper"
    );
    let delivered = &session.delivered;
    let shown = delivered.iter().position(|delivered| {
        matches!(delivered, Delivered::Output(lines) if lines.iter().any(|shown| shown.text == line))
    });
    let logged = delivered.iter().position(|delivered| {
        matches!(
            delivered,
            Delivered::Log(LogLevel::Error, LogCategory::Helper, message) if *message == log
        )
    });
    let reported = delivered
        .iter()
        .position(|delivered| matches!(delivered, Delivered::Stuck));
    assert!(
        matches!((shown, logged, reported), (Some(shown), Some(logged), Some(reported)) if shown < logged && logged < reported),
        "{delivered:?}"
    );
    release.send(()).unwrap();
    session.wait_for_exit();
    assert_eq!(stuck(&session), 1);
    session.assert_finished_cleanly();
}

#[test]
fn a_stuck_session_has_the_entry_abort_the_helper_after_its_lines() {
    let mut timing = timing();
    timing.stuck_after = Duration::from_millis(300);
    let (release, held) = mpsc::channel::<()>();
    let script = script(move |_, _| {
        let _ = held.recv();
        ended(-EINTR)
    });
    let directory = RunDirectory::new();
    let sessions = SessionManager::new(
        Arc::new(FakeBackend::new([script])),
        Arc::new(FakeSecrets::default()),
        directory.path.clone(),
        timing,
    );
    let (sender, messages) = mpsc::channel();
    let inbox = entry::start(sessions, move |message| {
        let _ = sender.send(message.clone());
    });
    let mut received = Vec::new();
    let mut wait_for = |wanted: fn(&FromEntry) -> bool| {
        loop {
            let message = messages
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|error| panic!("{error}: {received:?}"));
            let found = wanted(&message);
            received.push(message);
            if found {
                return;
            }
        }
    };
    wait_for(|message| matches!(message, FromEntry::Ready {}));
    inbox.deliver(ToEntry::AppConnected { app: 1, uid: 1 });
    wait_for(|message| {
        matches!(
            message,
            FromEntry::Admitted {
                app: 1,
                accept: true
            }
        )
    });
    let json = serde_json::to_string(&request(ATTEMPT)).unwrap();
    inbox.deliver(ToEntry::Request {
        id: 1,
        app: 1,
        request: AppRequest::Connect { json },
    });
    wait_for(|message| matches!(message, FromEntry::Reply { id: 1, json: None }));
    inbox.deliver(ToEntry::Request {
        id: 2,
        app: 1,
        request: AppRequest::Disconnect {
            attempt: ATTEMPT.to_owned(),
            reason: "userDisconnect".to_owned(),
        },
    });
    wait_for(|message| matches!(message, FromEntry::AbortAfterDraining {}));
    let shown = received.iter().position(|message| {
        matches!(
            message,
            FromEntry::SendOutput { app: 1, json }
                if json.contains("‹session did not end within 0.3s after the cancel: restarting the Helper›")
        )
    });
    let logged = received.iter().position(|message| {
        matches!(
            message,
            FromEntry::Log { level: LogLevel::Error, category: LogCategory::Helper, message }
                if message.ends_with("session did not end within 0.3s after the cancel; aborting so the OS restarts the Helper")
        )
    });
    let aborted = received.len() - 1;
    assert!(
        matches!((shown, logged), (Some(shown), Some(logged)) if shown < logged && logged < aborted),
        "{received:?}"
    );
    release.send(()).unwrap();
}

fn opened_then_returned() -> Script {
    script(|_, link| {
        trust_gateway(link);
        open(link);
        ended(-EIO)
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
    session.assert_finished_cleanly();
}

#[test]
fn a_stage_limit_does_not_expire_while_a_returned_session_deletes_its_secret() {
    let mut setup = Setup::default();
    setup.timing.stages.auth_to_tunnel = TINY;
    let secrets = Arc::clone(&setup.secrets);
    let release = secrets.hold_deletes();
    let mut session = Session::start(opened_then_returned(), setup);
    assert!(session.wait(Duration::from_secs(5), |_| !secrets.deletes().is_empty()));
    session.settle(TINY * 3);
    assert_eq!(session.exit_facts(), None);
    release.send(()).unwrap();
    session.wait_for_exit();
    assert_ended_by_the_client(&session);
}

#[test]
fn a_stop_while_a_returned_session_deletes_its_secret_changes_nothing() {
    let setup = Setup::default();
    let secrets = Arc::clone(&setup.secrets);
    let release = secrets.hold_deletes();
    let mut session = Session::start(opened_then_returned(), setup);
    assert!(session.wait(Duration::from_secs(5), |session| {
        !secrets.deletes().is_empty() && !session.run.session_running()
    }));
    session.run.stop(StopReason::UserDisconnect);
    session.settle(Duration::from_millis(100));
    assert_eq!(session.exit_facts(), None);
    assert!(!session.run.session_running());
    release.send(()).unwrap();
    session.wait_for_exit();
    assert_ended_by_the_client(&session);
}

#[test]
fn an_otp_while_a_returned_session_deletes_its_secret_is_refused_as_for_a_run_that_is_not_running()
{
    let setup = Setup::default();
    let secrets = Arc::clone(&setup.secrets);
    let release = secrets.hold_deletes();
    let mut session = Session::start(opened_then_returned(), setup);
    assert!(session.wait(Duration::from_secs(5), |session| {
        !secrets.deletes().is_empty() && !session.run.session_running()
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
    release.send(()).unwrap();
    session.wait_for_exit();
    assert!(!session.has_text("‹OTP sent›"));
    assert!(!session.reached(Milestone::OtpSubmitted));
    assert_ended_by_the_client(&session);
}
