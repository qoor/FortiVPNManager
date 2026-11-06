//! Stage time limits and stopping.

mod common;

use std::thread;
use std::time::Duration;

use common::*;
use fvm_contract::app_helper::{Milestone, ProcessExitStatus, StopInitiator, StopReason};
use fvm_contract::helper_internal::RouteRepairOutcome;
use fvm_watchdog::StageLimits;

const TINY: Duration = Duration::from_millis(150);

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
