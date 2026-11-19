mod common;

use std::env;
use std::io::{self, Write};
use std::process::{self, ExitStatus};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use common::*;
use fvm_contract::app_helper::{
    HelperError, HelperErrorCode, HelperEventKind, Milestone, StopInitiator, StopReason,
};
use fvm_contract::helper_internal::{
    AppRequest, FromEntry, LogCategory, LogLevel, NetworkOutcome, ToEntry,
};
use fvm_helper_core::entry;
use fvm_helper_core::recovery::RecordSlot;
use fvm_helper_core::session::backend::{Command, Link};
use fvm_helper_core::session::{Report, SessionManager, StageLimits, Timing};

const TINY: Duration = Duration::from_millis(150);

#[test]
fn a_stalled_stage_cancels_the_session() {
    type Shrink = fn(&mut StageLimits);
    type Stall = fn(&Link);
    let stalls: [(&str, Milestone, bool, Shrink, Stall); 5] = [
        (
            "startToGateway",
            Milestone::Started,
            false,
            |limits| limits.start_to_gateway = TINY,
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
                assert!(ask_otp(link).is_none(), "the stage limit ends the OTP wait");
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
                        reason: StopReason::StageLimit
                    },
                    "{name}"
                );
                assert_eq!(facts.furthest_milestone, furthest, "{name}");
                let texts = session.texts();
                let expired = format!("‹stage limit: {name} did not finish within 0.15s›");
                let fired = texts
                    .iter()
                    .position(|text| *text == expired)
                    .unwrap_or_else(|| panic!("{expired} missing from {texts:?}"));
                assert_eq!(texts[fired + 1], "‹session cancelled (reason: stageLimit)›");
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
        start_to_gateway: tiny,
        gateway_to_prompt_or_auth: tiny,
        otp_prompt_to_submit: tiny,
        otp_submit_to_auth: tiny,
        auth_to_tunnel: tiny,
    };
    let mut session = Session::start(up(settings("utun9", "10.10.0.5")), setup);
    session.bring_up(vec![gateway_route("192.0.2.1")]);
    session.settle(Duration::from_millis(500));
    assert_eq!(session.end_facts(), None);
    let facts = session.stop_and_revert(StopReason::Sleep);
    assert!(
        !session
            .texts()
            .iter()
            .any(|text| text.starts_with("‹stage limit: "))
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
    assert_eq!(facts.furthest_milestone, Milestone::Authenticated);
    assert_eq!(
        session.count_text("‹stage limit: authToTunnel did not finish within 1s›"),
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
    assert_eq!(facts.end_code, -EIO);
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
    assert_eq!(session.end_facts(), None);
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
    assert_eq!(session.end_facts(), None);
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
fn a_returned_session_whose_secret_is_not_deleted_in_time_is_reported_stuck_after_its_lines() {
    let stuck_after = Duration::from_millis(300);
    let mut setup = Setup::default();
    setup.timing.stuck_after = stuck_after;
    let release = setup.secrets.hold_deletes();
    let started = Instant::now();
    let mut session = Session::start(opened_then_returned(), setup);
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
    let waited = started.elapsed();
    assert!(waited >= stuck_after, "{waited:?}");
    assert!(!session.run.session_running());
    assert_eq!(session.end_facts(), None);
    let line = "‹saving and deleting the session secret did not finish within 0.3s after the session ended: restarting the Helper›";
    let log = format!(
        "0277070470 run {ATTEMPT}: saving and deleting the session secret did not finish within 0.3s after the session ended; aborting so the OS restarts the Helper"
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
    assert_ended_by_the_client(&session);
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
    let facts = session.end_facts().unwrap();
    assert_eq!(facts.stop_initiator, StopInitiator::Session {});
    assert_eq!(facts.furthest_milestone, Milestone::Authenticated);
    assert_eq!(facts.end_code, -EIO);
    let texts = session.texts();
    assert!(
        !texts.iter().any(
            |text| text.starts_with("‹stage limit: ") || text.starts_with("‹session cancelled")
        ),
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
    assert_eq!(session.end_facts(), None);
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
    assert_eq!(session.end_facts(), None);
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
    assert_eq!(session.end_facts(), None);
    release.send(()).unwrap();
    session.wait_for_exit();
    assert!(!session.has_text("‹OTP sent›"));
    assert!(!session.reached(Milestone::OtpSubmitted));
    assert_ended_by_the_client(&session);
}

const STALLED_AFTER: Duration = Duration::from_secs(1);

fn watched_timing() -> Timing {
    Timing {
        beat: Duration::from_millis(100),
        stalled_after: STALLED_AFTER,
        resume_gap: Duration::from_millis(500),
        ..timing()
    }
}

fn report_faults_to_stdout() {
    entry::report_faults(|line| {
        io::stdout()
            .write_all(format!("{line}\n").as_bytes())
            .unwrap();
    });
}

fn run_child(test: &str) -> (ExitStatus, String) {
    let child = process::Command::new(env::current_exe().unwrap())
        .args(["--exact", test, "--ignored"])
        .output()
        .unwrap();
    (child.status, String::from_utf8(child.stdout).unwrap())
}

#[cfg(unix)]
fn aborted(status: ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    const SIGABRT: i32 = 6;
    status.signal() == Some(SIGABRT)
}

#[cfg(windows)]
fn aborted(status: ExitStatus) -> bool {
    status.code() == Some(entry::RESTART_EXIT_CODE.cast_signed())
}

fn fault_lines(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| line.starts_with("watchdog: "))
        .collect()
}

fn assert_aborted_naming(test: &str, name: &str) {
    let (status, stdout) = run_child(test);
    assert!(aborted(status), "{status} {stdout}");
    let [line] = fault_lines(&stdout)[..] else {
        panic!("{stdout}");
    };
    let silent: f64 = line
        .strip_prefix(&format!("watchdog: {name} made no progress for "))
        .and_then(|rest| rest.strip_suffix("s; aborting so the OS restarts the Helper"))
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or_else(|| panic!("{line}"));
    assert!(silent >= STALLED_AFTER.as_secs_f64(), "{line}");
}

fn assert_not_aborted(test: &str) {
    let (status, stdout) = run_child(test);
    assert!(status.success(), "{status} {stdout}");
    assert!(fault_lines(&stdout).is_empty(), "{stdout}");
}

#[test]
fn a_run_loop_that_makes_no_progress_is_named_on_the_fault_channel_and_aborts_the_helper() {
    assert_aborted_naming("stall_a_run_loop", &format!("0277070470 run {ATTEMPT}"));
}

#[test]
#[ignore = "the child of a_run_loop_that_makes_no_progress_is_named_on_the_fault_channel_and_aborts_the_helper"]
fn stall_a_run_loop() {
    report_faults_to_stdout();
    let (_release, held) = mpsc::channel::<()>();
    let directory = RunDirectory::new();
    let script = script(|_, link| {
        trust_gateway(link);
        wait_for_cancel(link);
        ended(-EINTR)
    });
    let sessions = SessionManager::new(
        Arc::new(FakeBackend::new([script])),
        Arc::new(FakeSecrets::default()),
        directory.path.clone(),
        watched_timing(),
    );
    let mut run_request = request(ATTEMPT);
    let _run = sessions
        .start(
            &mut run_request,
            RecordSlot::First,
            None,
            None,
            move |report| {
                if matches!(
                    report,
                    Report::Event(HelperEventKind::Milestone {
                        milestone: Milestone::ConnectedToGateway,
                        ..
                    })
                ) {
                    let _ = held.recv();
                }
            },
        )
        .unwrap();
    thread::sleep(STALLED_AFTER * 10);
}

#[test]
fn the_entry_loop_that_makes_no_progress_is_named_on_the_fault_channel_and_aborts_the_helper() {
    assert_aborted_naming("stall_the_entry_loop", "entry");
}

#[test]
#[ignore = "the child of the_entry_loop_that_makes_no_progress_is_named_on_the_fault_channel_and_aborts_the_helper"]
fn stall_the_entry_loop() {
    report_faults_to_stdout();
    let (_release, held) = mpsc::channel::<()>();
    let (ready, is_ready) = mpsc::channel();
    let directory = RunDirectory::new();
    let sessions = SessionManager::new(
        Arc::new(FakeBackend::default()),
        Arc::new(FakeSecrets::default()),
        directory.path.clone(),
        watched_timing(),
    );
    let inbox = entry::start(sessions, move |message| match message {
        FromEntry::Ready {} => ready.send(()).unwrap(),
        FromEntry::Admitted { .. } => {
            let _ = held.recv();
        }
        _ => {}
    });
    is_ready.recv_timeout(Duration::from_secs(10)).unwrap();
    inbox.deliver(ToEntry::AppConnected { app: 1, uid: 1 });
    thread::sleep(STALLED_AFTER * 10);
}

#[test]
fn idle_loops_keep_beating() {
    assert_not_aborted("leave_the_entry_and_a_run_idle");
}

#[test]
#[ignore = "the child of idle_loops_keep_beating"]
fn leave_the_entry_and_a_run_idle() {
    report_faults_to_stdout();
    let (ready, is_ready) = mpsc::channel();
    let directory = RunDirectory::new();
    let sessions = SessionManager::new(
        Arc::new(FakeBackend::default()),
        Arc::new(FakeSecrets::default()),
        directory.path.clone(),
        watched_timing(),
    );
    let _inbox = entry::start(sessions, move |message| {
        if matches!(message, FromEntry::Ready {}) {
            ready.send(()).unwrap();
        }
    });
    is_ready.recv_timeout(Duration::from_secs(10)).unwrap();
    let setup = Setup {
        timing: watched_timing(),
        ..Setup::default()
    };
    let mut session = Session::start(up_without_otp(settings("utun9", "10.10.0.5")), setup);
    session.bring_up(vec![gateway_route("192.0.2.1")]);
    session.settle(STALLED_AFTER * 4);
    session.stop_and_revert(StopReason::UserDisconnect);
}

#[test]
fn a_run_that_exited_is_no_longer_watched() {
    assert_not_aborted("let_a_run_exit_and_wait");
}

#[test]
#[ignore = "the child of a_run_that_exited_is_no_longer_watched"]
fn let_a_run_exit_and_wait() {
    report_faults_to_stdout();
    let directory = RunDirectory::new();
    let script = script(|_, link| {
        trust_gateway(link);
        ended(-EIO)
    });
    let sessions = SessionManager::new(
        Arc::new(FakeBackend::new([script])),
        Arc::new(FakeSecrets::default()),
        directory.path.clone(),
        watched_timing(),
    );
    let (sender, reports) = mpsc::channel();
    let mut run_request = request(ATTEMPT);
    let _run = sessions
        .start(
            &mut run_request,
            RecordSlot::First,
            None,
            None,
            move |report| {
                let _ = sender.send(report);
            },
        )
        .unwrap();
    while !matches!(
        reports.recv_timeout(Duration::from_secs(10)).unwrap(),
        Report::Event(HelperEventKind::Exited { .. })
    ) {}
    thread::sleep(STALLED_AFTER * 4);
}
