//! Whole runs against the fake openfortivpn scripts.

mod common;

use std::time::{Duration, Instant};

use common::*;
use fvm_contract::app_helper::{
    ExitFacts, HelperErrorCode, HelperEventKind, Milestone, OutputSource, ProcessExitStatus,
    StopInitiator, StopReason,
};
use fvm_contract::helper_internal::RouteRepairOutcome;

fn client_exit(furthest: Milestone, code: i32) -> ExitFacts {
    ExitFacts {
        furthest_milestone: Some(furthest),
        stop_initiator: StopInitiator::Client {},
        cert_rejected_digests: Vec::new(),
        cert_rejected_at: None,
        tunnel_mode_rejected: false,
        up_duration: None,
        exit_status: ProcessExitStatus::Exited { code },
    }
}

#[test]
fn success_then_user_disconnect() {
    let mut session = fixture("success", &[], Setup::default().submitting_otp());
    session.wait_for(Milestone::TunnelUp);
    assert!(session.run.process_id().is_some());
    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();

    assert_eq!(session.milestones(), FULL_FLOW);
    for kind in session.events() {
        if let HelperEventKind::Milestone {
            milestone,
            tunnel_ip,
        } = kind
        {
            let expected =
                (*milestone == Milestone::TunnelUp).then(|| "10.212.134.200".parse().unwrap());
            assert_eq!(*tunnel_ip, expected, "{milestone:?}");
        }
    }
    assert_eq!(facts.furthest_milestone, Some(Milestone::TunnelUp));
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert!(facts.cert_rejected_digests.is_empty());
    assert_eq!(facts.cert_rejected_at, None);
    assert!(facts.up_duration.is_some());
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 0 });
    // "Restoring routes..." finished, so no repair.
    assert_eq!(session.repair_requests(), 0);
    assert_eq!(session.run.process_id(), None);

    // The script got exactly the password and the OTP, and stdin was closed after the OTP.
    let record = session.record.values();
    assert_eq!(record["password_eof"], "no");
    assert_eq!(record["password_len"], PASSWORD.len().to_string());
    assert_eq!(record["password_sha256"], sha256(PASSWORD));
    assert_eq!(record["otp_eof"], "no");
    assert_eq!(record["otp_sha256"], sha256(OTP));
    assert_eq!(record["stdin_closed"], "yes");
    assert_eq!(record["term"], "yes");

    session.assert_finished_cleanly();
    let texts = session.texts();
    let started = format!(
        "/bin/sh {} {}›",
        fixture_path("success").display(),
        session.record.path.display()
    );
    assert!(
        texts[0].starts_with("‹attempt started: pid "),
        "{}",
        texts[0]
    );
    assert!(texts[0].ends_with(&started), "{}", texts[0]);
    let order = [
        "‹password sent›",
        "INFO:   Connected to gateway.",
        "‹OTP sent›",
        "INFO:   Authenticated.",
        "INFO:   Tunnel is up and running.",
        "‹sent SIGTERM (reason: userDisconnect)›",
        "INFO:   Restoring routes...",
        "INFO:   Logged out.",
        "‹exited: code 0›",
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
    assert_eq!(texts.last(), Some(&"‹exited: code 0›"));
    assert!(session.has_line(
        OutputSource::Stderr,
        "Thu Oct 15 12:12:05 2026 : Modem hangup"
    ));
    assert!(session.has_line(
        OutputSource::Stdout,
        "add host 203.0.113.10: gateway 192.168.1.1"
    ));
    // The prompt is part of the log, as a line of its own once the answer took the idle period.
    assert!(session.has_text("VPN account password: "));
    // Each milestone follows the line that led to it.
    let before = |milestone: Milestone| {
        session.texts_before(|kind| {
            matches!(kind, HelperEventKind::Milestone { milestone: reached, .. } if *reached == milestone)
        })
    };
    assert!(before(Milestone::ConnectedToGateway).contains(&"INFO:   Connected to gateway."));
    assert!(before(Milestone::Authenticated).contains(&"INFO:   Authenticated."));
    assert_eq!(
        before(Milestone::TunnelUp).last(),
        Some(&"INFO:   Tunnel is up and running.")
    );
}

#[test]
fn certificate_rejected_before_login() {
    let mut session = fixture("cert-rejected", &[], Setup::default());
    let facts = session.wait_for_exit();

    assert_eq!(session.milestones(), [Milestone::Spawned]);
    assert_eq!(
        facts,
        ExitFacts {
            cert_rejected_digests: vec![REJECTED_DIGEST.to_owned()],
            cert_rejected_at: Some(Milestone::Spawned),
            ..client_exit(Milestone::Spawned, 1)
        }
    );
    // The password prompt comes before TLS, so it was answered.
    assert_eq!(
        session.record.value("password_sha256"),
        Some(sha256(PASSWORD))
    );
    assert_eq!(
        session.count_text(&format!("ERROR:      trusted-cert = {REJECTED_DIGEST}")),
        2
    );
    assert!(session.has_text("INFO:   Could not log out."));
    session.assert_finished_cleanly();
}

#[test]
fn password_rejected() {
    let mut session = fixture("auth-failed", &["password"], Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(
        session.milestones(),
        [Milestone::Spawned, Milestone::ConnectedToGateway]
    );
    assert_eq!(facts, client_exit(Milestone::ConnectedToGateway, 1));
    session.assert_finished_cleanly();
}

#[test]
fn otp_rejected() {
    let mut session = fixture("auth-failed", &["otp"], Setup::default().submitting_otp());
    let facts = session.wait_for_exit();
    assert_eq!(session.milestones(), FULL_FLOW[..4]);
    assert_eq!(facts, client_exit(Milestone::OtpSubmitted, 1));
    assert_eq!(session.record.value("otp_sha256"), Some(sha256(OTP)));
    session.assert_finished_cleanly();
}

#[test]
fn gateway_ends_the_session() {
    let mut session = fixture("server-cut", &["cut"], Setup::default().submitting_otp());
    let facts = session.wait_for_exit();

    assert_eq!(session.milestones(), FULL_FLOW);
    assert_eq!(facts.furthest_milestone, Some(Milestone::TunnelUp));
    assert_eq!(facts.stop_initiator, StopInitiator::Client {});
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 1 });
    // The script stays up for 0.3 s.
    let up = facts.up_duration.unwrap();
    assert!(
        up >= Duration::from_millis(150) && up < Duration::from_secs(5),
        "{up:?}"
    );
    assert_eq!(session.repair_requests(), 0);
    assert!(session.has_line(
        OutputSource::Stderr,
        "Thu Oct 15 12:12:05 2026 : Connection terminated."
    ));
    assert!(session.has_line(
        OutputSource::Stdout,
        "ERROR:  pppd: The link was terminated by the modem hanging up."
    ));
    assert_eq!(session.texts().last(), Some(&"‹exited: code 1›"));
    session.assert_finished_cleanly();
}

/// Killed after "Setting new routes...", the client never restores them: the caller is asked once
/// and the outcome follows the exit line.
#[test]
fn crash_after_routes_changed_asks_for_route_repair() {
    for (outcome, text) in [
        (RouteRepairOutcome::Repaired, "‹routes repaired›"),
        (
            RouteRepairOutcome::NotNeeded,
            "‹routes checked: no repair needed›",
        ),
        (
            RouteRepairOutcome::Failed,
            "‹route repair failed — see the Helper log›",
        ),
    ] {
        let setup = Setup {
            route_repair: outcome,
            ..Setup::default().submitting_otp()
        };
        let mut session = fixture("server-cut", &["crash"], setup);
        let facts = session.wait_for_exit();

        assert_eq!(session.repair_requests(), 1);
        assert_eq!(facts.exit_status, ProcessExitStatus::Signaled { signal: 9 });
        assert_eq!(facts.furthest_milestone, Some(Milestone::TunnelUp));
        assert_eq!(facts.stop_initiator, StopInitiator::Client {});
        let texts = session.texts();
        assert_eq!(
            texts[texts.len() - 2..],
            ["‹exited: signal 9 (SIGKILL)›", text]
        );
        session.assert_finished_cleanly();
    }
}

#[test]
fn no_route_repair_when_routes_were_never_changed() {
    let mut session = fixture("auth-failed", &["password"], Setup::default());
    session.wait_for_exit();
    assert_eq!(session.repair_requests(), 0);
    assert!(
        !session
            .texts()
            .iter()
            .any(|text| text.starts_with("‹route"))
    );
}

/// The client closes stdin and exits right after the prompt: the password write fails with EPIPE,
/// which is logged and otherwise taken as the client exiting.
#[test]
fn password_write_after_the_client_closed_stdin() {
    let mut session = fixture("early-exit", &[], Setup::default());
    let facts = session.wait_for_exit();

    assert_eq!(facts, client_exit(Milestone::Spawned, 1));
    assert!(session.has_text("‹input failed: openfortivpn exited before reading it (EPIPE)›"));
    assert!(!session.has_text("‹password sent›"));
    assert_eq!(session.record.value("exited").as_deref(), Some("yes"));
    session.assert_finished_cleanly();
    // A finished run takes no OTP.
    let error = session.run.submit_otp(OTP.to_owned()).unwrap_err();
    assert_eq!(error.code, HelperErrorCode::WrongPhase);
    assert_eq!(error.message, "openfortivpn is not running");
}

#[test]
fn immediate_exit_before_any_prompt() {
    let mut session = script(
        "echo 'ERROR:  Specify a valid host:port couple.'; exit 1",
        Setup::default(),
    );
    let facts = session.wait_for_exit();
    assert_eq!(session.milestones(), [Milestone::Spawned]);
    assert_eq!(facts, client_exit(Milestone::Spawned, 1));
    assert!(session.has_text("ERROR:  Specify a valid host:port couple."));
    session.assert_finished_cleanly();
}

#[test]
fn spawn_failure_is_reported_as_the_exit() {
    let launch = shell(
        "/nonexistent/openfortivpn",
        vec!["-c".to_owned(), "/x".to_owned()],
    );
    let mut session = Session::start(launch, RecordFile::new(), Setup::default());
    let facts = session.wait_for_exit();

    assert_eq!(
        facts,
        ExitFacts {
            furthest_milestone: None,
            stop_initiator: StopInitiator::Client {},
            cert_rejected_digests: Vec::new(),
            cert_rejected_at: None,
            tunnel_mode_rejected: false,
            up_duration: None,
            exit_status: ProcessExitStatus::SpawnFailed { errno: 2 },
        }
    );
    assert!(session.milestones().is_empty());
    assert_eq!(
        session.texts(),
        ["‹spawn failed: /nonexistent/openfortivpn: No such file or directory (errno 2)›"]
    );
    assert_eq!(session.lines()[0].source, OutputSource::Helper);
    assert_eq!(session.run.process_id(), None);
    // Stopping after the end does nothing.
    session.run.stop(StopReason::UserDisconnect);
    session.settle(Duration::from_millis(50));
    assert_eq!(session.events().len(), 1);
}

#[test]
fn spawn_failure_of_a_file_that_is_not_executable() {
    let launch = shell("/dev/null", Vec::new());
    let mut session = Session::start(launch, RecordFile::new(), Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.exit_status,
        ProcessExitStatus::SpawnFailed { errno: 13 }
    );
    assert_eq!(
        session.texts(),
        ["‹spawn failed: /dev/null: Permission denied (errno 13)›"]
    );
}

/// pppd keeps the client's stderr and lives in its own session, so a pipe may stay open after the
/// client exits: output is still collected for the drain period, then the exit is reported anyway
/// and the run closes its end of the pipe.
#[test]
fn pipe_held_open_by_another_process_group() {
    let script_text = "\
set -m
( trap '' PIPE
  /bin/sleep 0.1; echo 'Thu Oct 15 12:12:06 2026 : late pppd line' >&2
  /bin/sleep 1.2; /bin/echo 'after the drain' >&2 || echo 'late_write_failed=yes' >> \"$1\"
  exec /bin/sleep 3 ) </dev/null >/dev/null &
set +m
echo 'INFO:   bye'
exit 0";
    let mut setup = Setup::default();
    setup.timing.pipe_drain = Duration::from_millis(700);
    let started = Instant::now();
    let mut session = script(script_text, setup);
    let facts = session.wait_for_exit();
    let elapsed = started.elapsed();

    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 0 });
    assert!(elapsed >= Duration::from_millis(650), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(2_500), "{elapsed:?}");
    assert!(session.has_line(
        OutputSource::Stderr,
        "Thu Oct 15 12:12:06 2026 : late pppd line"
    ));
    assert!(session.has_text("‹output pipes still open after 0.7s; not waiting any longer›"));
    session.assert_finished_cleanly();
    // Nothing reads the pipe any more, so the holder's next write fails.
    assert!(session.wait(Duration::from_secs(3), |session| {
        session.record.value("late_write_failed").is_some()
    }));
}

/// Output goes out in several batches, by time and before each event, and never after the exit.
#[test]
fn output_is_batched_and_delivered_before_events() {
    let mut session = fixture("server-cut", &["cut"], Setup::default().submitting_otp());
    session.wait_for_exit();

    assert!(session.batches().len() >= FULL_FLOW.len());
    // Every event but the OTP prompt, which comes from the unfinished prompt line, follows at
    // least one new line.
    let mut output_since_event = false;
    for delivered in &session.delivered {
        match delivered {
            Delivered::Output(_) => output_since_event = true,
            Delivered::Event(kind) => {
                let otp_prompt = matches!(
                    kind,
                    HelperEventKind::Milestone {
                        milestone: Milestone::OtpPrompted,
                        ..
                    }
                );
                assert!(
                    otp_prompt || output_since_event,
                    "no output right before {kind:?}"
                );
                output_since_event = false;
            }
            Delivered::RepairRoutes => unreachable!("the routes were restored"),
        }
    }
    let order = [
        "INFO:   Connected to gateway.",
        "INFO:   Authenticated.",
        "INFO:   Setting new routes...",
        "INFO:   Tunnel is up and running.",
        "INFO:   Restoring routes...",
        "INFO:   Logged out.",
    ];
    let stdout: Vec<&str> = session
        .lines()
        .iter()
        .filter(|line| line.source == OutputSource::Stdout && order.contains(&line.text.as_str()))
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(stdout, order);
    session.assert_finished_cleanly();
}

#[test]
fn unfinished_lines_are_shown_at_the_exit() {
    let mut session = script(
        "printf 'INFO:   complete\\nunfinished tail'; printf 'err tail' >&2; exit 0",
        Setup::default(),
    );
    session.wait_for_exit();
    let mut texts: Vec<&str> = session
        .lines()
        .iter()
        .filter(|line| line.source != OutputSource::Helper)
        .map(|line| line.text.as_str())
        .collect();
    texts.sort_unstable();
    assert_eq!(texts, ["INFO:   complete", "err tail", "unfinished tail"]);
    session.assert_finished_cleanly();
}

/// A line without a break, such as a prompt, shows up on its own after the idle period, while
/// complete lines go out with the next batch.
#[test]
fn an_unfinished_line_is_shown_after_a_pause() {
    let mut session = script(
        "echo 'INFO:   first'; printf 'partial tail'; exec /bin/sleep 30",
        Setup::default(),
    );
    assert!(session.wait(Duration::from_secs(5), |session| {
        session.has_text("INFO:   first")
    }));
    assert!(!session.has_text("partial tail"));
    assert!(session.wait(Duration::from_secs(5), |session| {
        session.has_text("partial tail")
    }));
    let lines = session.lines();
    let first = lines
        .iter()
        .find(|line| line.text == "INFO:   first")
        .unwrap();
    let partial = lines
        .iter()
        .find(|line| line.text == "partial tail")
        .unwrap();
    let pause = partial.date.duration_since(first.date).unwrap();
    assert!(pause >= Duration::from_millis(200), "{pause:?}");
    assert_eq!(session.events().len(), 1);

    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.exit_status,
        ProcessExitStatus::Signaled { signal: 15 }
    );
    session.assert_finished_cleanly();
}
