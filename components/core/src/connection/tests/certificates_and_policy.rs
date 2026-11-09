//! Gateway certificate auto-trust, retry values and the failure ledger (SPEC F-CM-04, F-CM-09).

use fvm_contract::app_helper::{Milestone, StopReason};

use super::super::certs::{gateway_change_drops_certs, merge_trusted_certs};
use super::super::machine::{AttemptReason, Phase, RetryReason};
use super::super::policy::{network_backoff, planned_reconnect};
use super::super::{Effect, Failure, FailureLedger, Input, Settings, TimerKind, VpnAccount};
use super::harness::*;

fn reason(h: &Harness) -> Option<AttemptReason> {
    h.attempt().map(|attempt| attempt.reason)
}

fn started_certs(h: &Harness) -> Vec<String> {
    run_starts(&h.effects)
        .first()
        .expect("a run start")
        .trusted_certs
        .clone()
}

fn with_account(account: VpnAccount) -> Settings {
    Settings {
        account,
        ..settings()
    }
}

fn rejected(certs: &[String]) -> Exit {
    ended(Milestone::Spawned).certs(certs, Some(Milestone::Spawned))
}

#[test]
fn a_rejection_learns_the_value_and_retries_at_once_without_counting() {
    let mut h = Harness::new();
    h.launch();
    let first = h.start_tunnel();
    assert_eq!(run_starts(&h.effects), [account()]);
    h.advance(1.0);
    h.milestone(Milestone::Spawned);
    let effects = connection_effects(&h.exit(rejected(&[digest('a'), digest('a')])));
    let second = id(2);
    assert_eq!(
        effects,
        [
            Effect::SaveTrustedCerts(vec![digest('a'), pin_old()]),
            Effect::CloseOtpSession {
                attempt: first,
                check_spam: false
            },
            Effect::CheckReadiness,
            h.set(TimerKind::AttemptDeadline, h.now + secs(150.0)),
        ]
    );
    assert_eq!(h.attempt().unwrap().id, second);
    assert_eq!(reason(&h), Some(AttemptReason::CertificateRetry));
    assert_eq!(h.cm.ledger, empty_ledger());
    // The learned value goes with the very next run request: one client run from now on.
    h.start_tunnel();
    assert_eq!(started_certs(&h), [digest('a'), pin_old()]);
    h.run_milestones(&second, Milestone::TunnelUp);
    assert!(h.connected().is_some());
    // ...and with the one after that.
    h.send(Input::ReconnectNow);
    h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::Reconnect)
            .up(30.0),
    );
    h.start_tunnel();
    assert_eq!(started_certs(&h), [digest('a'), pin_old()]);
}

#[test]
fn a_second_rejection_in_the_same_chain_is_a_network_error() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.exit(rejected(&[digest('a')]));
    h.start_tunnel();
    let effects = h.exit(rejected(&[digest('b')]));
    assert_eq!(
        saved_certs(&effects),
        Some(vec![digest('b'), digest('a'), pin_old()])
    );
    assert_eq!(
        h.retry_wait(),
        Some((h.now + secs(10.0), RetryReason::Network))
    );
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
    // A new chain gets its own single immediate retry.
    h.fire(TimerKind::Retry);
    h.start_tunnel();
    h.exit(rejected(&[digest('c')]));
    assert_eq!(reason(&h), Some(AttemptReason::CertificateRetry));
}

#[test]
fn a_rejection_after_the_otp_submission_counts_and_still_learns() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpSubmitted);
    let effects =
        h.exit(ended(Milestone::OtpSubmitted).certs(&[digest('a')], Some(Milestone::OtpSubmitted)));
    assert_eq!(saved_certs(&effects), Some(vec![digest('a'), pin_old()]));
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::AuthFailure(
            Failure::CertificateRejectedAfterLogin
        ))
    );
    assert_eq!(saved_ledger(&effects).unwrap().consecutive_auth_failures, 1);
}

#[test]
fn values_are_capped_at_four_newest_first() {
    let mut account = account();
    account.trusted_certs = vec![digest('a'), digest('b'), digest('c'), digest('d')];
    let mut h = Harness::with_settings(with_account(account));
    h.launch();
    h.start_tunnel();
    let effects = h.exit(rejected(&[pin_old()]));
    let expected = vec![pin_old(), digest('a'), digest('b'), digest('c')];
    assert_eq!(saved_certs(&effects), Some(expected.clone()));
    assert_eq!(h.cm.settings.account.trusted_certs, expected);
}

#[test]
fn values_learned_from_a_stopped_run_are_kept() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.send(Input::Disconnect);
    let effects = h.exit_for(
        &id,
        ended(Milestone::Spawned)
            .by_helper(StopReason::UserDisconnect)
            .certs(&[digest('a')], Some(Milestone::Spawned)),
    );
    assert_eq!(saved_certs(&effects), Some(vec![digest('a'), pin_old()]));
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn a_gateway_change_drops_the_values() {
    let moved = |change: fn(&mut VpnAccount)| {
        let mut after = account();
        change(&mut after);
        after
    };
    for (field, after, drops) in [
        ("host", moved(|a| a.host = "vpn2.example.com".into()), true),
        ("port", moved(|a| a.port = 443), true),
        ("username", moved(|a| a.username = "bob".into()), false),
        ("realm", moved(|a| a.realm = "staff".into()), false),
    ] {
        assert_eq!(
            gateway_change_drops_certs(&account(), &after),
            drops,
            "{field}"
        );
    }
    // Values the user supplied for the new gateway are kept.
    let supplied = moved(|a| {
        a.host = "vpn2.example.com".into();
        a.trusted_certs = vec![digest('b')];
    });
    assert!(!gateway_change_drops_certs(&account(), &supplied));
}

#[test]
fn hand_edited_values_are_used_for_the_next_connect() {
    let mut h = Harness::new();
    h.launch_with(false, None);
    let mut edited = account();
    edited.trusted_certs = vec![digest('c')];
    assert_eq!(
        connection_effects(&h.send(Input::SettingsChanged(with_account(edited)))),
        []
    );
    h.send(Input::Connect);
    h.start_tunnel();
    assert_eq!(started_certs(&h), [digest('c')]);
}

/// A run started for the old gateway must not plant its certificate in the new gateway's values,
/// and its rejection does not use up the new gateway's one immediate retry.
#[test]
fn a_rejection_from_the_old_gateway_is_not_learned_for_the_new_one() {
    for run_started in [false, true] {
        let mut h = Harness::new();
        h.launch();
        h.send(Input::ReadinessChecked(ready()));
        let first = h.attempt().unwrap().id.clone();
        if run_started {
            h.send(Input::OtpSessionReady(first.clone()));
        }
        let mut moved = account();
        moved.host = "vpn2.example.com".into();
        moved.trusted_certs = vec![];
        h.send(Input::SettingsChanged(with_account(moved)));
        if !run_started {
            let starts = run_starts(&h.send(Input::OtpSessionReady(first.clone())));
            assert_eq!(
                starts.iter().map(|a| a.host.as_str()).collect::<Vec<_>>(),
                ["vpn.example.com"]
            );
        }
        let effects = h.exit_for(&first, rejected(&[digest('a')]));
        assert_eq!(saved_certs(&effects), None, "run started: {run_started}");
        assert!(h.cm.settings.account.trusted_certs.is_empty());
        assert_eq!(reason(&h), Some(AttemptReason::CertificateRetry));
        // The new gateway's own rejection is learned and retried once.
        let second = h.start_tunnel();
        assert_eq!(run_starts(&h.effects)[0].host, "vpn2.example.com");
        let effects = h.exit_for(&second, rejected(&[digest('b')]));
        assert_eq!(saved_certs(&effects), Some(vec![digest('b')]));
        assert_eq!(
            reason(&h),
            Some(AttemptReason::CertificateRetry),
            "the new gateway still has its own retry"
        );
        h.start_tunnel();
        assert_eq!(started_certs(&h), [digest('b')]);
    }
}

// MARK: Values

#[test]
fn the_network_backoff_doubles_up_to_five_minutes() {
    for (failures, expected) in [
        (1, 10),
        (2, 20),
        (3, 40),
        (4, 80),
        (5, 160),
        (6, 300),
        (7, 300),
        (60, 300),
        (0, 10),
    ] {
        assert_eq!(
            network_backoff(failures),
            secs(f64::from(expected)),
            "{failures}"
        );
    }
}

#[test]
fn the_planned_reconnect_is_the_session_maximum_less_the_margin() {
    let settings = connection_settings();
    assert_eq!(
        planned_reconnect(t0(), &settings, t0()),
        Some(t0() + secs(175.0 * 60.0))
    );
    let off = super::super::ConnectionSettings {
        proactive_reconnect: false,
        ..connection_settings()
    };
    assert_eq!(planned_reconnect(t0(), &off, t0()), None);
    let short = super::super::ConnectionSettings {
        session_max_minutes: 10,
        ..connection_settings()
    };
    let now = t0() + secs(20.0 * 60.0);
    assert_eq!(planned_reconnect(t0(), &short, now), Some(now + secs(60.0)));
    // A margin longer than the session cannot give a time in the past either.
    let silly = super::super::ConnectionSettings {
        session_max_minutes: 5,
        proactive_margin_minutes: 10,
        ..connection_settings()
    };
    assert_eq!(
        planned_reconnect(t0(), &silly, t0()),
        Some(t0() + secs(60.0))
    );
}

#[test]
fn the_rolling_window_leaves_out_attempts_an_hour_old() {
    let now = t0() + secs(7_200.0);
    let mut ledger = FailureLedger {
        recent_auth_attempts: vec![now - secs(3_600.0), now - secs(3_599.0), now - secs(10.0)],
        ..FailureLedger::new("k".into())
    };
    ledger.prune(now);
    assert_eq!(
        ledger.recent_auth_attempts,
        [now - secs(3_599.0), now - secs(10.0)]
    );
}

#[test]
fn the_cooldown_ends_ninety_seconds_after_the_last_auth_failure() {
    let mut ledger = FailureLedger::new("k".into());
    assert_eq!(ledger.cooldown_end(), None);
    ledger.record_auth_failure(Failure::OtpRejected, t0());
    assert_eq!(ledger.cooldown_end(), Some(t0() + secs(90.0)));
    assert_eq!(ledger.consecutive_auth_failures, 1);
    assert_eq!(ledger.last_auth_failure, Some(Failure::OtpRejected));
}

#[test]
fn a_user_reset_keeps_the_cooldown() {
    let mut ledger = FailureLedger {
        consecutive_auth_failures: 2,
        last_auth_failure_at: Some(t0()),
        last_auth_failure: Some(Failure::OtpRejected),
        recent_auth_attempts: vec![t0()],
        ..FailureLedger::new("k".into())
    };
    ledger.reset_for_user_action();
    assert_eq!(ledger.consecutive_auth_failures, 0);
    assert!(ledger.recent_auth_attempts.is_empty());
    assert_eq!(ledger.cooldown_end(), Some(t0() + secs(90.0)));
}

#[test]
fn merging_puts_the_newest_first_and_keeps_four() {
    let existing = vec![digest('a'), digest('b'), digest('c'), digest('d')];
    assert_eq!(
        merge_trusted_certs(&[pin_old()], &existing),
        [pin_old(), digest('a'), digest('b'), digest('c')]
    );
    // Learning a value again moves it to the front instead of repeating it.
    assert_eq!(
        merge_trusted_certs(&[digest('c')], &existing),
        [digest('c'), digest('a'), digest('b'), digest('d')]
    );
    assert!(merge_trusted_certs(&[], &[]).is_empty());
}

#[test]
fn the_run_request_carries_the_values() {
    let mut account = account();
    account.trusted_certs = vec![digest('a'), pin_old()];
    let request = account.run_request(id(1), "pw".into());
    assert_eq!(request.attempt_id, id(1));
    assert_eq!(request.host, "vpn.example.com");
    assert_eq!(request.port, 8443);
    assert_eq!(request.username, "alice");
    assert_eq!(request.realm, None);
    assert_eq!(request.trusted_certs, [digest('a'), pin_old()]);
    account.realm = "staff".into();
    assert_eq!(
        account.run_request(id(1), "pw".into()).realm.as_deref(),
        Some("staff")
    );
    assert_eq!(account.ledger_key(), "alice@vpn.example.com");
}
