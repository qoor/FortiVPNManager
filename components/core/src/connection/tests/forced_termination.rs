//! A forced termination reconnects at once, once until a tunnel comes up again.

use std::time::Duration;

use fvm_contract::app_helper::{Milestone, StopReason};

use super::super::machine::{AttemptReason, Phase, RetryReason};
use super::super::{Effect, Failure, FailureLedger, Input, OtpFailure, TimerKind};
use super::harness::*;

const FORCED: AttemptReason = AttemptReason::Retry(RetryReason::ForcedTermination);

fn reason(h: &Harness) -> Option<AttemptReason> {
    h.attempt().map(|attempt| attempt.reason)
}

fn waits(h: &Harness, seconds: f64, reason: RetryReason) {
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(seconds), reason)),
        "{:?}",
        h.cm.phase
    );
}

fn scheduled_at(day: i8, hour: i8, minute: i8) -> Harness {
    let mut h = Harness::with_settings(scheduled(weekday_hours()));
    h.now = at(day, hour, minute, 0);
    h
}

#[test]
fn a_lost_helper_while_connected_reconnects_at_once_uncounted() {
    for up in [1.0, 30.0, 7200.0] {
        let mut h = Harness::new();
        h.launch_connected();
        h.lose_helper(up);
        assert!(h.effects.contains(&Effect::CheckReadiness), "{up}");
        assert_eq!(reason(&h), Some(FORCED));
        assert_eq!(h.timer(TimerKind::Retry), None);
        assert_eq!(h.cm.ledger.consecutive_auth_failures, 0, "{up}");
        h.run_to_connected();
    }
}

#[test]
fn a_session_the_server_ended_or_rejected_reconnects_shortly() {
    for code in [-EPIPE, -EPERM] {
        let mut h = Harness::new();
        h.launch_connected();
        h.advance(30.0);
        h.exit(ended(Milestone::TunnelUp).code(code));
        waits(&h, 5.0, RetryReason::ServerSessionEnded);
        assert_eq!(h.cm.ledger.consecutive_auth_failures, 0, "{code}");
    }
}

#[test]
fn any_other_end_of_a_session_backs_off_uncounted() {
    for exit in [
        ended(Milestone::TunnelUp),
        ended(Milestone::TunnelUp).code(-EINTR),
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::HelperShutdown)
            .code(-EINTR),
    ] {
        let mut h = Harness::new();
        h.launch_connected();
        h.advance(7200.0);
        h.exit(exit);
        waits(&h, 10.0, RetryReason::Network);
        assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
    }
}

#[test]
fn losing_the_helper_after_gmail_rejected_the_login_still_fails() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    h.send(Input::OtpSessionFailed(id, OtpFailure::Authentication));
    h.send(Input::HelperConnectionLost);
    assert_eq!(h.cm.phase, Phase::Failed(Failure::GmailRejected));
}

#[test]
fn an_immediate_reconnect_that_loses_the_helper_while_connecting_backs_off() {
    let mut h = Harness::new();
    h.launch_connected();
    h.lose_helper(600.0);
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::ConnectedToGateway);
    h.send(Input::HelperConnectionLost);
    waits(&h, 10.0, RetryReason::Network);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
}

#[test]
fn a_tunnel_up_makes_the_next_lost_helper_immediate_again() {
    let mut h = Harness::new();
    h.launch_connected();
    h.lose_helper(600.0);
    assert_eq!(reason(&h), Some(FORCED));
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpSubmitted);
    h.send(Input::HelperConnectionLost);
    waits(&h, 10.0, RetryReason::Network);
    // No tunnel has come up since the immediate reconnect.
    h.fire(TimerKind::Retry);
    h.run_to_connected();
    h.lose_helper(1.0);
    assert_eq!(reason(&h), Some(FORCED));
}

#[test]
fn a_user_connect_or_reconnect_starts_fresh() {
    for reconnect_now in [false, true] {
        let mut h = Harness::new();
        h.launch_connected();
        h.lose_helper(600.0);
        let id = h.start_tunnel();
        h.run_milestones(&id, Milestone::Started);
        h.send(Input::HelperConnectionLost);
        assert_eq!(
            h.retry_wait().map(|(_, reason)| reason),
            Some(RetryReason::Network)
        );
        if reconnect_now {
            h.send(Input::ReconnectNow);
        } else {
            h.send(Input::Disconnect);
            h.send(Input::Connect);
        }
        let id = h.start_tunnel();
        h.run_milestones(&id, Milestone::Started);
        h.send(Input::HelperConnectionLost);
        assert_eq!(reason(&h), Some(FORCED), "reconnect now: {reconnect_now}");
    }
}

#[test]
fn a_lost_helper_during_sleep_waits_for_the_wake() {
    let mut h = Harness::new();
    h.launch_connected();
    h.advance(600.0);
    h.sleep();
    h.lose_helper(1200.0);
    assert_eq!(h.network_wait(), Some((RetryReason::Sleep, Duration::ZERO)));
    assert!(!h.effects.contains(&Effect::CheckReadiness));
    assert!(
        h.connection_timers().is_empty(),
        "{:?}",
        h.connection_timers()
    );
    h.advance(60.0);
    h.wake();
    let effects = h.settle(&observe().dead());
    assert!(effects.contains(&Effect::CheckReadiness), "{effects:?}");
    assert_eq!(reason(&h), Some(AttemptReason::AfterSleep));
}

#[test]
fn a_lost_helper_while_offline_waits_for_the_path_then_starts_at_once() {
    let mut h = Harness::new();
    h.launch_connected();
    h.advance(300.0);
    h.set_path(&offline());
    assert!(h.connected().is_some());
    h.lose_helper(600.0);
    assert_eq!(
        h.network_wait(),
        Some((RetryReason::ForcedTermination, Duration::ZERO))
    );
    assert!(
        h.connection_timers().is_empty(),
        "{:?}",
        h.connection_timers()
    );
    h.advance(120.0);
    let effects = h.set_path(&online());
    assert!(effects.contains(&Effect::CheckReadiness), "{effects:?}");
    assert_eq!(reason(&h), Some(FORCED));
}

#[test]
fn a_lost_helper_outside_the_hours_while_away_holds_the_reconnect() {
    let mut h = scheduled_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    h.advance(600.0);
    h.send(Input::ScreenLocked { at: h.now });
    h.lose_helper(6000.0);
    assert_eq!(
        h.retry_wait(),
        Some((at(18, 20, 10, 0), RetryReason::UserAway))
    );
    assert!(!h.effects.contains(&Effect::CheckReadiness));
    h.advance(600.0);
    h.send(Input::ScreenUnlocked);
    assert_eq!(
        reason(&h),
        Some(AttemptReason::Retry(RetryReason::UserAway))
    );
}

#[test]
fn a_lost_helper_after_reconnects_were_suspended_stays_off() {
    let mut h = scheduled_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    h.advance(600.0);
    h.send(Input::ScreenLocked { at: h.now });
    h.fire(TimerKind::AwayLimit);
    assert!(h.cm.reconnect_limits.reconnect_suspended);
    h.lose_helper(4.0 * 3600.0);
    assert_eq!(h.cm.phase, Phase::Idle);
    assert!(!h.cm.wants_connection);
}

#[test]
fn a_lost_helper_at_the_hourly_attempt_cap_fails() {
    let mut h = Harness::new();
    let mut attempts: Vec<_> = (1..=11)
        .map(|n| h.now - Duration::from_secs(n * 60))
        .collect();
    attempts.reverse();
    h.launch_with(
        true,
        Some(FailureLedger {
            recent_auth_attempts: attempts,
            ..empty_ledger()
        }),
    );
    h.run_to_connected();
    h.lose_helper(600.0);
    assert_eq!(h.cm.phase, Phase::Failed(Failure::TooManyReconnects));
}
