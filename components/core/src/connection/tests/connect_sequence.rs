//! One attempt from start to tunnel (SPEC F-CM-01, F-CM-02).

use fvm_contract::app_helper::{Milestone, StopReason};
use fvm_contract::app_internal::SecretText;
use fvm_contract::app_internal::section::ConnectionIcon;

use super::super::machine::{AttemptReason, Phase, RetryReason, Stage};
use super::super::{Effect, Failure, FailureLedger, Input, TimerKind};
use super::harness::*;

fn code(text: &str) -> SecretText {
    SecretText(text.to_owned())
}

#[test]
fn launch_runs_the_whole_sequence_in_order() {
    let mut h = Harness::new();
    let t0 = h.now;
    let id1 = id(1);
    let effects = connection_effects(&h.launch());
    assert_eq!(
        effects,
        [
            Effect::CheckReadiness,
            h.set(TimerKind::AttemptDeadline, t0 + secs(150.0)),
            Effect::HoldActivity(true)
        ]
    );
    assert_eq!(h.attempt().unwrap().stage, Stage::CheckingReadiness);
    assert!(h.cm.wants_connection);
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(ready()))),
        [Effect::OpenOtpSession {
            attempt: id1.clone()
        }]
    );
    assert_eq!(h.attempt().unwrap().stage, Stage::PreparingOtp);
    assert_eq!(
        connection_effects(&h.send(Input::OtpSessionReady(id1.clone()))),
        [Effect::StartRun {
            attempt: id1.clone(),
            account: account()
        }]
    );
    assert_eq!(h.attempt().unwrap().stage, Stage::Run);
    h.advance(1.0);
    assert_eq!(connection_effects(&h.milestone(Milestone::Spawned)), []);
    h.advance(1.0);
    assert_eq!(
        connection_effects(&h.milestone(Milestone::ConnectedToGateway)),
        [Effect::SaveLedger(FailureLedger {
            recent_auth_attempts: vec![t0 + secs(2.0)],
            ..empty_ledger()
        })]
    );
    h.advance(1.0);
    let effects = connection_effects(&h.milestone(Milestone::OtpPrompted));
    assert_eq!(
        effects,
        [
            Effect::SearchOtp {
                attempt: id1.clone(),
                prompted_at: t0 + secs(3.0)
            },
            h.set(TimerKind::OtpDeadline, t0 + secs(53.0)),
        ]
    );
    h.advance(5.0);
    assert_eq!(
        connection_effects(&h.send(Input::OtpCodeFound(id1.clone(), code("123456")))),
        [
            Effect::SubmitOtp {
                attempt: id1.clone(),
                code: code("123456")
            },
            Effect::CancelTimer(TimerKind::OtpDeadline)
        ]
    );
    h.advance(1.0);
    assert_eq!(
        connection_effects(&h.milestone(Milestone::OtpSubmitted)),
        []
    );
    h.advance(1.0);
    assert_eq!(
        connection_effects(&h.milestone(Milestone::Authenticated)),
        []
    );
    h.advance(2.0);
    let planned = t0 + secs(12.0 + (180.0 - 5.0) * 60.0);
    let effects = connection_effects(&h.milestone_for(&id1, Milestone::TunnelUp, Some(TUNNEL_IP)));
    assert_eq!(
        effects,
        [
            Effect::CancelTimer(TimerKind::AttemptDeadline),
            h.set(TimerKind::ProactiveReconnect, planned)
        ]
    );
    let connected = h.connected().unwrap();
    assert_eq!(connected.attempt, id1);
    assert_eq!(connected.since, t0 + secs(12.0));
    assert_eq!(connected.tunnel_ip, Some(TUNNEL_IP));
    assert_eq!(connected.planned_reconnect, Some(planned));
    assert_eq!(h.cm.connection_section().icon, ConnectionIcon::Connected);
}

#[test]
fn proactive_reconnect_disabled_sets_no_timer() {
    let mut settings = settings();
    settings.connection.proactive_reconnect = false;
    let mut h = Harness::with_settings(settings);
    h.launch_connected();
    assert_eq!(timer_set(&h.effects, TimerKind::ProactiveReconnect), None);
    assert_eq!(h.connected().unwrap().planned_reconnect, None);
    assert_eq!(h.timer(TimerKind::ProactiveReconnect), None);
}

#[test]
fn custom_session_settings_drive_the_planned_reconnect() {
    let mut settings = settings();
    settings.connection.session_max_minutes = 60;
    settings.connection.proactive_margin_minutes = 10;
    let mut h = Harness::with_settings(settings);
    h.launch_connected();
    let connected = h.connected().unwrap();
    assert_eq!(
        connected.planned_reconnect,
        Some(connected.since + secs(50.0 * 60.0))
    );
}

#[test]
fn connecting_from_idle_needs_a_user_action() {
    let mut h = Harness::new();
    assert_eq!(
        connection_effects(&h.launch_with(false, None)),
        [Effect::CheckReadiness]
    );
    assert_eq!(h.cm.phase, Phase::Idle);
    assert!(!h.cm.wants_connection);
    // An unsolicited "ready" starts nothing while the user wants to stay disconnected.
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(ready()))),
        []
    );
    assert_eq!(h.cm.phase, Phase::Idle);
    let effects = connection_effects(&h.send(Input::Connect));
    assert_eq!(effects.first(), Some(&Effect::CheckReadiness));
    assert!(effects.contains(&Effect::HoldActivity(true)));
    assert_eq!(h.attempt().unwrap().reason, AttemptReason::User);
    h.run_to_connected();
}

#[test]
fn the_otp_deadline_stops_without_submitting_and_counts_once() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    let prompted_at = h.now;
    let effects = connection_effects(&h.fire(TimerKind::OtpDeadline));
    assert_eq!(h.now, prompted_at + secs(50.0));
    assert_eq!(
        effects,
        [
            Effect::StopRun {
                attempt: id.clone(),
                reason: StopReason::OtpTimeout
            },
            Effect::CloseOtpSession {
                attempt: id.clone(),
                check_spam: true
            },
            Effect::CancelTimer(TimerKind::AttemptDeadline),
        ]
    );
    // A code arriving after the deadline is not submitted.
    assert_eq!(
        connection_effects(&h.send(Input::OtpCodeFound(id.clone(), code("654321")))),
        []
    );
    h.advance(1.0);
    let exit = h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::OtpTimeout));
    let (until, reason) = h.retry_wait().unwrap();
    assert_eq!(
        reason,
        RetryReason::AuthFailure(Failure::OtpNotReceived { in_spam: false })
    );
    assert_eq!(until, h.now + secs(90.0));
    assert_eq!(timer_set(&exit, TimerKind::Retry), Some(h.now + secs(90.0)));
    let ledger = saved_ledger(&exit).unwrap();
    assert_eq!(ledger.consecutive_auth_failures, 1);
    assert_eq!(ledger.last_auth_failure_at, Some(h.now));
}

#[test]
fn the_spam_hint_upgrades_the_reason_before_or_after_the_exit() {
    // Reported before the exit.
    let mut before = Harness::new();
    before.launch();
    let id = before.start_tunnel();
    before.run_milestones(&id, Milestone::OtpPrompted);
    before.fire(TimerKind::OtpDeadline);
    assert_eq!(
        connection_effects(&before.send(Input::OtpFoundInSpam(id))),
        []
    );
    before.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::OtpTimeout));
    let in_spam = Failure::OtpNotReceived { in_spam: true };
    assert_eq!(
        before.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::AuthFailure(in_spam))
    );
    assert_eq!(before.cm.ledger.last_auth_failure, Some(in_spam));

    // Reported after the exit.
    let mut after = Harness::new();
    after.launch();
    let id2 = after.start_tunnel();
    after.run_milestones(&id2, Milestone::OtpPrompted);
    after.fire(TimerKind::OtpDeadline);
    after.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::OtpTimeout));
    let effects = after.send(Input::OtpFoundInSpam(id2));
    assert_eq!(
        after.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::AuthFailure(in_spam))
    );
    assert_eq!(
        saved_ledger(&effects).unwrap().last_auth_failure,
        Some(in_spam)
    );
    // A hint for an unrelated attempt changes nothing.
    assert_eq!(
        connection_effects(&after.send(Input::OtpFoundInSpam(stranger()))),
        []
    );
}

#[test]
fn the_attempt_deadline_stops_a_running_tunnel() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.advance(1.0);
    h.milestone(Milestone::Spawned);
    let effects = connection_effects(&h.fire(TimerKind::AttemptDeadline));
    assert_eq!(h.now, t0() + secs(150.0));
    assert_eq!(
        effects,
        [
            Effect::StopRun {
                attempt: id.clone(),
                reason: StopReason::AttemptTimeout
            },
            Effect::CloseOtpSession {
                attempt: id,
                check_spam: false
            },
        ]
    );
    h.exit(ended(Milestone::Spawned).by_helper(StopReason::AttemptTimeout));
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(10.0), RetryReason::Network))
    );
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
}

#[test]
fn the_attempt_deadline_while_the_gmail_login_hangs_is_a_network_error() {
    let mut h = Harness::new();
    h.launch();
    h.send(Input::ReadinessChecked(ready()));
    let id = h.attempt().unwrap().id.clone();
    let effects = connection_effects(&h.fire(TimerKind::AttemptDeadline));
    assert_eq!(
        effects.first(),
        Some(&Effect::CloseOtpSession {
            attempt: id.clone(),
            check_spam: false
        })
    );
    assert!(stop_reasons(&effects).is_empty());
    assert!(run_starts(&effects).is_empty());
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::Network)
    );
    // The late session result is stale now.
    assert_eq!(connection_effects(&h.send(Input::OtpSessionReady(id))), []);
}

#[test]
fn a_gateway_without_otp_closes_the_session_at_tunnel_up() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.milestone(Milestone::Spawned);
    h.milestone(Milestone::ConnectedToGateway);
    h.milestone(Milestone::Authenticated);
    let effects = connection_effects(&h.milestone_for(&id, Milestone::TunnelUp, Some(TUNNEL_IP)));
    assert_eq!(
        effects.first(),
        Some(&Effect::CloseOtpSession {
            attempt: id,
            check_spam: false
        })
    );
    assert!(h.connected().is_some());
}

#[test]
fn a_code_is_submitted_once() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    assert!(
        h.send(Input::OtpCodeFound(id.clone(), code("123456")))
            .contains(&Effect::SubmitOtp {
                attempt: id.clone(),
                code: code("123456")
            })
    );
    assert_eq!(
        connection_effects(&h.send(Input::OtpCodeFound(id, code("654321")))),
        []
    );
}

#[test]
fn a_code_before_the_prompt_is_ignored() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.milestone(Milestone::Spawned);
    h.milestone(Milestone::ConnectedToGateway);
    assert_eq!(
        connection_effects(&h.send(Input::OtpCodeFound(id, code("123456")))),
        []
    );
}

#[test]
fn out_of_order_and_repeated_milestones_are_ignored() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    assert_eq!(connection_effects(&h.milestone(Milestone::OtpPrompted)), []);
    assert_eq!(connection_effects(&h.milestone(Milestone::Spawned)), []);
    assert_eq!(h.attempt().unwrap().milestone, Some(Milestone::OtpPrompted));
}

#[test]
fn the_auth_attempt_is_recorded_once_per_attempt_even_when_milestones_skip() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.milestone(Milestone::Spawned);
    // `connectedToGateway` was never reported; `otpPrompted` implies it.
    let effects = h.milestone(Milestone::OtpPrompted);
    assert_eq!(
        saved_ledger(&effects).unwrap().recent_auth_attempts,
        [h.now]
    );
    h.milestone(Milestone::OtpSubmitted);
    assert_eq!(h.cm.ledger.recent_auth_attempts.len(), 1);
}

#[test]
fn attempt_ids_are_deterministic_and_fresh() {
    let mut h = Harness::new();
    h.launch();
    assert_eq!(h.attempt().unwrap().id, id(1));
    h.start_tunnel();
    h.exit(ended(Milestone::Spawned));
    h.fire(TimerKind::Retry);
    assert_eq!(h.attempt().unwrap().id, id(2));
    assert_ne!(id(1), id(2));
}
