//! Retries, failure counting and limits (SPEC F-CM-03, F-CM-04).

use std::time::Duration;

use fvm_contract::app_helper::{AttemptId, Milestone, StopReason};
use fvm_contract::app_internal::ButtonState;
use fvm_contract::app_internal::section::{ConnectionIcon, ConnectionStatus};

use super::super::machine::{AttemptReason, Phase, RetryReason};
use super::super::{Effect, Failure, Input, OtpFailure, TimerKind};
use super::harness::*;

/// Fails one attempt at the gateway (a wrong password) and returns its ID.
fn fail_at_gateway(h: &mut Harness) -> AttemptId {
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::ConnectedToGateway);
    h.exit(ended(Milestone::ConnectedToGateway));
    id
}

fn reason(h: &Harness) -> Option<AttemptReason> {
    h.attempt().map(|attempt| attempt.reason)
}

fn wait_reason(h: &Harness) -> Option<RetryReason> {
    h.retry_wait().map(|(_, reason)| reason)
}

#[test]
fn a_server_end_reconnects_after_five_seconds_and_resets_the_counter() {
    let mut h = Harness::new();
    h.launch();
    fail_at_gateway(&mut h);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
    h.fire(TimerKind::Retry);
    h.run_to_connected();
    h.advance(3.0 * 3600.0);
    let effects = h.exit(ended(Milestone::TunnelUp).up(3.0 * 3600.0));
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(5.0), RetryReason::ServerSessionEnded))
    );
    assert!(cancels(&effects, TimerKind::ProactiveReconnect));
    assert_eq!(
        timer_set(&effects, TimerKind::Retry),
        Some(h.now + secs(5.0))
    );
    assert_eq!(saved_ledger(&effects).unwrap().consecutive_auth_failures, 0);
    h.fire(TimerKind::Retry);
    assert_eq!(reason(&h), Some(retry(RetryReason::ServerSessionEnded)));
    h.run_to_connected();
}

#[test]
fn one_auth_failure_retries_after_ninety_seconds_and_the_second_fails() {
    let mut h = Harness::new();
    h.launch();
    fail_at_gateway(&mut h);
    let first_failure = h.now;
    let rejected = RetryReason::AuthFailure(Failure::AuthenticationRejected);
    assert_eq!(h.retry_wait(), Some((first_failure + secs(90.0), rejected)));
    assert_eq!(
        saved_ledger(&h.effects).unwrap().consecutive_auth_failures,
        1
    );
    let section = h.cm.connection_section();
    assert_eq!(section.icon, ConnectionIcon::InProgress);
    assert_eq!(section.reconnect_now, ButtonState::Enabled);
    h.fire(TimerKind::Retry);
    assert_eq!(h.now, first_failure + secs(90.0));
    assert_eq!(reason(&h), Some(retry(rejected)));
    fail_at_gateway(&mut h);
    assert_eq!(h.cm.phase, Phase::Failed(Failure::AuthenticationRejected));
    assert_eq!(
        saved_ledger(&h.effects).unwrap().consecutive_auth_failures,
        2
    );
    assert!(h.effects.contains(&Effect::HoldActivity(false)));
    assert!(
        h.connection_timers().is_empty(),
        "{:?}",
        h.connection_timers()
    );
    let section = h.cm.connection_section();
    assert_eq!(section.icon, ConnectionIcon::Error);
    assert!(
        matches!(section.status, ConnectionStatus::Failed { .. }),
        "{:?}",
        section.status
    );
    assert_eq!(section.connect, ButtonState::Enabled);
}

/// The retry time equals the ledger's cooldown end; a firing delivered a hair early must not be
/// sent back to the same, already fired, wait and stall.
#[test]
fn an_auth_retry_delivered_slightly_early_still_starts_the_attempt() {
    for early in [0.001, 0.005, 2.0] {
        let mut h = Harness::new();
        h.launch();
        fail_at_gateway(&mut h);
        let (until, _) = h.retry_wait().unwrap();
        assert_eq!(h.cm.ledger.cooldown_end(), Some(until));
        let effects = h.fire_early(TimerKind::Retry, early);
        assert!(effects.contains(&Effect::CheckReadiness), "{early}");
        assert_eq!(
            reason(&h),
            Some(retry(RetryReason::AuthFailure(
                Failure::AuthenticationRejected
            )))
        );
        assert_eq!(h.timer(TimerKind::Retry), None);
        h.run_to_connected();
    }
}

#[test]
fn a_network_retry_delivered_slightly_early_starts_the_attempt() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.exit(ended(Milestone::Spawned));
    assert_eq!(wait_reason(&h), Some(RetryReason::Network));
    assert!(
        h.fire_early(TimerKind::Retry, 0.005)
            .contains(&Effect::CheckReadiness)
    );
    assert_eq!(reason(&h), Some(retry(RetryReason::Network)));
}

#[test]
fn a_quick_drop_loop_counts_and_fails() {
    let mut h = Harness::new();
    h.launch_connected();
    h.advance(30.0);
    h.exit(ended(Milestone::TunnelUp).up(30.0));
    assert_eq!(
        wait_reason(&h),
        Some(RetryReason::AuthFailure(Failure::SessionDroppedEarly))
    );
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
    h.fire(TimerKind::Retry);
    h.run_to_connected();
    h.advance(45.0);
    h.exit(ended(Milestone::TunnelUp).up(45.0));
    assert_eq!(h.cm.phase, Phase::Failed(Failure::SessionDroppedEarly));
}

#[test]
fn network_failures_back_off_without_counting() {
    let mut h = Harness::new();
    h.launch();
    for expected in [10.0, 20.0, 40.0, 80.0, 160.0, 300.0, 300.0] {
        h.start_tunnel();
        h.milestone(Milestone::Spawned);
        let effects = h.exit(ended(Milestone::Spawned).by_helper(StopReason::Watchdog));
        assert_eq!(
            h.retry_wait(),
            Some((h.now + secs(expected), RetryReason::Network))
        );
        assert_eq!(saved_ledger(&effects), None);
        h.fire(TimerKind::Retry);
    }
    assert_eq!(h.cm.ledger, empty_ledger());
    // A tunnel coming up resets the backoff.
    h.run_to_connected();
    h.advance(600.0);
    h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::HelperShutdown)
            .up(600.0),
    );
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(10.0), RetryReason::Network))
    );
}

#[test]
fn a_network_failure_waits_for_the_path_then_backs_off() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.milestone(Milestone::Spawned);
    assert_eq!(connection_effects(&h.set_path(offline())), []);
    h.exit(ended(Milestone::Spawned));
    assert_eq!(
        h.network_wait(),
        Some((RetryReason::Network, Duration::from_secs(10)))
    );
    assert!(
        h.connection_timers().is_empty(),
        "{:?}",
        h.connection_timers()
    );
    assert_eq!(h.cm.connection_section().icon, ConnectionIcon::InProgress);
    h.advance(300.0);
    let effects = h.set_path(online());
    assert_eq!(
        timer_set(&effects, TimerKind::Retry),
        Some(h.now + secs(10.0))
    );
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(10.0), RetryReason::Network))
    );
    h.fire(TimerKind::Retry);
    assert_ne!(h.attempt().unwrap().id, id);
}

#[test]
fn a_retry_due_while_offline_waits_for_the_path_then_starts_at_once() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.exit(ended(Milestone::Spawned));
    h.set_path(offline());
    assert!(
        h.retry_wait().is_some(),
        "a retry wait ignores path changes until it is due"
    );
    h.fire(TimerKind::Retry);
    assert_eq!(
        h.network_wait(),
        Some((RetryReason::Network, Duration::ZERO))
    );
    let effects = h.set_path(online());
    assert!(effects.contains(&Effect::CheckReadiness));
    assert_eq!(reason(&h), Some(retry(RetryReason::Network)));
}

#[test]
fn a_gateway_failure_right_after_a_wake_is_a_network_error() {
    for (seconds_after_wake, is_network) in [(10.0, true), (30.0, true), (31.0, false)] {
        let mut h = Harness::new();
        h.launch_with(false, None);
        h.sleep();
        h.advance(600.0);
        h.wake();
        h.settle(observe().dead());
        assert_eq!(h.cm.phase, Phase::Idle);
        let settled_at = h.now;
        h.now = h.wake_time() + secs(seconds_after_wake);
        assert!(h.now >= settled_at, "{seconds_after_wake}");
        h.send(Input::Connect);
        fail_at_gateway(&mut h);
        if is_network {
            assert_eq!(
                wait_reason(&h),
                Some(RetryReason::Network),
                "{seconds_after_wake}"
            );
            assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
        } else {
            assert_eq!(
                wait_reason(&h),
                Some(RetryReason::AuthFailure(Failure::AuthenticationRejected))
            );
            assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
        }
    }
}

#[test]
fn a_gateway_failure_after_a_path_change_is_a_network_error() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::ConnectedToGateway);
    assert_eq!(
        connection_effects(&h.set_path(online_via(other_wifi()))),
        []
    );
    h.exit(ended(Milestone::ConnectedToGateway));
    assert_eq!(wait_reason(&h), Some(RetryReason::Network));
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
}

#[test]
fn a_gmail_auth_failure_fails_without_starting_the_client() {
    let mut h = Harness::new();
    h.launch();
    h.send(Input::ReadinessChecked(ready()));
    let id = h.attempt().unwrap().id.clone();
    let effects = connection_effects(&h.send(Input::OtpSessionFailed(
        id.clone(),
        OtpFailure::Authentication,
    )));
    assert_eq!(h.cm.phase, Phase::Failed(Failure::GmailRejected));
    assert!(run_starts(&effects).is_empty());
    assert_eq!(
        effects,
        [
            Effect::CancelTimer(TimerKind::AttemptDeadline),
            Effect::HoldActivity(false)
        ]
    );
    assert_eq!(connection_effects(&h.send(Input::OtpSessionReady(id))), []);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
}

#[test]
fn a_gmail_network_failure_backs_off() {
    let mut h = Harness::new();
    h.launch();
    h.send(Input::ReadinessChecked(ready()));
    let id = h.attempt().unwrap().id.clone();
    h.send(Input::OtpSessionFailed(id, OtpFailure::Network));
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(10.0), RetryReason::Network))
    );
}

#[test]
fn a_gmail_failure_while_the_tunnel_runs_stops_it() {
    for failure in [OtpFailure::Authentication, OtpFailure::Network] {
        let mut h = Harness::new();
        h.launch();
        let id = h.start_tunnel();
        h.run_milestones(&id, Milestone::OtpPrompted);
        assert_eq!(
            connection_effects(&h.send(Input::OtpSessionFailed(id.clone(), failure))),
            [
                Effect::StopRun {
                    attempt: id.clone(),
                    reason: StopReason::OtpUnavailable
                },
                Effect::CancelTimer(TimerKind::AttemptDeadline),
                Effect::CancelTimer(TimerKind::OtpDeadline),
            ]
        );
        h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::OtpUnavailable));
        match failure {
            OtpFailure::Authentication => {
                assert_eq!(h.cm.phase, Phase::Failed(Failure::GmailRejected))
            }
            OtpFailure::Network => assert_eq!(wait_reason(&h), Some(RetryReason::Network)),
        }
        assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
        assert_eq!(h.cm.ledger.recent_auth_attempts.len(), 1);
    }
}

#[test]
fn a_gmail_wrap_up_failure_after_submission_is_ignored() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpSubmitted);
    assert_eq!(
        connection_effects(&h.send(Input::OtpSessionFailed(id.clone(), OtpFailure::Network))),
        []
    );
    h.milestone(Milestone::Authenticated);
    h.milestone_for(&id, Milestone::TunnelUp, Some(TUNNEL_IP));
    assert!(h.connected().is_some());
}

#[test]
fn the_rolling_cap_stops_a_reconnect_loop() {
    let mut h = Harness::new();
    h.launch();
    for attempt in 1..=12 {
        h.run_to_connected();
        h.advance(130.0);
        h.exit(ended(Milestone::TunnelUp).up(130.0));
        assert_eq!(wait_reason(&h), Some(RetryReason::ServerSessionEnded));
        h.fire(TimerKind::Retry);
        if attempt < 12 {
            assert!(
                h.attempt().is_some(),
                "attempt {} should start",
                attempt + 1
            );
        }
    }
    assert_eq!(h.cm.phase, Phase::Failed(Failure::TooManyReconnects));
    let mut ledger = h.cm.ledger.clone();
    ledger.prune(h.now);
    assert_eq!(ledger.recent_auth_attempts.len(), 12);
    // A manual action clears the history and is let through.
    let effects = h.send(Input::Connect);
    assert!(effects.contains(&Effect::CheckReadiness));
    assert_eq!(saved_ledger(&effects).unwrap().recent_auth_attempts, []);
}

#[test]
fn the_rolling_cap_window_slides() {
    let mut h = Harness::new();
    h.launch();
    for _ in 1..=20 {
        h.run_to_connected();
        h.advance(400.0);
        h.exit(ended(Milestone::TunnelUp).up(400.0));
        h.fire(TimerKind::Retry);
        assert!(h.attempt().is_some());
    }
    let mut ledger = h.cm.ledger.clone();
    ledger.prune(h.now);
    assert!(ledger.recent_auth_attempts.len() < 12);
}

#[test]
fn failed_only_clears_on_a_user_action() {
    let mut h = Harness::new();
    h.launch();
    fail_at_gateway(&mut h);
    h.fire(TimerKind::Retry);
    fail_at_gateway(&mut h);
    let failed = Phase::Failed(Failure::AuthenticationRejected);
    assert_eq!(h.cm.phase, failed);
    h.advance(3600.0);
    let mut shorter_session = settings();
    shorter_session.connection.session_max_minutes = 60;
    let not_registered = super::super::ReadinessFacts {
        helper: super::super::HelperFacts::NotRegistered,
        ..ready()
    };
    h.set_path(offline());
    assert_eq!(h.cm.phase, failed, "offline");
    h.set_path(online());
    assert_eq!(h.cm.phase, failed, "online");
    let stale_retry = super::super::TimerId {
        kind: TimerKind::Retry,
        generation: 1,
    };
    for input in [
        Input::ReadinessChecked(ready()),
        Input::ReadinessChecked(not_registered),
        Input::SettingsChanged(shorter_session),
        Input::TimerFired(stale_retry),
        Input::HelperConnectionLost,
    ] {
        let label = format!("{input:?}");
        h.send(input);
        assert_eq!(h.cm.phase, failed, "{label} must not clear failed");
    }
    h.exit_for(&id(2), ended(Milestone::Spawned));
    assert_eq!(h.cm.phase, failed);
    h.sleep();
    h.advance(4000.0);
    h.wake();
    h.settle(observe().dead());
    assert_eq!(h.cm.phase, failed, "sleep and wake");
    let effects = h.send(Input::Connect);
    assert!(effects.contains(&Effect::CheckReadiness));
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
    assert!(h.cm.ledger.recent_auth_attempts.is_empty());
    assert!(h.cm.wants_connection);
}

#[test]
fn connecting_right_after_a_failure_waits_out_the_ninety_seconds() {
    let mut h = Harness::new();
    h.launch();
    fail_at_gateway(&mut h);
    h.fire(TimerKind::Retry);
    fail_at_gateway(&mut h);
    let last_failure = h.now;
    h.advance(10.0);
    let effects = h.send(Input::Connect);
    assert_eq!(
        h.retry_wait(),
        Some((last_failure + secs(90.0), RetryReason::AuthCooldown))
    );
    assert_eq!(saved_ledger(&effects).unwrap().consecutive_auth_failures, 0);
    assert!(!effects.contains(&Effect::CheckReadiness));
    h.fire(TimerKind::Retry);
    assert_eq!(reason(&h), Some(retry(RetryReason::AuthCooldown)));
}

#[test]
fn reconnect_now_clears_failed() {
    let mut h = Harness::new();
    h.launch();
    h.send(Input::ReadinessChecked(ready()));
    let id = h.current_id();
    h.send(Input::OtpSessionFailed(id, OtpFailure::Authentication));
    assert_eq!(h.cm.phase, Phase::Failed(Failure::GmailRejected));
    assert_ne!(
        h.cm.connection_section().reconnect_now,
        ButtonState::Enabled
    );
    h.send(Input::ReconnectNow);
    assert_eq!(reason(&h), Some(AttemptReason::ReconnectNow));
}

#[test]
fn a_stable_session_resets_the_counter_even_on_a_user_disconnect() {
    let mut h = Harness::new();
    h.launch();
    fail_at_gateway(&mut h);
    h.fire(TimerKind::Retry);
    h.run_to_connected();
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
    h.advance(200.0);
    assert_eq!(
        stop_reasons(&h.send(Input::Disconnect)),
        [StopReason::UserDisconnect]
    );
    let effects = h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::UserDisconnect)
            .up(200.0),
    );
    assert_eq!(saved_ledger(&effects).unwrap().consecutive_auth_failures, 0);
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn a_short_session_ended_by_the_user_keeps_the_counter() {
    let mut h = Harness::new();
    h.launch();
    fail_at_gateway(&mut h);
    h.fire(TimerKind::Retry);
    h.run_to_connected();
    h.advance(60.0);
    h.send(Input::Disconnect);
    let effects = h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::UserDisconnect)
            .up(60.0),
    );
    assert_eq!(saved_ledger(&effects), None);
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
}

#[test]
fn every_ledger_change_is_saved() {
    /// Sends `input`; true when the ledger changed, in which case it must be saved.
    fn step(h: &mut Harness, input: Input) -> bool {
        let before = h.cm.ledger.clone();
        let label = format!("{input:?}");
        let effects = h.send(input);
        let changed = h.cm.ledger != before;
        if changed {
            assert_eq!(
                saved_ledger(&effects).as_ref(),
                Some(&h.cm.ledger),
                "{label}"
            );
        } else {
            assert_eq!(saved_ledger(&effects), None, "{label}");
        }
        changed
    }
    let mut h = Harness::new();
    let first = id(1);
    let mut changes = 0;
    for input in [
        Input::Launched(super::super::Launch {
            auto_connect: true,
            restored_ledger: None,
            import_report_shown: false,
        }),
        Input::ReadinessChecked(ready()),
        Input::OtpSessionReady(first.clone()),
        Input::Helper(fvm_contract::app_helper::HelperEvent {
            attempt_id: first.clone(),
            kind: fvm_contract::app_helper::HelperEventKind::Milestone {
                milestone: Milestone::ConnectedToGateway,
                tunnel_ip: None,
            },
        }),
        Input::Helper(fvm_contract::app_helper::HelperEvent {
            attempt_id: first.clone(),
            kind: fvm_contract::app_helper::HelperEventKind::Exited {
                facts: ended(Milestone::ConnectedToGateway).facts(),
            },
        }),
    ] {
        changes += usize::from(step(&mut h, input));
    }
    h.advance(90.0);
    let retry_timer = h.timer_id(TimerKind::Retry);
    let mut other_account = settings();
    other_account.account = bob();
    for input in [
        Input::TimerFired(retry_timer),
        Input::ReconnectNow, // ignored while connecting
        Input::Disconnect,
        Input::Connect, // a user action resets
        Input::SettingsChanged(other_account),
    ] {
        changes += usize::from(step(&mut h, input));
    }
    assert_eq!(changes, 4);
}

#[test]
fn a_launch_honours_a_restored_cooldown() {
    let mut h = Harness::new();
    let ledger = super::super::FailureLedger {
        consecutive_auth_failures: 1,
        last_auth_failure_at: Some(h.now - secs(30.0)),
        last_auth_failure: Some(Failure::OtpRejected),
        recent_auth_attempts: vec![h.now - secs(40.0)],
        ..empty_ledger()
    };
    let effects = connection_effects(&h.launch_with(true, Some(ledger.clone())));
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(60.0), RetryReason::AuthCooldown))
    );
    assert_eq!(
        effects,
        [
            h.set(TimerKind::Retry, h.now + secs(60.0)),
            Effect::HoldActivity(true)
        ]
    );
    assert_eq!(h.cm.ledger, ledger);
}

#[test]
fn a_launch_stays_failed_when_the_restored_counter_is_used_up() {
    let mut h = Harness::new();
    let in_spam = Failure::OtpNotReceived { in_spam: true };
    let ledger = super::super::FailureLedger {
        consecutive_auth_failures: 2,
        last_auth_failure_at: Some(h.now - secs(86_400.0)),
        last_auth_failure: Some(in_spam),
        ..empty_ledger()
    };
    let effects = connection_effects(&h.launch_with(true, Some(ledger)));
    assert_eq!(h.cm.phase, Phase::Failed(in_spam));
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(h.cm.connection_section().connect, ButtonState::Enabled);
}

#[test]
fn a_launch_enforces_the_restored_rolling_cap() {
    let mut h = Harness::new();
    let mut attempts: Vec<_> = (1..=12).map(|n| h.now - secs(n as f64 * 60.0)).collect();
    attempts.reverse();
    let effects = connection_effects(&h.launch_with(
        true,
        Some(super::super::FailureLedger {
            recent_auth_attempts: attempts,
            ..empty_ledger()
        }),
    ));
    assert_eq!(h.cm.phase, Phase::Failed(Failure::TooManyReconnects));
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn a_launch_discards_a_ledger_of_another_account_and_prunes_old_attempts() {
    let mut other = Harness::new();
    let foreign = super::super::FailureLedger {
        account_key: "bob@vpn.example.com".into(),
        consecutive_auth_failures: 2,
        ..empty_ledger()
    };
    other.launch_with(true, Some(foreign));
    assert_eq!(other.cm.ledger, empty_ledger());
    assert!(other.attempt().is_some());
    let mut stale = Harness::new();
    let old = super::super::FailureLedger {
        recent_auth_attempts: vec![stale.now - secs(7200.0)],
        ..empty_ledger()
    };
    let effects = stale.launch_with(true, Some(old));
    assert_eq!(saved_ledger(&effects), Some(empty_ledger()));
}

#[test]
fn connect_rejections_are_classified_by_code() {
    use fvm_contract::app_helper::HelperErrorCode as Code;
    for (code, failure) in [
        (Code::Busy, None),
        (Code::WrongPhase, None),
        (Code::UnknownAttempt, None),
        (Code::InternalError, None),
        (
            Code::ClientUnavailable,
            Some(Failure::Helper(Code::ClientUnavailable)),
        ),
        (
            Code::InvalidRequest,
            Some(Failure::Helper(Code::InvalidRequest)),
        ),
        (
            Code::NotAuthorized,
            Some(Failure::Helper(Code::NotAuthorized)),
        ),
    ] {
        let mut h = Harness::new();
        h.launch();
        let id = h.start_tunnel();
        let effects = connection_effects(&h.send(Input::RunRejected(id.clone(), code)));
        assert_eq!(
            effects.first(),
            Some(&Effect::CloseOtpSession {
                attempt: id,
                check_spam: false
            }),
            "{code:?}"
        );
        match failure {
            Some(failure) => assert_eq!(h.cm.phase, Phase::Failed(failure)),
            None => assert_eq!(wait_reason(&h), Some(RetryReason::Network), "{code:?}"),
        }
    }
}

#[test]
fn losing_the_helper_connection_ends_the_run() {
    let mut connected = Harness::new();
    connected.launch_connected();
    connected.send(Input::HelperConnectionLost);
    assert_eq!(
        reason(&connected),
        Some(retry(RetryReason::ForcedTermination))
    );

    let mut connecting = Harness::new();
    connecting.launch();
    let id = connecting.start_tunnel();
    let effects = connection_effects(&connecting.send(Input::HelperConnectionLost));
    assert_eq!(
        effects.first(),
        Some(&Effect::CloseOtpSession {
            attempt: id,
            check_spam: false
        })
    );
    assert_eq!(
        reason(&connecting),
        Some(retry(RetryReason::ForcedTermination))
    );

    let mut stopping = Harness::new();
    stopping.launch_connected();
    stopping.send(Input::Disconnect);
    stopping.send(Input::HelperConnectionLost);
    assert_eq!(stopping.cm.phase, Phase::Idle);

    let mut idle = Harness::new();
    assert_eq!(
        connection_effects(&idle.send(Input::HelperConnectionLost)),
        []
    );
}
