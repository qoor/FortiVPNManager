//! Disconnect, quit and reconnect now.

use fvm_contract::app_helper::{Milestone, StopReason};
use fvm_contract::app_internal::ButtonState;
use fvm_contract::app_internal::section::ConnectionIcon;

use super::super::machine::{AfterStop, AttemptReason, Phase, RetryReason};
use super::super::{Effect, Input, TimerKind};
use super::harness::*;

fn reason(h: &Harness) -> Option<AttemptReason> {
    h.attempt().map(|attempt| attempt.reason)
}

fn after(h: &Harness) -> Option<AfterStop> {
    h.stopping().map(|stop| stop.after)
}

#[test]
fn a_disconnect_while_connected_finishes_on_the_exit() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    assert_eq!(
        connection_effects(&h.send(Input::Disconnect)),
        [
            Effect::StopRun {
                attempt: id,
                reason: StopReason::UserDisconnect
            },
            Effect::CancelTimer(TimerKind::ProactiveReconnect),
        ]
    );
    assert!(!h.cm.wants_connection);
    assert_eq!(after(&h), Some(AfterStop::Idle));
    let section = h.cm.connection_section();
    assert_eq!(section.icon, ConnectionIcon::InProgress);
    assert_eq!(section.connect, ButtonState::Enabled);
    assert_eq!(section.disconnect, ButtonState::Hidden);
    assert_ne!(section.reconnect_now, ButtonState::Enabled);
    h.advance(2.0);
    assert_eq!(
        connection_effects(
            &h.exit(
                ended(Milestone::TunnelUp)
                    .by_helper(StopReason::UserDisconnect)
                    .up(2.0)
            )
        ),
        [Effect::HoldActivity(false)]
    );
    assert_eq!(h.cm.phase, Phase::Idle);
    assert_eq!(h.cm.connection_section().icon, ConnectionIcon::Disconnected);
}

#[test]
fn a_disconnect_while_the_tunnel_is_starting() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.milestone(Milestone::Spawned);
    assert_eq!(
        connection_effects(&h.send(Input::Disconnect)),
        [
            Effect::StopRun {
                attempt: id.clone(),
                reason: StopReason::UserDisconnect
            },
            Effect::CloseOtpSession {
                attempt: id,
                check_spam: false
            },
            Effect::CancelTimer(TimerKind::AttemptDeadline),
        ]
    );
    h.exit(ended(Milestone::Spawned).by_helper(StopReason::UserDisconnect));
    assert_eq!(h.cm.phase, Phase::Idle);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
}

#[test]
fn a_disconnect_before_the_tunnel_started_is_immediate() {
    let mut h = Harness::new();
    h.launch();
    assert_eq!(
        connection_effects(&h.send(Input::Disconnect)),
        [
            Effect::CancelTimer(TimerKind::AttemptDeadline),
            Effect::HoldActivity(false)
        ]
    );
    assert_eq!(h.cm.phase, Phase::Idle);
    // The late readiness answer starts nothing.
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(ready()))),
        []
    );
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn a_disconnect_while_waiting_cancels_the_wait() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.exit(ended(Milestone::Spawned));
    assert_eq!(
        connection_effects(&h.send(Input::Disconnect)),
        [
            Effect::CancelTimer(TimerKind::Retry),
            Effect::HoldActivity(false)
        ]
    );
    assert_eq!(h.cm.phase, Phase::Idle);

    let mut offline_harness = Harness::with(settings(), Some(offline()));
    offline_harness.launch();
    assert!(offline_harness.network_wait().is_some());
    assert_eq!(
        connection_effects(&offline_harness.send(Input::Disconnect)),
        [Effect::HoldActivity(false)]
    );
    assert_eq!(offline_harness.cm.phase, Phase::Idle);
}

#[test]
fn a_connect_during_a_user_disconnect_reconnects_after_the_exit() {
    let mut h = Harness::new();
    h.launch_connected();
    h.send(Input::Disconnect);
    // The user action clears the rolling history this session recorded.
    assert_eq!(
        connection_effects(&h.send(Input::Connect)),
        [Effect::SaveLedger(empty_ledger())]
    );
    assert_eq!(after(&h), Some(AfterStop::Reconnect(AttemptReason::User)));
    assert!(h.cm.wants_connection);
    h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::UserDisconnect)
            .up(30.0),
    );
    assert_eq!(reason(&h), Some(AttemptReason::User));
}

#[test]
fn a_disconnect_during_a_planned_reconnect_ends_idle() {
    let mut h = Harness::new();
    h.launch_connected();
    h.fire(TimerKind::ProactiveReconnect);
    h.send(Input::Disconnect);
    assert_eq!(after(&h), Some(AfterStop::Idle));
    h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::ProactiveReconnect)
            .up(10_500.0),
    );
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn a_quit_while_idle_is_approved_at_once() {
    let mut h = Harness::new();
    h.launch_with(false, None);
    assert_eq!(
        connection_effects(&h.send(Input::QuitRequested)),
        [Effect::ApproveQuit]
    );
    assert_eq!(connection_effects(&h.send(Input::Connect)), []);
}

#[test]
fn a_quit_while_connected_stops_then_is_approved_on_the_exit() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    let quit = connection_effects(&h.send(Input::QuitRequested));
    assert_eq!(
        quit,
        [
            Effect::StopRun {
                attempt: id,
                reason: StopReason::AppQuit
            },
            Effect::CancelTimer(TimerKind::ProactiveReconnect),
            h.set(TimerKind::QuitDeadline, h.now + secs(12.0)),
        ]
    );
    assert!(!h.cm.wants_connection);
    h.advance(1.0);
    assert_eq!(
        connection_effects(
            &h.exit(
                ended(Milestone::TunnelUp)
                    .by_helper(StopReason::AppQuit)
                    .up(20.0)
            )
        ),
        [
            Effect::CancelTimer(TimerKind::QuitDeadline),
            Effect::HoldActivity(false),
            Effect::ApproveQuit
        ]
    );
    assert_eq!(
        connection_effects(&h.exit(ended(Milestone::TunnelUp))),
        [],
        "nothing after the quit"
    );
}

#[test]
fn the_quit_deadline_approves_without_the_exit() {
    let mut h = Harness::new();
    h.launch_connected();
    h.send(Input::QuitRequested);
    assert_eq!(
        connection_effects(&h.fire(TimerKind::QuitDeadline)),
        [Effect::HoldActivity(false), Effect::ApproveQuit]
    );
    assert_eq!(h.now, t0() + secs(12.0 + 12.0));
}

#[test]
fn a_quit_while_the_gmail_session_starts_closes_it_and_is_approved() {
    let mut h = Harness::new();
    h.launch();
    h.send(Input::ReadinessChecked(ready()));
    assert_eq!(
        connection_effects(&h.send(Input::QuitRequested)),
        [
            Effect::CloseOtpSession {
                attempt: id(1),
                check_spam: false
            },
            Effect::CancelTimer(TimerKind::AttemptDeadline),
            Effect::HoldActivity(false),
            Effect::ApproveQuit,
        ]
    );
}

#[test]
fn a_quit_while_waiting_is_approved_at_once() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.exit(ended(Milestone::Spawned));
    assert_eq!(
        connection_effects(&h.send(Input::QuitRequested)),
        [
            Effect::CancelTimer(TimerKind::Retry),
            Effect::HoldActivity(false),
            Effect::ApproveQuit
        ]
    );
}

#[test]
fn a_quit_during_a_stop_in_flight_does_not_stop_twice() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    h.fire(TimerKind::OtpDeadline);
    let quit = h.send(Input::QuitRequested);
    assert!(stop_reasons(&quit).is_empty());
    assert_eq!(
        timer_set(&quit, TimerKind::QuitDeadline),
        Some(h.now + secs(12.0))
    );
    assert_eq!(after(&h), Some(AfterStop::Quit));
    let exit = connection_effects(
        &h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::OtpTimeout)),
    );
    assert_eq!(exit.last(), Some(&Effect::ApproveQuit));
    assert_eq!(saved_ledger(&exit), None);
}

#[test]
fn a_quit_during_a_user_disconnect_waits_for_the_same_exit() {
    let mut h = Harness::new();
    h.launch_connected();
    h.send(Input::Disconnect);
    let quit = h.send(Input::QuitRequested);
    assert!(stop_reasons(&quit).is_empty());
    assert_eq!(after(&h), Some(AfterStop::Quit));
    // A disconnect click during the quit cannot cancel it.
    h.send(Input::Disconnect);
    assert_eq!(after(&h), Some(AfterStop::Quit));
    let exit = connection_effects(
        &h.exit(
            ended(Milestone::TunnelUp)
                .by_helper(StopReason::UserDisconnect)
                .up(30.0),
        ),
    );
    assert_eq!(exit.last(), Some(&Effect::ApproveQuit));
}

#[test]
fn reconnect_now_while_connected_is_break_before_make() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    assert_eq!(
        h.cm.connection_section().reconnect_now,
        ButtonState::Enabled
    );
    assert_eq!(
        connection_effects(&h.send(Input::ReconnectNow)),
        [
            Effect::StopRun {
                attempt: id,
                reason: StopReason::Reconnect
            },
            Effect::CancelTimer(TimerKind::ProactiveReconnect),
            Effect::SaveLedger(empty_ledger()),
        ]
    );
    h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::Reconnect)
            .up(50.0),
    );
    assert_eq!(reason(&h), Some(AttemptReason::ReconnectNow));
}

#[test]
fn reconnect_now_soon_after_an_auth_failure_waits_until_ninety_seconds() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::ConnectedToGateway);
    h.exit(ended(Milestone::ConnectedToGateway));
    let failed_at = h.now;
    h.advance(20.0);
    let effects = h.send(Input::ReconnectNow);
    assert_eq!(
        h.retry_wait(),
        Some((failed_at + secs(90.0), RetryReason::AuthCooldown))
    );
    assert_eq!(saved_ledger(&effects).unwrap().consecutive_auth_failures, 0);
    assert!(!effects.contains(&Effect::CheckReadiness));
    h.fire(TimerKind::Retry);
    assert_eq!(h.now, failed_at + secs(90.0));
    assert!(h.attempt().is_some());
}

#[test]
fn a_reconnect_now_cooldown_delivered_slightly_early_still_starts_the_attempt() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::ConnectedToGateway);
    h.exit(ended(Milestone::ConnectedToGateway));
    let failed_at = h.now;
    h.advance(20.0);
    h.send(Input::ReconnectNow);
    assert_eq!(
        h.retry_wait(),
        Some((failed_at + secs(90.0), RetryReason::AuthCooldown))
    );
    let effects = h.fire_early(TimerKind::Retry, 0.001);
    assert!(effects.contains(&Effect::CheckReadiness));
    assert_eq!(reason(&h), Some(retry(RetryReason::AuthCooldown)));
}

#[test]
fn reconnect_now_after_the_gap_starts_at_once() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.exit(ended(Milestone::Spawned));
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::Network)
    );
    assert!(
        h.send(Input::ReconnectNow)
            .contains(&Effect::CheckReadiness)
    );
    assert_eq!(reason(&h), Some(AttemptReason::ReconnectNow));
}

#[test]
fn reconnect_now_while_connecting_is_ignored() {
    let mut h = Harness::new();
    h.launch();
    assert_eq!(connection_effects(&h.send(Input::ReconnectNow)), []);
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    let before = snapshot(&h);
    assert_eq!(connection_effects(&h.send(Input::ReconnectNow)), []);
    assert_eq!(snapshot(&h), before);
    assert_ne!(
        h.cm.connection_section().reconnect_now,
        ButtonState::Enabled
    );
}

#[test]
fn reconnect_now_is_ignored_when_nothing_is_wanted() {
    let mut h = Harness::new();
    h.launch_with(false, None);
    assert_eq!(connection_effects(&h.send(Input::ReconnectNow)), []);
    h.send(Input::ReadinessChecked(super::super::ReadinessFacts {
        vpn_password_stored: false,
        ..ready()
    }));
    assert_eq!(connection_effects(&h.send(Input::ReconnectNow)), []);
    let mut stopping = Harness::new();
    stopping.launch_connected();
    stopping.send(Input::Disconnect);
    assert_eq!(connection_effects(&stopping.send(Input::ReconnectNow)), []);
}

#[test]
fn the_proactive_timer_reconnects_right_after_the_exit() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    let planned = h.connected().unwrap().planned_reconnect.unwrap();
    assert_eq!(
        connection_effects(&h.fire(TimerKind::ProactiveReconnect)),
        [Effect::StopRun {
            attempt: id,
            reason: StopReason::ProactiveReconnect
        }]
    );
    assert_eq!(h.now, planned);
    assert_eq!(
        after(&h),
        Some(AfterStop::Reconnect(AttemptReason::Proactive))
    );
    h.advance(1.0);
    let effects = h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::ProactiveReconnect)
            .up(10_500.0),
    );
    assert!(effects.contains(&Effect::CheckReadiness));
    assert_eq!(reason(&h), Some(AttemptReason::Proactive));
}

#[test]
fn the_proactive_timer_firing_first_then_a_server_cut_gives_one_reconnect() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    h.fire(TimerKind::ProactiveReconnect);
    // The server cut the session before our SIGTERM landed.
    let effects = h.exit(ended(Milestone::TunnelUp).up(10_500.0));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| **effect == Effect::CheckReadiness)
            .count(),
        1
    );
    assert_eq!(reason(&h), Some(AttemptReason::Proactive));
    assert_eq!(
        connection_effects(&h.exit_for(&id, ended(Milestone::TunnelUp).up(10_500.0))),
        [],
        "a repeated exit is stale"
    );
}

#[test]
fn a_server_cut_first_makes_the_proactive_timer_stale() {
    let mut h = Harness::new();
    h.launch_connected();
    let proactive = h.timer_id(TimerKind::ProactiveReconnect);
    h.advance(10_499.0);
    h.exit(ended(Milestone::TunnelUp).up(10_499.0));
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::ServerSessionEnded)
    );
    h.advance(1.0);
    assert_eq!(
        connection_effects(&h.send(Input::TimerFired(proactive))),
        []
    );
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::ServerSessionEnded)
    );
}

#[test]
fn a_settings_change_recomputes_the_planned_reconnect() {
    let mut h = Harness::new();
    h.launch_connected();
    let since = h.connected().unwrap().since;
    let mut settings = settings();
    settings.connection.session_max_minutes = 60;
    let shorter = h.send(Input::SettingsChanged(settings.clone()));
    assert_eq!(
        timer_set(&shorter, TimerKind::ProactiveReconnect),
        Some(since + secs(55.0 * 60.0))
    );
    assert_eq!(
        h.connected().unwrap().planned_reconnect,
        Some(since + secs(55.0 * 60.0))
    );
    h.advance(400.0);
    settings.connection.session_max_minutes = 10;
    let past = h.send(Input::SettingsChanged(settings.clone()));
    assert_eq!(
        timer_set(&past, TimerKind::ProactiveReconnect),
        Some(h.now + secs(60.0)),
        "at least a minute from now"
    );
    settings.connection.proactive_reconnect = false;
    assert_eq!(
        connection_effects(&h.send(Input::SettingsChanged(settings.clone()))),
        [Effect::CancelTimer(TimerKind::ProactiveReconnect)]
    );
    assert_eq!(h.connected().unwrap().planned_reconnect, None);
    settings.connection.reconnect_after_sleep = true;
    assert_eq!(
        connection_effects(&h.send(Input::SettingsChanged(settings))),
        []
    );
}

#[test]
fn a_settings_change_while_not_connected_applies_to_the_next_session() {
    let mut h = Harness::new();
    h.launch_with(false, None);
    let mut settings = settings();
    settings.connection.session_max_minutes = 30;
    assert_eq!(
        connection_effects(&h.send(Input::SettingsChanged(settings))),
        []
    );
    h.send(Input::Connect);
    h.run_to_connected();
    let connected = h.connected().unwrap();
    assert_eq!(
        connected.planned_reconnect,
        Some(connected.since + secs(25.0 * 60.0))
    );
}

// MARK: Stale inputs

/// What an ignored input must leave as it was.
fn snapshot(
    h: &Harness,
) -> (
    Phase,
    super::super::FailureLedger,
    Vec<(TimerKind, Option<std::time::SystemTime>)>,
) {
    let timers = h
        .connection_timers()
        .into_iter()
        .map(|kind| (kind, h.timer(kind)))
        .collect();
    (h.cm.phase.clone(), h.cm.ledger.clone(), timers)
}

#[test]
fn helper_events_of_other_attempts_are_ignored() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    let before = snapshot(&h);
    assert_eq!(
        connection_effects(&h.milestone_for(&stranger(), Milestone::TunnelUp, Some(TUNNEL_IP))),
        []
    );
    assert_eq!(
        connection_effects(&h.exit_for(&stranger(), ended(Milestone::Spawned))),
        []
    );
    assert_eq!(snapshot(&h), before);
    h.run_milestones(&id, Milestone::TunnelUp);
    assert!(h.connected().is_some());
    let connected = snapshot(&h);
    assert_eq!(
        connection_effects(&h.exit_for(&stranger(), ended(Milestone::TunnelUp).up(5.0))),
        []
    );
    assert_eq!(
        connection_effects(&h.milestone_for(&stranger(), Milestone::OtpPrompted, None)),
        []
    );
    assert_eq!(snapshot(&h), connected);
}

#[test]
fn the_exit_of_an_old_attempt_while_waiting_is_ignored() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.exit(ended(Milestone::Spawned));
    let waiting = snapshot(&h);
    assert_eq!(
        connection_effects(&h.exit_for(&id, ended(Milestone::Spawned))),
        []
    );
    assert_eq!(
        connection_effects(&h.exit_for(&id, ended(Milestone::TunnelUp).up(9_999.0))),
        []
    );
    let late = Input::RunRejected(
        id,
        fvm_contract::app_helper::HelperErrorCode::ClientUnavailable,
    );
    assert_eq!(connection_effects(&h.send(late)), []);
    assert_eq!(snapshot(&h), waiting);
}

#[test]
fn helper_events_before_the_run_was_requested_are_ignored() {
    let mut h = Harness::new();
    h.launch();
    let id = h.attempt().unwrap().id.clone();
    assert_eq!(
        connection_effects(&h.milestone_for(&id, Milestone::Spawned, None)),
        []
    );
    h.send(Input::ReadinessChecked(ready()));
    assert_eq!(
        connection_effects(&h.exit_for(&id, ended(Milestone::Spawned))),
        []
    );
    assert_eq!(
        h.attempt().unwrap().stage,
        super::super::machine::Stage::PreparingOtp
    );
}

#[test]
fn inputs_for_the_attempt_before_a_certificate_retry_are_ignored() {
    let mut h = Harness::new();
    h.launch();
    let first = h.start_tunnel();
    h.run_milestones(&first, Milestone::Spawned);
    let old_deadline = h.timer_id(TimerKind::AttemptDeadline);
    h.exit(ended(Milestone::Spawned).certs(&[digest('a')], Some(Milestone::Spawned)));
    h.send(Input::ReadinessChecked(ready()));
    let retry_state = snapshot(&h);
    for input in [
        Input::TimerFired(old_deadline),
        Input::OtpSessionReady(first.clone()),
        Input::OtpSessionFailed(first.clone(), super::super::OtpFailure::Authentication),
        Input::OtpCodeFound(
            first.clone(),
            fvm_contract::app_internal::SecretText("123456".into()),
        ),
        Input::RunRejected(
            first.clone(),
            fvm_contract::app_helper::HelperErrorCode::Busy,
        ),
    ] {
        let label = format!("{input:?}");
        assert_eq!(connection_effects(&h.send(input)), [], "{label}");
    }
    assert_eq!(
        connection_effects(&h.milestone_for(&first, Milestone::OtpPrompted, None)),
        []
    );
    assert_eq!(snapshot(&h), retry_state);
}

#[test]
fn timers_fire_once_per_generation() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.exit(ended(Milestone::Spawned));
    let timer = h.timer_id(TimerKind::Retry);
    h.fire(TimerKind::Retry);
    let fired = snapshot(&h);
    assert_eq!(connection_effects(&h.send(Input::TimerFired(timer))), []);
    let unknown = super::super::TimerId {
        kind: TimerKind::AttemptDeadline,
        generation: 999,
    };
    assert_eq!(connection_effects(&h.send(Input::TimerFired(unknown))), []);
    assert_eq!(snapshot(&h), fired);
}
