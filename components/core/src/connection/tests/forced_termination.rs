//! A forced termination reconnects at once, once until a session is stable (SPEC F-CM-03, F-CM-04).

use std::time::Duration;

use fvm_contract::app_helper::{Milestone, ProcessExitStatus, StopReason};

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

/// Starts from one counted failure: a forced end never adds to it, and a stable session still
/// clears it.
#[test]
fn a_forced_end_while_connected_reconnects_at_once_uncounted() {
    for end in FORCED_ENDS {
        for (up, failures_after) in [(30.0, 1), (119.0, 1), (180.0, 0), (7200.0, 0)] {
            let mut h = Harness::new();
            let ledger = FailureLedger {
                consecutive_auth_failures: 1,
                last_auth_failure_at: Some(h.now - Duration::from_secs(3600)),
                last_auth_failure: Some(Failure::OtpRejected),
                ..empty_ledger()
            };
            h.launch_with(true, Some(ledger));
            h.run_to_connected();
            h.forcibly_end(end, up);
            assert!(h.effects.contains(&Effect::CheckReadiness), "{end:?} {up}");
            assert_eq!(reason(&h), Some(FORCED));
            assert_eq!(h.timer(TimerKind::Retry), None);
            assert_eq!(
                h.cm.ledger.consecutive_auth_failures, failures_after,
                "{end:?} {up}"
            );
            h.run_to_connected();
        }
    }
}

#[test]
fn normal_exits_keep_their_rules() {
    let mut stable = Harness::new();
    stable.launch_connected();
    stable.advance(180.0);
    stable.exit(
        ended(Milestone::TunnelUp)
            .up(180.0)
            .status(ProcessExitStatus::Exited { code: 0 }),
    );
    waits(&stable, 5.0, RetryReason::ServerSessionEnded);

    let mut early = Harness::new();
    early.launch_connected();
    early.advance(30.0);
    early.exit(
        ended(Milestone::TunnelUp)
            .up(30.0)
            .status(ProcessExitStatus::Exited { code: 0 }),
    );
    waits(
        &early,
        90.0,
        RetryReason::AuthFailure(Failure::SessionDroppedEarly),
    );
    assert_eq!(early.cm.ledger.consecutive_auth_failures, 1);
}

#[test]
fn a_kill_before_the_tunnel_came_up_keeps_the_stage_rules() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::Authenticated);
    h.exit(killed(Milestone::Authenticated));
    waits(
        &h,
        90.0,
        RetryReason::AuthFailure(Failure::TunnelSetupFailed),
    );
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
}

#[test]
fn a_helper_stop_ended_by_a_signal_is_not_forced() {
    let mut h = Harness::new();
    h.launch_connected();
    h.advance(30.0);
    h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::HelperShutdown)
            .up(30.0)
            .status(ProcessExitStatus::Signaled { signal: 15 }),
    );
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::AuthFailure(Failure::SessionDroppedEarly))
    );
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
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
fn a_second_forced_end_before_a_stable_session_backs_off() {
    for end in FORCED_ENDS {
        let mut h = Harness::new();
        h.launch_connected();
        h.forcibly_end(ForcedEnd::ClientKilled, 600.0);
        assert_eq!(reason(&h), Some(FORCED));
        h.run_to_connected();
        h.forcibly_end(end, 30.0);
        waits(&h, 10.0, RetryReason::Network);
        assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
        // No session has been stable since the immediate reconnect.
        h.fire(TimerKind::Retry);
        h.run_to_connected();
        h.forcibly_end(ForcedEnd::ClientKilled, 60.0);
        waits(&h, 10.0, RetryReason::Network);
    }
}

#[test]
fn an_immediate_reconnect_that_loses_the_helper_while_connecting_backs_off() {
    let mut h = Harness::new();
    h.launch_connected();
    h.forcibly_end(ForcedEnd::HelperLost, 600.0);
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::ConnectedToGateway);
    h.send(Input::HelperConnectionLost);
    waits(&h, 10.0, RetryReason::Network);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
}

#[test]
fn a_stable_session_makes_the_next_forced_end_immediate_again() {
    for end in FORCED_ENDS {
        let mut h = Harness::new();
        h.launch_connected();
        h.forcibly_end(ForcedEnd::ClientKilled, 600.0);
        h.run_to_connected();
        h.forcibly_end(end, 120.0);
        assert_eq!(reason(&h), Some(FORCED), "{end:?}");
    }
}

#[test]
fn a_stable_session_the_app_replaced_also_counts() {
    let mut h = Harness::new();
    h.launch_connected();
    h.forcibly_end(ForcedEnd::ClientKilled, 600.0);
    h.run_to_connected();
    let id = h.connected().unwrap().attempt.clone();
    h.fire(TimerKind::ProactiveReconnect);
    h.exit_for(
        &id,
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::ProactiveReconnect)
            .up(175.0 * 60.0),
    );
    assert_eq!(reason(&h), Some(AttemptReason::Proactive));
    h.run_to_connected();
    h.forcibly_end(ForcedEnd::ClientKilled, 30.0);
    assert_eq!(reason(&h), Some(FORCED));
}

#[test]
fn a_user_connect_or_reconnect_starts_fresh() {
    for reconnect_now in [false, true] {
        let mut h = Harness::new();
        h.launch_connected();
        h.forcibly_end(ForcedEnd::ClientKilled, 600.0);
        h.run_to_connected();
        h.forcibly_end(ForcedEnd::ClientKilled, 30.0);
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
        h.run_to_connected();
        h.forcibly_end(ForcedEnd::ClientKilled, 30.0);
        assert_eq!(reason(&h), Some(FORCED), "reconnect now: {reconnect_now}");
    }
}

#[test]
fn a_kill_during_sleep_waits_for_the_wake() {
    let mut h = Harness::new();
    h.launch_connected();
    h.advance(600.0);
    h.sleep();
    h.forcibly_end(ForcedEnd::ClientKilled, 1200.0);
    assert_eq!(h.network_wait(), Some((RetryReason::Sleep, Duration::ZERO)));
    assert!(!h.effects.contains(&Effect::CheckReadiness));
    assert!(
        h.connection_timers().is_empty(),
        "{:?}",
        h.connection_timers()
    );
    h.advance(60.0);
    h.wake();
    let effects = h.settle(observe().dead());
    assert!(effects.contains(&Effect::CheckReadiness), "{effects:?}");
    assert_eq!(reason(&h), Some(AttemptReason::AfterSleep));
}

#[test]
fn a_kill_while_offline_waits_for_the_path_then_starts_at_once() {
    let mut h = Harness::new();
    h.launch_connected();
    h.advance(300.0);
    h.set_path(offline());
    assert!(h.connected().is_some());
    h.forcibly_end(ForcedEnd::ClientKilled, 600.0);
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
    let effects = h.set_path(online());
    assert!(effects.contains(&Effect::CheckReadiness), "{effects:?}");
    assert_eq!(reason(&h), Some(FORCED));
}

#[test]
fn a_kill_outside_the_hours_while_away_holds_the_reconnect() {
    let mut h = scheduled_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    h.advance(600.0);
    h.send(Input::ScreenLocked { at: h.now });
    h.forcibly_end(ForcedEnd::ClientKilled, 6000.0);
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
fn a_kill_after_reconnects_were_suspended_stays_off() {
    let mut h = scheduled_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    h.advance(600.0);
    h.send(Input::ScreenLocked { at: h.now });
    h.fire(TimerKind::AwayLimit);
    assert!(h.cm.reconnect_suspended);
    h.forcibly_end(ForcedEnd::HelperLost, 4.0 * 3600.0);
    assert_eq!(h.cm.phase, Phase::Idle);
    assert!(!h.cm.wants_connection);
}

#[test]
fn a_kill_at_the_hourly_attempt_cap_fails() {
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
    h.forcibly_end(ForcedEnd::ClientKilled, 600.0);
    assert_eq!(h.cm.phase, Phase::Failed(Failure::TooManyReconnects));
}
