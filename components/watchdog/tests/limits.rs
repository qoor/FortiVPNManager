//! Stage time limits and stopping.

mod common;

use std::thread;
use std::time::{Duration, Instant};

use common::*;
use fvm_contract::app_helper::{Milestone, ProcessExitStatus, StopInitiator, StopReason};
use fvm_contract::helper_internal::RouteRepairOutcome;
use fvm_watchdog::StageLimits;
use rustix::process::Pid;

const TINY: Duration = Duration::from_millis(150);

fn wait_for_start(session: &mut Session) {
    assert!(session.wait(Duration::from_secs(5), |session| {
        session.record.value("started").is_some()
    }));
}

/// SIGTERM goes to the client only: a process it started, such as /sbin/route restoring the
/// routes, must be left to finish.
#[test]
fn sigterm_goes_to_the_client_only() {
    let script_text = "\
( trap 'echo leftover_term=yes >> \"$1\"' TERM; while :; do /bin/sleep 0.05; done ) </dev/null >/dev/null 2>&1 &
trap 'echo client_term=yes >> \"$1\"; /bin/sleep 0.3; exit 0' TERM
echo 'started=yes' >> \"$1\"
while :; do /bin/sleep 0.05; done";
    let mut session = script(script_text, Setup::default());
    wait_for_start(&mut session);
    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 0 });
    assert_eq!(session.record.value("client_term").as_deref(), Some("yes"));
    assert_eq!(session.record.value("leftover_term"), None);
}

/// What the client left in its group is killed once it exits, even when it ignores SIGTERM.
#[test]
fn leftovers_in_the_client_group_are_killed_after_it_exits() {
    let script_text = "\
( trap '' TERM; exec /bin/sleep 30 ) </dev/null >/dev/null 2>&1 &
echo \"leftover=$!\" >> \"$1\"
echo 'started=yes' >> \"$1\"
exec /bin/sleep 30";
    let mut session = script(script_text, Setup::default());
    wait_for_start(&mut session);
    let leftover: i32 = session.record.value("leftover").unwrap().parse().unwrap();
    let leftover = Pid::from_raw(leftover).unwrap();
    assert!(rustix::process::test_kill_process(leftover).is_ok());
    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.exit_status,
        ProcessExitStatus::Signaled { signal: 15 }
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while rustix::process::test_kill_process(leftover).is_ok() {
        assert!(Instant::now() < deadline, "the leftover still runs");
        thread::sleep(Duration::from_millis(10));
    }
}

/// A leftover in the client's group that holds stdout is killed with the group, so the exit is
/// reported without waiting for the drain period.
#[test]
fn a_leftover_holding_stdout_does_not_delay_the_exit() {
    let mut setup = Setup::default();
    setup.timing.pipe_drain = Duration::from_secs(2);
    let started = Instant::now();
    let mut session = script("/bin/sleep 30 &\necho 'INFO:   bye'\nexit 0", setup);
    let facts = session.wait_for_exit();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 0 });
    assert!(session.has_text("INFO:   bye"));
    assert!(
        !session
            .texts()
            .iter()
            .any(|text| text.starts_with("‹output pipes still open"))
    );
    session.assert_finished_cleanly();
}

#[test]
fn a_stalled_stage_stops_the_client() {
    type Shrink = fn(&mut StageLimits);
    let stalls: [(&str, &str, Milestone, bool, Shrink); 5] = [
        (
            "spawned",
            "spawnToGateway",
            Milestone::Spawned,
            false,
            |limits| limits.spawn_to_gateway = TINY,
        ),
        (
            "gateway",
            "gatewayToPromptOrAuth",
            Milestone::ConnectedToGateway,
            false,
            |limits| limits.gateway_to_prompt_or_auth = TINY,
        ),
        (
            "otp",
            "otpPromptToSubmit",
            Milestone::OtpPrompted,
            false,
            |limits| limits.otp_prompt_to_submit = TINY,
        ),
        (
            "submitted",
            "otpSubmitToAuth",
            Milestone::OtpSubmitted,
            true,
            |limits| limits.otp_submit_to_auth = TINY,
        ),
        (
            "authenticated",
            "authToTunnel",
            Milestone::Authenticated,
            true,
            |limits| limits.auth_to_tunnel = TINY,
        ),
    ];
    let sessions: Vec<_> = stalls
        .into_iter()
        .map(|(stage, name, furthest, submits_otp, shrink)| {
            thread::spawn(move || {
                let mut setup = Setup::default();
                shrink(&mut setup.timing.stages);
                if submits_otp {
                    setup = setup.submitting_otp();
                }
                let mut session = fixture("hang", &[stage], setup);
                let facts = session.wait_for_exit();

                assert_eq!(
                    facts.stop_initiator,
                    StopInitiator::Helper {
                        reason: StopReason::Watchdog
                    },
                    "{stage}"
                );
                assert_eq!(facts.furthest_milestone, Some(furthest), "{stage}");
                assert_eq!(
                    facts.exit_status,
                    ProcessExitStatus::Signaled { signal: 15 }
                );
                assert_eq!(facts.up_duration, None);
                let texts = session.texts();
                let expired = format!("‹watchdog: {name} did not finish within 0.15s›");
                let fired = texts
                    .iter()
                    .position(|text| *text == expired)
                    .unwrap_or_else(|| panic!("{expired} missing from {texts:?}"));
                assert_eq!(texts[fired + 1], "‹sent SIGTERM (reason: watchdog)›");
                assert_eq!(texts.last(), Some(&"‹exited: signal 15 (SIGTERM)›"));
                session.assert_finished_cleanly();
            })
        })
        .collect();
    for session in sessions {
        session.join().unwrap();
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
    let mut session = fixture("hang", &["tunnel"], setup);
    session.wait_for(Milestone::TunnelUp);
    session.settle(Duration::from_millis(500));
    assert_eq!(session.exit_facts(), None);
    assert!(session.run.process_id().is_some());
    session.run.stop(StopReason::Sleep);
    let facts = session.wait_for_exit();
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
    assert_eq!(facts.furthest_milestone, Some(Milestone::TunnelUp));
}

/// SIGTERM is ignored, so SIGKILL follows after the grace period; the stage limit stays the
/// reason.
#[test]
fn sigkill_follows_an_ignored_sigterm() {
    let mut setup = Setup {
        grace_period: Duration::from_millis(200),
        ..Setup::default()
    };
    setup.timing.stages.spawn_to_gateway = Duration::from_millis(300);
    let mut session = fixture("hang", &["spawned", "ignore-term"], setup);
    let facts = session.wait_for_exit();

    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::Watchdog
        }
    );
    assert_eq!(facts.exit_status, ProcessExitStatus::Signaled { signal: 9 });
    assert!(session.has_text("‹watchdog: spawnToGateway did not finish within 0.3s›"));
    assert!(session.has_text("‹sent SIGTERM (reason: watchdog)›"));
    assert!(session.has_text("‹sent SIGKILL: still running 0.2s after SIGTERM›"));
    assert_eq!(
        session.texts().last(),
        Some(&"‹exited: signal 9 (SIGKILL)›")
    );
    session.assert_finished_cleanly();
}

/// A user stop that needs SIGKILL keeps the user as the reason, a second stop changes nothing, and
/// the routes left changed are repaired.
#[test]
fn sigkill_after_a_user_stop_on_an_up_tunnel() {
    let setup = Setup {
        grace_period: Duration::from_millis(200),
        route_repair: RouteRepairOutcome::Repaired,
        ..Setup::default().submitting_otp()
    };
    let mut session = fixture("hang", &["tunnel", "ignore-term"], setup);
    session.wait_for(Milestone::TunnelUp);
    session.run.stop(StopReason::UserDisconnect);
    session.run.stop(StopReason::AppQuit);
    let facts = session.wait_for_exit();

    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(facts.exit_status, ProcessExitStatus::Signaled { signal: 9 });
    assert_eq!(session.texts().last(), Some(&"‹routes repaired›"));
    assert_eq!(session.repair_requests(), 1);
    assert_eq!(
        session
            .texts()
            .iter()
            .filter(|text| text.starts_with("‹sent SIGTERM"))
            .count(),
        1
    );
    assert!(session.has_text("‹sent SIGKILL: still running 0.2s after SIGTERM›"));
    session.assert_finished_cleanly();
}

#[test]
fn up_duration_runs_from_tunnel_up_to_the_exit() {
    let mut session = fixture("success", &[], Setup::default().submitting_otp());
    session.wait_for(Milestone::TunnelUp);
    thread::sleep(Duration::from_millis(400));
    session.run.stop(StopReason::ProactiveReconnect);
    let facts = session.wait_for_exit();

    let up = facts.up_duration.unwrap();
    assert!(
        up >= Duration::from_millis(400) && up < Duration::from_secs(3),
        "{up:?}"
    );
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::ProactiveReconnect
        }
    );
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 0 });
    session.assert_finished_cleanly();
}

/// The Log tab shows each stop reason as the app sends it.
#[test]
fn stop_reasons_are_shown_as_the_app_sends_them() {
    for (reason, raw) in [
        (StopReason::AppDisconnected, "clientInvalidated"),
        (StopReason::HelperShutdown, "helperShutdown"),
    ] {
        let mut session = fixture("hang", &["spawned"], Setup::default());
        session.wait_for(Milestone::Spawned);
        session.run.stop(reason);
        session.wait_for_exit();
        assert!(session.has_text(&format!("‹sent SIGTERM (reason: {raw})›")));
    }
}
