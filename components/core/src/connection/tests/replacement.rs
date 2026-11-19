use std::net::Ipv4Addr;

use fvm_contract::app_helper::{AttemptId, HelperErrorCode, Milestone, StopReason};
use fvm_contract::app_internal::ButtonState;
use fvm_contract::app_internal::section::{
    ConnectProgress, ConnectionIcon, ConnectionStatus, ProactiveReconnect,
};

use super::super::machine::{
    AfterStop, AttemptReason, Phase, Renewal, RetryReason, Session, Stage,
};
use super::super::{Effect, Failure, FailureLedger, Input, OtpFailure, Settings, TimerKind};
use super::harness::*;

const NEXT_TUNNEL_IP: Ipv4Addr = Ipv4Addr::new(10, 212, 134, 9);

fn connected_with(settings: Settings) -> (Harness, Session) {
    let mut h = Harness::with_settings(settings);
    h.launch_connected();
    let session = h.connected().unwrap().clone();
    (h, session)
}

fn connected() -> (Harness, Session) {
    connected_with(settings())
}

fn replaced(h: &Harness) -> Option<&Session> {
    h.attempt().and_then(|attempt| attempt.replacing.as_ref())
}

fn renewal(h: &Harness) -> Option<Renewal> {
    h.connected()
        .or_else(|| replaced(h))
        .and_then(|session| session.renewal)
}

fn run_replacing(effects: &[Effect]) -> Vec<Option<AttemptId>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::StartRun { replacing, .. } => Some(replacing.clone()),
            _ => None,
        })
        .collect()
}

fn reason(h: &Harness) -> Option<AttemptReason> {
    h.attempt().map(|attempt| attempt.reason)
}

fn after(h: &Harness) -> Option<AfterStop> {
    h.stopping().map(|stop| stop.after)
}

fn shows_connected_on(h: &Harness, session: &Session) {
    let section = h.cm.connection_section();
    assert_eq!(section.icon, ConnectionIcon::Connected);
    assert_eq!(
        section.status,
        ConnectionStatus::Connected {
            since: session.since,
            tunnel_ip: session.tunnel_ip,
            proactive_reconnect: ProactiveReconnect::At {
                at: session.planned_reconnect.unwrap()
            },
        }
    );
    assert_eq!(section.wait_reason, None);
    assert_eq!(section.reconnect_now, ButtonState::Enabled);
    assert_eq!(section.disconnect, ButtonState::Enabled);
}

fn start_replacement(h: &mut Harness, old: &Session) -> AttemptId {
    assert_eq!(
        replaced(h).map(|session| &session.attempt),
        Some(&old.attempt)
    );
    let id = h.attempt().unwrap().id.clone();
    if h.attempt().unwrap().stage == Stage::CheckingReadiness {
        h.send(Input::ReadinessChecked(ready()));
    }
    let effects = h.send(Input::OtpSessionReady(id.clone()));
    assert_eq!(run_replacing(&effects), [Some(old.attempt.clone())]);
    id
}

fn reject_login(h: &mut Harness, id: &AttemptId) {
    h.run_milestones(id, Milestone::ConnectedToGateway);
    h.exit_for(
        id,
        ended(Milestone::ConnectedToGateway).by_helper(StopReason::PromptRepeated),
    );
}

fn fail_on_the_network(h: &mut Harness, id: &AttemptId) {
    h.advance(1.0);
    h.milestone_for(id, Milestone::Spawned, None);
    h.exit_for(id, ended(Milestone::Spawned));
}

#[test]
fn the_proactive_reconnect_brings_up_the_next_tunnel_before_the_old_one_ends() {
    let (mut h, old) = connected();
    let effects = h.fire(TimerKind::ProactiveReconnect);
    assert_eq!(stop_reasons(&effects), []);
    assert!(effects.contains(&Effect::CheckReadiness));
    assert_eq!(reason(&h), Some(AttemptReason::Proactive));
    shows_connected_on(&h, &old);
    assert!(h.cm.wants_vpn());

    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::Authenticated);
    shows_connected_on(&h, &old);
    h.advance(2.0);
    let effects = h.milestone_for(&id, Milestone::TunnelUp, Some(NEXT_TUNNEL_IP));
    assert_eq!(stop_reasons(&effects), []);
    let session = h.connected().unwrap().clone();
    assert_eq!(session.attempt, id);
    assert_eq!(session.since, h.now);
    assert_eq!(session.tunnel_ip, Some(NEXT_TUNNEL_IP));
    assert_eq!(session.planned_reconnect, Some(h.now + secs(175.0 * 60.0)));
    assert_eq!(session.renewal, None);
    assert_eq!(session.replaced, Some(old.attempt.clone()));
    assert_eq!(
        timer_set(&effects, TimerKind::ProactiveReconnect),
        session.planned_reconnect
    );
    shows_connected_on(&h, &session);

    h.advance(1.0);
    let effects = h.exit_for(
        &old.attempt,
        ended(Milestone::TunnelUp).by_helper(StopReason::Replaced),
    );
    assert_eq!(connection_effects(&effects), []);
    assert_eq!(h.connected().unwrap().replaced, None);
    assert_eq!(h.connected().unwrap().attempt, id);
}

#[test]
fn reconnect_now_while_connected_brings_up_the_next_tunnel_first() {
    let (mut h, old) = connected();
    h.advance(600.0);
    let effects = h.send(Input::ReconnectNow);
    assert_eq!(stop_reasons(&effects), []);
    assert_eq!(saved_ledger(&effects).unwrap().recent_auth_attempts, []);
    assert_eq!(reason(&h), Some(AttemptReason::ReconnectNow));
    assert_eq!(
        h.cm.connection_section().status,
        ConnectionStatus::Connecting {
            reconnecting: true,
            progress: ConnectProgress::CheckingReadiness
        }
    );
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::TunnelUp);
    let session = h.connected().unwrap().clone();
    assert_eq!(session.attempt, id);
    shows_connected_on(&h, &session);
}

#[test]
fn a_wake_past_the_planned_time_keeping_the_tunnel_prepares_the_next_one() {
    let (mut h, old) = connected();
    h.sleep();
    h.advance(175.5 * 60.0);
    h.wake();
    let effects = h.settle(observe());
    assert_eq!(stop_reasons(&effects), []);
    assert_eq!(reason(&h), Some(AttemptReason::Proactive));
    shows_connected_on(&h, &old);
    start_replacement(&mut h, &old);
}

#[test]
fn a_wake_that_does_not_keep_the_tunnel_stops_it_first_as_before() {
    for observation in [observe().dead(), observe().via(Some(ethernet()))] {
        let (mut h, _) = connected();
        h.sleep();
        h.advance(175.5 * 60.0);
        h.wake();
        let effects = h.settle(observation);
        assert_eq!(stop_reasons(&effects), [StopReason::ProactiveReconnect]);
        assert_eq!(
            after(&h),
            Some(AfterStop::Reconnect(AttemptReason::Proactive))
        );
    }
}

#[test]
fn without_a_network_the_old_tunnel_is_stopped_first() {
    let (mut h, _) = connected();
    h.sleep();
    h.advance(175.5 * 60.0);
    h.wake();
    h.settle(observe().via(None).unsettled());
    assert!(h.connected().is_some());
    assert_eq!(
        stop_reasons(&h.fire(TimerKind::ProactiveReconnect)),
        [StopReason::ProactiveReconnect]
    );
}

#[test]
fn a_session_too_close_to_its_end_stops_first() {
    let mut settings = settings();
    settings.connection.proactive_margin_minutes = 2;
    let (mut h, _) = connected_with(settings);
    assert_eq!(
        stop_reasons(&h.fire(TimerKind::ProactiveReconnect)),
        [StopReason::ProactiveReconnect]
    );

    let (mut h, _) = connected();
    h.sleep();
    h.advance(4.0 * 3600.0);
    h.wake();
    assert_eq!(
        stop_reasons(&h.settle(observe())),
        [StopReason::ProactiveReconnect]
    );
}

#[test]
fn a_retry_too_close_to_the_session_end_stops_first() {
    let mut settings = settings();
    settings.connection.proactive_margin_minutes = 4;
    let (mut h, old) = connected_with(settings);
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    reject_login(&mut h, &id);
    assert!(h.connected().is_some());
    let effects = h.fire(TimerKind::Retry);
    assert_eq!(stop_reasons(&effects), [StopReason::ProactiveReconnect]);
    assert_eq!(
        after(&h),
        Some(AfterStop::Reconnect(AttemptReason::Proactive))
    );
    h.exit_for(
        &old.attempt,
        ended(Milestone::TunnelUp).by_helper(StopReason::ProactiveReconnect),
    );
    assert_eq!(reason(&h), Some(AttemptReason::Proactive));
    assert_eq!(replaced(&h), None);
}

#[test]
fn login_failures_wait_out_the_cooldown_and_the_second_keeps_the_tunnel() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let first = start_replacement(&mut h, &old);
    reject_login(&mut h, &first);
    let failed_at = h.now;
    assert_eq!(
        h.connected().map(|session| &session.attempt),
        Some(&old.attempt)
    );
    shows_connected_on(&h, &old);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
    assert_eq!(h.cm.ledger.last_auth_failure_at, Some(failed_at));
    assert_eq!(
        h.cm.ledger.last_auth_failure,
        Some(Failure::AuthenticationRejected)
    );
    assert_eq!(renewal(&h).unwrap().tries, 1);
    assert_eq!(h.timer(TimerKind::Retry), Some(failed_at + secs(90.0)));
    assert_eq!(h.timer(TimerKind::ProactiveReconnect), None);

    h.fire(TimerKind::Retry);
    let second = start_replacement(&mut h, &old);
    h.run_milestones(&second, Milestone::OtpSubmitted);
    let effects = h.exit_for(
        &second,
        ended(Milestone::OtpSubmitted).by_helper(StopReason::PromptRepeated),
    );
    assert_eq!(stop_reasons(&effects), []);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 2);
    assert_eq!(h.cm.ledger.last_auth_failure, Some(Failure::OtpRejected));
    assert_eq!(
        h.connected().map(|session| &session.attempt),
        Some(&old.attempt)
    );
    assert_eq!(renewal(&h).unwrap().next_try, None);
    assert_eq!(h.connection_timers(), []);
    shows_connected_on(&h, &old);

    let second_failed_at = h.now;
    h.exit_for(&old.attempt, ended(Milestone::TunnelUp).code(-EPIPE));
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::ServerSessionEnded)
    );
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 2);
    h.fire(TimerKind::Retry);
    assert_eq!(
        h.retry_wait(),
        Some((second_failed_at + secs(90.0), RetryReason::AuthCooldown))
    );
}

#[test]
fn other_failures_retry_at_once_then_back_off_uncounted() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let first = start_replacement(&mut h, &old);
    h.run_milestones(&first, Milestone::OtpPrompted);
    let attempts = h.cm.ledger.recent_auth_attempts.len();
    h.advance(5.0);
    assert_eq!(
        stop_reasons(&h.send(Input::OtpSessionFailed(first.clone(), OtpFailure::Network))),
        [StopReason::OtpUnavailable]
    );
    let effects = h.exit_for(
        &first,
        ended(Milestone::OtpPrompted).by_helper(StopReason::OtpUnavailable),
    );
    assert!(effects.contains(&Effect::CheckReadiness));
    assert_eq!(renewal(&h).unwrap().tries, 1);
    let second = start_replacement(&mut h, &old);
    fail_on_the_network(&mut h, &second);
    assert_eq!(
        h.connected().map(|session| &session.attempt),
        Some(&old.attempt)
    );
    assert_eq!(renewal(&h).unwrap().tries, 2);
    assert_eq!(h.timer(TimerKind::Retry), Some(h.now + secs(10.0)));

    h.fire(TimerKind::Retry);
    let third = start_replacement(&mut h, &old);
    h.run_milestones(&third, Milestone::OtpPrompted);
    assert_eq!(h.cm.ledger.recent_auth_attempts.len(), attempts);
    h.fire(TimerKind::OtpDeadline);
    h.exit_for(
        &third,
        ended(Milestone::OtpPrompted).by_helper(StopReason::OtpTimeout),
    );
    assert_eq!(h.timer(TimerKind::Retry), Some(h.now + secs(20.0)));
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
    assert_eq!(h.cm.ledger.last_auth_failure_at, None);
    shows_connected_on(&h, &old);
}

#[test]
fn failures_before_the_run_count_as_other_failures_and_never_stop() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let mut missing = ready();
    missing.vpn_password_stored = false;
    h.send(Input::ReadinessChecked(missing));
    assert_eq!(renewal(&h).unwrap().tries, 1);
    let id = h.attempt().unwrap().id.clone();
    h.send(Input::ReadinessChecked(ready()));
    h.send(Input::OtpSessionFailed(id, OtpFailure::Authentication));
    assert_eq!(
        h.connected().map(|session| &session.attempt),
        Some(&old.attempt)
    );
    assert_eq!(renewal(&h).unwrap().tries, 2);
    assert_eq!(h.timer(TimerKind::Retry), Some(h.now + secs(10.0)));

    h.fire(TimerKind::Retry);
    let id = start_replacement(&mut h, &old);
    h.send(Input::RunRejected(id, HelperErrorCode::ClientUnavailable));
    assert_eq!(h.timer(TimerKind::Retry), Some(h.now + secs(20.0)));
    h.fire(TimerKind::Retry);
    h.fire(TimerKind::AttemptDeadline);
    assert_eq!(h.timer(TimerKind::Retry), Some(h.now + secs(40.0)));
    shows_connected_on(&h, &old);
}

#[test]
fn a_tunnel_address_in_use_stops_the_old_tunnel_first() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::Authenticated);
    let effects = h.exit_for(
        &id,
        ended(Milestone::Authenticated)
            .by_helper(StopReason::NetworkSetupFailed)
            .address_in_use(),
    );
    assert_eq!(stop_reasons(&effects), [StopReason::ProactiveReconnect]);
    assert_eq!(
        after(&h),
        Some(AfterStop::Reconnect(AttemptReason::Proactive))
    );
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
    h.exit_for(
        &old.attempt,
        ended(Milestone::TunnelUp).by_helper(StopReason::ProactiveReconnect),
    );
    assert_eq!(reason(&h), Some(AttemptReason::Proactive));
    assert_eq!(replaced(&h), None);
}

#[test]
fn a_failed_switch_ends_both_tunnels_as_a_network_setup_failure() {
    let switch_failed =
        || ended(Milestone::Authenticated).by_helper(StopReason::NetworkSetupFailed);
    let old_failed = || ended(Milestone::TunnelUp).by_helper(StopReason::NetworkSetupFailed);

    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::Authenticated);
    h.exit_for(&id, switch_failed());
    let next = h.attempt().unwrap().id.clone();
    let effects = h.exit_for(&old.attempt, old_failed());
    assert_eq!(connection_effects(&effects), []);
    assert_eq!(h.attempt().map(|attempt| &attempt.id), Some(&next));
    assert_eq!(replaced(&h), None);

    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::Authenticated);
    h.exit_for(&old.attempt, old_failed());
    h.exit_for(&id, switch_failed());
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(10.0), RetryReason::NetworkSetupFailed))
    );

    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    for _ in 0..2 {
        let id = start_replacement(&mut h, &old);
        h.run_milestones(&id, Milestone::Authenticated);
        h.exit_for(&id, switch_failed());
    }
    assert!(h.connected().is_some());
    h.exit_for(&old.attempt, old_failed());
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(10.0), RetryReason::NetworkSetupFailed))
    );
}

#[test]
fn a_certificate_change_prepares_again_at_once_uncounted() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.advance(1.0);
    h.milestone_for(&id, Milestone::Spawned, None);
    let effects = h.exit_for(
        &id,
        ended(Milestone::Spawned).certs(&[digest('b')], Some(Milestone::Spawned)),
    );
    assert_eq!(saved_certs(&effects).unwrap()[0], digest('b'));
    assert_eq!(renewal(&h).unwrap().tries, 0);
    let id = start_replacement(&mut h, &old);
    h.advance(1.0);
    h.milestone_for(&id, Milestone::Spawned, None);
    h.exit_for(
        &id,
        ended(Milestone::Spawned).certs(&[digest('c')], Some(Milestone::Spawned)),
    );
    assert_eq!(renewal(&h).unwrap().tries, 1);
    start_replacement(&mut h, &old);
}

#[test]
fn the_hourly_limit_keeps_the_tunnel() {
    let mut settings = settings();
    settings.connection.session_max_minutes = 60;
    settings.connection.proactive_margin_minutes = 50;
    let mut ledger = empty_ledger();
    ledger.recent_auth_attempts = vec![t0() - secs(60.0); 11];
    let mut h = Harness::with_settings(settings);
    h.launch_with(true, Some(ledger));
    h.run_to_connected();
    let old = h.connected().unwrap().clone();
    let effects = h.fire(TimerKind::ProactiveReconnect);
    assert_eq!(connection_effects(&effects), []);
    assert_eq!(
        h.connected().map(|session| &session.attempt),
        Some(&old.attempt)
    );
    assert_eq!(renewal(&h).unwrap().next_try, None);
    assert_eq!(h.connection_timers(), []);

    h.send(Input::ReconnectNow);
    start_replacement(&mut h, &old);
}

#[test]
fn a_renewal_started_by_reconnect_now_shows_what_a_reconnect_attempt_shows() {
    let (mut renewing, old) = connected();
    let (mut restarting, _) = connected();
    renewing.advance(600.0);
    renewing.send(Input::ReconnectNow);
    restarting.advance(180.0 * 60.0 - 60.0);
    assert_eq!(
        stop_reasons(&restarting.send(Input::ReconnectNow)),
        [StopReason::Reconnect]
    );
    assert_eq!(
        after(&restarting),
        Some(AfterStop::Reconnect(AttemptReason::ReconnectNow))
    );
    restarting.exit(ended(Milestone::TunnelUp).by_helper(StopReason::Reconnect));
    let section = renewing.cm.connection_section();
    assert_eq!(section, restarting.cm.connection_section());
    assert_eq!(section.icon, ConnectionIcon::InProgress);
    assert_eq!(
        section.status,
        ConnectionStatus::Connecting {
            reconnecting: true,
            progress: ConnectProgress::CheckingReadiness
        }
    );
    assert_eq!(section.reconnect_now, ButtonState::Disabled);
    assert_eq!(section.disconnect, ButtonState::Enabled);

    let renewal = start_replacement(&mut renewing, &old);
    let attempt = restarting.start_tunnel();
    renewing.run_milestones(&renewal, Milestone::OtpPrompted);
    restarting.run_milestones(&attempt, Milestone::OtpPrompted);
    let section = renewing.cm.connection_section();
    assert_eq!(section, restarting.cm.connection_section());
    assert_eq!(
        section.status,
        ConnectionStatus::Connecting {
            reconnecting: true,
            progress: ConnectProgress::WaitingForOtp
        }
    );
    assert_eq!(section.reconnect_now, ButtonState::Disabled);
}

#[test]
fn reconnect_now_during_a_proactive_preparation_turns_it_into_a_reconnect_now_renewal() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::OtpPrompted);
    shows_connected_on(&h, &old);
    assert!(!h.cm.ledger.recent_auth_attempts.is_empty());

    let effects = h.send(Input::ReconnectNow);
    assert_eq!(
        connection_effects(&effects),
        [Effect::SaveLedger(FailureLedger {
            recent_auth_attempts: Vec::new(),
            ..h.cm.ledger.clone()
        })]
    );
    assert_eq!(h.attempt().map(|attempt| &attempt.id), Some(&id));
    assert_eq!(reason(&h), Some(AttemptReason::ReconnectNow));
    assert_eq!(renewal(&h).unwrap().reason, AttemptReason::ReconnectNow);
    let section = h.cm.connection_section();
    assert_eq!(section.icon, ConnectionIcon::InProgress);
    assert_eq!(
        section.status,
        ConnectionStatus::Connecting {
            reconnecting: true,
            progress: ConnectProgress::WaitingForOtp
        }
    );
    assert_eq!(section.reconnect_now, ButtonState::Disabled);

    h.run_milestones(&id, Milestone::TunnelUp);
    let session = h.connected().unwrap().clone();
    assert_eq!(session.attempt, id);
    assert_eq!(session.replaced, Some(old.attempt.clone()));
    shows_connected_on(&h, &session);
}

#[test]
fn reconnect_now_during_a_later_proactive_preparation_starts_its_tries_over() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let first = start_replacement(&mut h, &old);
    fail_on_the_network(&mut h, &first);
    assert_eq!(renewal(&h).unwrap().tries, 1);
    let second = start_replacement(&mut h, &old);
    h.send(Input::ReconnectNow);
    assert_eq!(h.attempt().map(|attempt| &attempt.id), Some(&second));
    assert_eq!(
        renewal(&h),
        Some(Renewal {
            reason: AttemptReason::ReconnectNow,
            tries: 0,
            later_failures: 0,
            next_try: None,
        })
    );

    fail_on_the_network(&mut h, &second);
    assert_eq!(renewal(&h).unwrap().tries, 1);
    let third = start_replacement(&mut h, &old);
    assert_eq!(reason(&h), Some(AttemptReason::ReconnectNow));
    fail_on_the_network(&mut h, &third);
    assert_eq!(
        h.connected().map(|session| &session.attempt),
        Some(&old.attempt)
    );
    assert_eq!(renewal(&h).unwrap().tries, 2);
    assert_eq!(h.timer(TimerKind::Retry), Some(h.now + secs(10.0)));
}

#[test]
fn reconnect_now_during_a_reconnect_now_preparation_is_ignored() {
    let (mut h, old) = connected();
    h.advance(600.0);
    h.send(Input::ReconnectNow);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::OtpPrompted);
    let before = h.cm.phase.clone();
    assert_eq!(connection_effects(&h.send(Input::ReconnectNow)), []);
    assert_eq!(h.cm.phase, before);

    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.send(Input::ReconnectNow);
    h.run_milestones(&id, Milestone::OtpPrompted);
    let before = h.cm.phase.clone();
    assert_eq!(connection_effects(&h.send(Input::ReconnectNow)), []);
    assert_eq!(h.cm.phase, before);
}

#[test]
fn reconnect_now_while_waiting_to_prepare_again_starts_a_new_cycle() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    for _ in 0..2 {
        let id = start_replacement(&mut h, &old);
        fail_on_the_network(&mut h, &id);
    }
    assert!(h.timer(TimerKind::Retry).is_some());
    h.send(Input::ReconnectNow);
    assert_eq!(reason(&h), Some(AttemptReason::ReconnectNow));
    assert_eq!(renewal(&h).unwrap().tries, 0);
}

#[test]
fn a_given_up_reconnect_now_leaves_the_planned_reconnect() {
    let (mut h, old) = connected();
    h.advance(600.0);
    h.send(Input::ReconnectNow);
    for _ in 0..2 {
        if h.timer(TimerKind::Retry).is_some() {
            h.fire(TimerKind::Retry);
        }
        let id = start_replacement(&mut h, &old);
        reject_login(&mut h, &id);
        shows_connected_on(&h, &old);
    }
    assert_eq!(renewal(&h), None);
    assert_eq!(h.connection_timers(), [TimerKind::ProactiveReconnect]);
    assert_eq!(
        h.timer(TimerKind::ProactiveReconnect),
        old.planned_reconnect
    );
}

#[test]
fn a_renewal_started_by_reconnect_now_retries_while_the_user_is_away_outside_the_hours() {
    let mut settings = settings();
    settings.connection.schedule = Some(weekday_hours());
    let mut h = Harness::with_settings(settings);
    h.now = at(18, 17, 0, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    let old = h.connected().unwrap().clone();
    h.send(Input::ReconnectNow);
    let first = start_replacement(&mut h, &old);
    reject_login(&mut h, &first);
    let retry_at = h.timer(TimerKind::Retry).unwrap();
    h.send(Input::ScreenLocked { at: h.now });
    assert_eq!(h.timer(TimerKind::Retry), Some(retry_at));
    h.fire(TimerKind::Retry);
    start_replacement(&mut h, &old);
    assert_eq!(reason(&h), Some(AttemptReason::ReconnectNow));
}

#[test]
fn a_proactive_renewal_waits_for_the_user_away_outside_the_hours() {
    let mut settings = settings();
    settings.connection.schedule = Some(weekday_hours());
    let mut h = Harness::with_settings(settings);
    h.now = at(18, 17, 0, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    let old = h.connected().unwrap().clone();
    h.fire(TimerKind::ProactiveReconnect);
    let first = start_replacement(&mut h, &old);
    reject_login(&mut h, &first);
    let retry_at = h.timer(TimerKind::Retry).unwrap();
    h.send(Input::ScreenLocked { at: h.now });
    assert_eq!(h.timer(TimerKind::Retry), None);
    h.send(Input::ScreenUnlocked);
    assert_eq!(h.timer(TimerKind::Retry), Some(retry_at));
    h.fire(TimerKind::Retry);
    start_replacement(&mut h, &old);
    assert_eq!(reason(&h), Some(AttemptReason::Proactive));
}

#[test]
fn reconnect_now_during_a_proactive_preparation_keeps_retrying_while_away_outside_the_hours() {
    let mut settings = settings();
    settings.connection.schedule = Some(weekday_hours());
    let mut h = Harness::with_settings(settings);
    h.now = at(18, 17, 0, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    let old = h.connected().unwrap().clone();
    h.fire(TimerKind::ProactiveReconnect);
    let first = start_replacement(&mut h, &old);
    h.send(Input::ReconnectNow);
    reject_login(&mut h, &first);
    let retry_at = h.timer(TimerKind::Retry).unwrap();
    h.send(Input::ScreenLocked { at: h.now });
    assert_eq!(h.timer(TimerKind::Retry), Some(retry_at));
    h.fire(TimerKind::Retry);
    start_replacement(&mut h, &old);
    assert_eq!(reason(&h), Some(AttemptReason::ReconnectNow));
}

#[test]
fn disconnect_and_quit_stop_both_tunnels_and_wait_for_both() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::OtpPrompted);
    let effects = h.send(Input::Disconnect);
    assert_eq!(
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::StopRun { attempt, reason } => Some((attempt.clone(), *reason)),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [
            (id.clone(), StopReason::UserDisconnect),
            (old.attempt.clone(), StopReason::UserDisconnect)
        ]
    );
    assert_eq!(after(&h), Some(AfterStop::Idle));
    h.exit_for(
        &id,
        ended(Milestone::OtpPrompted).by_helper(StopReason::UserDisconnect),
    );
    assert_eq!(after(&h), Some(AfterStop::Idle));
    h.exit_for(
        &old.attempt,
        ended(Milestone::TunnelUp).by_helper(StopReason::UserDisconnect),
    );
    assert_eq!(h.cm.phase, Phase::Idle);

    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    let effects = h.send(Input::QuitRequested);
    assert_eq!(
        stop_reasons(&effects),
        [StopReason::AppQuit, StopReason::AppQuit]
    );
    let effects = h.exit_for(
        &old.attempt,
        ended(Milestone::TunnelUp).by_helper(StopReason::AppQuit),
    );
    assert!(!effects.contains(&Effect::ApproveQuit));
    assert_eq!(h.cm.current_attempt(), Some(&id));
    let effects = h.exit_for(
        &id,
        ended(Milestone::Spawned).by_helper(StopReason::AppQuit),
    );
    assert_eq!(effects.last(), Some(&Effect::ApproveQuit));

    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let effects = h.send(Input::Disconnect);
    assert_eq!(stop_reasons(&effects), [StopReason::UserDisconnect]);
    h.exit_for(
        &old.attempt,
        ended(Milestone::TunnelUp).by_helper(StopReason::UserDisconnect),
    );
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn the_old_tunnel_ending_during_the_preparation_leaves_the_next_as_the_attempt() {
    let mut h = Harness::with_settings(settings());
    h.launch();
    let first = h.start_tunnel();
    reject_login(&mut h, &first);
    h.fire(TimerKind::Retry);
    h.run_to_connected();
    let old = h.connected().unwrap().clone();
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::OtpPrompted);
    let effects = h.exit_for(&old.attempt, ended(Milestone::TunnelUp));
    assert_eq!(connection_effects(&effects), []);
    assert_eq!(h.attempt().map(|attempt| &attempt.id), Some(&id));
    assert_eq!(replaced(&h), None);
    assert_eq!(
        h.cm.connection_section().status,
        ConnectionStatus::Connecting {
            reconnecting: true,
            progress: ConnectProgress::WaitingForOtp
        }
    );
    h.run_milestones(&id, Milestone::TunnelUp);
    assert_eq!(h.connected().unwrap().replaced, None);

    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    h.exit_for(&old.attempt, ended(Milestone::TunnelUp));
    h.send(Input::ReadinessChecked(ready()));
    let id = h.attempt().unwrap().id.clone();
    let effects = h.send(Input::OtpSessionReady(id.clone()));
    assert_eq!(run_replacing(&effects), [None]);
    fail_on_the_network(&mut h, &id);
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(10.0), RetryReason::Network))
    );
}

#[test]
fn a_replacement_reaching_the_tunnel_clears_the_failure_count() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let first = start_replacement(&mut h, &old);
    reject_login(&mut h, &first);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
    h.fire(TimerKind::Retry);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::Authenticated);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
    h.advance(2.0);
    let effects = h.milestone_for(&id, Milestone::TunnelUp, Some(NEXT_TUNNEL_IP));
    assert_eq!(saved_ledger(&effects).unwrap().consecutive_auth_failures, 0);
    assert_eq!(h.connected().unwrap().attempt, id);
    let effects = h.exit_for(
        &old.attempt,
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::Replaced)
            .code(-EINTR),
    );
    assert_eq!(saved_ledger(&effects), None);
    assert_eq!(h.connected().unwrap().replaced, None);
}

#[test]
fn sleep_stops_the_preparation_and_the_wake_prepares_again() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::OtpPrompted);
    let effects = h.sleep();
    assert_eq!(stop_reasons(&effects), [StopReason::Sleep]);
    assert_eq!(
        h.connected().map(|session| &session.attempt),
        Some(&old.attempt)
    );
    h.advance(30.0);
    h.wake();
    h.settle(observe());
    assert!(h.connected().is_some());
    h.fire(TimerKind::Retry);
    start_replacement(&mut h, &old);
    assert_eq!(renewal(&h).unwrap().tries, 0);
}

#[test]
fn a_network_change_during_the_preparation_stops_both_and_reconnects() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::OtpPrompted);
    let effects = h.set_path(online_via(ethernet()));
    assert_eq!(
        stop_reasons(&effects),
        [StopReason::Reconnect, StopReason::Reconnect]
    );
    assert_eq!(
        after(&h),
        Some(AfterStop::Reconnect(AttemptReason::NetworkChanged))
    );
    h.exit_for(
        &id,
        ended(Milestone::OtpPrompted).by_helper(StopReason::Reconnect),
    );
    h.exit_for(
        &old.attempt,
        ended(Milestone::TunnelUp).by_helper(StopReason::Reconnect),
    );
    assert_eq!(reason(&h), Some(AttemptReason::NetworkChanged));
}

#[test]
fn losing_the_helper_during_the_preparation_ends_the_old_tunnel() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.run_milestones(&id, Milestone::OtpPrompted);
    h.send(Input::HelperConnectionLost);
    assert_eq!(reason(&h), Some(retry(RetryReason::ForcedTermination)));
    assert_eq!(replaced(&h), None);

    let (mut h, _) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = h.attempt().unwrap().id.clone();
    h.send(Input::HelperConnectionLost);
    assert_eq!(h.attempt().map(|attempt| &attempt.id), Some(&id));
    assert_eq!(replaced(&h), None);
}

#[test]
fn a_late_exit_of_a_stopped_preparation_changes_nothing() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    let id = start_replacement(&mut h, &old);
    h.sleep();
    h.advance(30.0);
    h.wake();
    h.settle(observe());
    let before = h.cm.phase.clone();
    assert_eq!(
        connection_effects(
            &h.exit_for(&id, ended(Milestone::Spawned).by_helper(StopReason::Sleep))
        ),
        []
    );
    assert_eq!(h.cm.phase, before);
}

#[test]
fn a_new_plan_after_giving_up_reconnects_at_its_time() {
    let (mut h, old) = connected();
    h.fire(TimerKind::ProactiveReconnect);
    for _ in 0..2 {
        if h.timer(TimerKind::Retry).is_some() {
            h.fire(TimerKind::Retry);
        }
        let id = start_replacement(&mut h, &old);
        reject_login(&mut h, &id);
    }
    assert_eq!(h.connection_timers(), []);
    let mut settings = settings();
    settings.connection.session_max_minutes = 240;
    h.send(Input::SettingsChanged(settings));
    assert_eq!(renewal(&h), None);
    assert_eq!(
        h.timer(TimerKind::ProactiveReconnect),
        Some(old.since + secs(235.0 * 60.0))
    );
}
