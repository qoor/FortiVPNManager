//! Sleep, wake and network path changes (SPEC F-CM-06, F-CM-07).

use std::time::Duration;

use fvm_contract::app_helper::{Milestone, StopReason};

use super::super::machine::{AfterStop, AttemptReason, RetryReason};
use super::super::{Effect, Input, Settings, TimerKind};
use super::harness::*;

fn reason(h: &Harness) -> Option<AttemptReason> {
    h.attempt().map(|attempt| attempt.reason)
}

fn after(h: &Harness) -> Option<AfterStop> {
    h.stopping().map(|stop| stop.after)
}

fn after_sleep_on() -> Settings {
    let mut settings = settings();
    settings.connection.reconnect_after_sleep = true;
    settings.connection.min_sleep_minutes = 30;
    settings
}

struct WakeCase {
    name: &'static str,
    settings: Settings,
    sleep: f64,
    observation: Observe,
    /// `None` keeps the tunnel.
    expected: Option<AttemptReason>,
}

fn case(
    name: &'static str,
    sleep: f64,
    observation: Observe,
    expected: Option<AttemptReason>,
) -> WakeCase {
    WakeCase {
        name,
        settings: settings(),
        sleep,
        observation,
        expected,
    }
}

/// Connected at t0+12; the planned reconnect is 175 minutes later.
fn wake_cases() -> Vec<WakeCase> {
    use AttemptReason::*;
    vec![
        case(
            "1. planned time passed",
            4.0 * 3600.0,
            observe(),
            Some(Proactive),
        ),
        case(
            "1 beats 2: planned passed and process gone",
            4.0 * 3600.0,
            observe().alive(false),
            Some(Proactive),
        ),
        case(
            "2. process gone",
            600.0,
            observe().alive(false),
            Some(TunnelLost),
        ),
        case(
            "2. tunnel interface gone",
            600.0,
            observe().tunnel(false),
            Some(TunnelLost),
        ),
        case(
            "2 beats 3: process gone and underlay changed",
            600.0,
            observe().alive(false).via(Some(ethernet())),
            Some(TunnelLost),
        ),
        case(
            "3. underlay interface changed",
            600.0,
            observe().via(Some(ethernet())),
            Some(NetworkChanged),
        ),
        case(
            "3. same interface, new address",
            600.0,
            observe().via(Some(other_wifi())),
            Some(NetworkChanged),
        ),
        WakeCase {
            settings: after_sleep_on(),
            ..case(
                "3 beats 4: underlay changed after a long sleep",
                3600.0,
                observe().via(Some(ethernet())),
                Some(NetworkChanged),
            )
        },
        WakeCase {
            settings: after_sleep_on(),
            ..case("4. slept 31 min", 31.0 * 60.0, observe(), Some(AfterSleep))
        },
        WakeCase {
            settings: after_sleep_on(),
            ..case(
                "4. slept exactly the minimum",
                30.0 * 60.0,
                observe(),
                Some(AfterSleep),
            )
        },
        WakeCase {
            settings: after_sleep_on(),
            ..case("5. a short sleep keeps", 10.0 * 60.0, observe(), None)
        },
        case(
            "5. reconnect after sleep off keeps",
            60.0 * 60.0,
            observe(),
            None,
        ),
        case(
            "5. underlay unknown after the wake keeps",
            600.0,
            observe().via(None),
            None,
        ),
        case(
            "not settled within 30 s keeps even with a dead process",
            600.0,
            observe().alive(false).tunnel(false).via(None).unsettled(),
            None,
        ),
    ]
}

#[test]
fn wake_rules_while_connected() {
    for wake in wake_cases() {
        let mut h = Harness::with_settings(wake.settings);
        h.launch_connected();
        let id = h.connected().unwrap().attempt.clone();
        h.advance(100.0);
        assert_eq!(
            connection_effects(&h.sleep()),
            [Effect::CancelTimer(TimerKind::ProactiveReconnect)],
            "{}",
            wake.name
        );
        h.advance(wake.sleep);
        h.wake();
        let effects = h.settle(wake.observation);
        match wake.expected {
            Some(reason) => {
                let stop = if reason == AttemptReason::Proactive {
                    StopReason::ProactiveReconnect
                } else {
                    StopReason::Reconnect
                };
                assert_eq!(stop_reasons(&effects), [stop], "{}", wake.name);
                assert_eq!(
                    after(&h),
                    Some(AfterStop::Reconnect(reason)),
                    "{}",
                    wake.name
                );
                let up = h.now.duration_since(t0()).unwrap().as_secs_f64();
                h.exit_for(&id, ended(Milestone::TunnelUp).by_helper(stop).up(up));
                assert_eq!(self::reason(&h), Some(reason), "{}", wake.name);
            }
            None => {
                assert!(stop_reasons(&effects).is_empty(), "{}", wake.name);
                assert!(h.connected().is_some(), "{}", wake.name);
                // The proactive timer is set again.
                assert!(
                    h.timer(TimerKind::ProactiveReconnect).is_some(),
                    "{}",
                    wake.name
                );
            }
        }
    }
}

#[test]
fn an_unsettled_wake_after_the_planned_time_defers_the_proactive_reconnect() {
    let mut h = Harness::new();
    h.launch_connected();
    h.sleep();
    h.advance(4.0 * 3600.0);
    h.wake();
    let effects = h.settle(observe().via(None).unsettled());
    assert_eq!(
        timer_set(&effects, TimerKind::ProactiveReconnect),
        Some(h.now + secs(60.0))
    );
    assert_eq!(
        h.connected().unwrap().planned_reconnect,
        Some(h.now + secs(60.0))
    );
    assert_eq!(
        stop_reasons(&h.fire(TimerKind::ProactiveReconnect)),
        [StopReason::ProactiveReconnect]
    );
    assert_eq!(
        after(&h),
        Some(AfterStop::Reconnect(AttemptReason::Proactive))
    );
}

#[test]
fn the_proactive_timer_is_suspended_until_the_wake_is_judged() {
    let mut h = Harness::new();
    h.launch_connected();
    let planned = h.connected().unwrap().planned_reconnect.unwrap();
    let old = h.timer_id(TimerKind::ProactiveReconnect);
    h.sleep();
    h.advance(60.0);
    h.wake();
    assert_eq!(h.timer(TimerKind::ProactiveReconnect), None);
    let effects = h.settle(observe());
    assert_eq!(
        timer_set(&effects, TimerKind::ProactiveReconnect),
        Some(planned)
    );
    assert_eq!(connection_effects(&h.send(Input::TimerFired(old))), []);
    assert!(h.connected().is_some());
}

#[test]
fn sleep_while_connecting_stops_the_attempt_and_resumes_after_the_wake() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    assert_eq!(
        connection_effects(&h.sleep()),
        [
            Effect::StopRun {
                attempt: id.clone(),
                reason: StopReason::Sleep
            },
            Effect::CloseOtpSession {
                attempt: id,
                check_spam: false
            },
            Effect::CancelTimer(TimerKind::AttemptDeadline),
            Effect::CancelTimer(TimerKind::OtpDeadline),
        ]
    );
    assert_eq!(after(&h), Some(AfterStop::ResumeAfterSleep));
    let exit = h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::Sleep));
    assert_eq!(h.network_wait(), Some((RetryReason::Sleep, Duration::ZERO)));
    assert_eq!(
        saved_ledger(&exit),
        None,
        "an attempt ended by a sleep is not an authentication failure"
    );
    h.advance(600.0);
    h.wake();
    assert!(h.network_wait().is_some());
    let resumed = h.settle(observe().dead());
    assert!(resumed.contains(&Effect::CheckReadiness), "{resumed:?}");
    assert_eq!(reason(&h), Some(AttemptReason::AfterSleep));
}

#[test]
fn a_wake_without_network_keeps_waiting() {
    let mut h = Harness::new();
    h.launch();
    h.sleep();
    h.wake();
    assert!(!checks_readiness(
        &h.settle(observe().dead().via(None).unsettled())
    ));
    assert_eq!(
        h.network_wait().map(|(reason, _)| reason),
        Some(RetryReason::Sleep)
    );
    assert!(h.set_path(online()).contains(&Effect::CheckReadiness));
}

#[test]
fn sleep_before_the_tunnel_started_just_waits() {
    for session_open in [false, true] {
        let mut h = Harness::new();
        h.launch();
        if session_open {
            h.send(Input::ReadinessChecked(ready()));
        }
        let id = h.attempt().unwrap().id.clone();
        let effects = connection_effects(&h.sleep());
        let mut expected = Vec::new();
        if session_open {
            expected.push(Effect::CloseOtpSession {
                attempt: id.clone(),
                check_spam: false,
            });
        }
        expected.push(Effect::CancelTimer(TimerKind::AttemptDeadline));
        assert_eq!(effects, expected, "session open: {session_open}");
        assert_eq!(h.network_wait(), Some((RetryReason::Sleep, Duration::ZERO)));
        assert_eq!(connection_effects(&h.send(Input::OtpSessionReady(id))), []);
    }
}

#[test]
fn sleep_while_waiting_cancels_the_timer_but_keeps_the_auth_gap() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::ConnectedToGateway);
    h.exit(ended(Milestone::ConnectedToGateway));
    let failed_at = h.now;
    assert_eq!(
        connection_effects(&h.sleep()),
        [Effect::CancelTimer(TimerKind::Retry)]
    );
    assert_eq!(
        h.network_wait().map(|(reason, _)| reason),
        Some(RetryReason::Sleep)
    );
    h.advance(20.0);
    h.wake();
    h.settle(observe());
    assert_eq!(
        h.retry_wait(),
        Some((failed_at + secs(90.0), RetryReason::AuthCooldown))
    );
    h.fire(TimerKind::Retry);
    assert!(h.attempt().is_some());
}

#[test]
fn a_tunnel_that_dies_during_sleep_reconnects_after_the_wake() {
    let mut h = Harness::new();
    h.launch_connected();
    h.sleep();
    h.advance(3.0 * 3600.0);
    h.exit(ended(Milestone::TunnelUp).up(3.0 * 3600.0));
    assert_eq!(
        h.network_wait().map(|(reason, _)| reason),
        Some(RetryReason::Sleep)
    );
    assert!(
        h.connection_timers().is_empty(),
        "{:?}",
        h.connection_timers()
    );
    h.wake();
    assert!(h.settle(observe().dead()).contains(&Effect::CheckReadiness));
}

#[test]
fn sleep_during_a_planned_reconnect_waits_for_the_wake() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    h.fire(TimerKind::ProactiveReconnect);
    h.sleep();
    assert_eq!(after(&h), Some(AfterStop::ResumeAfterSleep));
    h.exit_for(
        &id,
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::ProactiveReconnect)
            .up(10_500.0),
    );
    assert_eq!(
        h.network_wait().map(|(reason, _)| reason),
        Some(RetryReason::Sleep)
    );
}

// MARK: Network path while connected

#[test]
fn an_underlay_change_reconnects() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    let effects = connection_effects(&h.set_path(online_via(ethernet())));
    assert_eq!(
        effects,
        [
            Effect::StopRun {
                attempt: id,
                reason: StopReason::Reconnect
            },
            Effect::CancelTimer(TimerKind::ProactiveReconnect),
        ]
    );
    assert_eq!(
        after(&h),
        Some(AfterStop::Reconnect(AttemptReason::NetworkChanged))
    );
    h.exit(
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::Reconnect)
            .up(500.0),
    );
    assert_eq!(reason(&h), Some(AttemptReason::NetworkChanged));
}

#[test]
fn other_path_reports_keep_the_tunnel() {
    for path in [
        online(),
        offline(),
        Path {
            satisfied: true,
            underlay: None,
        },
        Path {
            satisfied: false,
            underlay: Some(ethernet()),
        },
    ] {
        let mut h = Harness::new();
        h.launch_connected();
        assert_eq!(connection_effects(&h.set_path(path)), []);
        assert!(h.connected().is_some());
        // The underlay is still the one the session started on.
        assert!(stop_reasons(&h.set_path(online())).is_empty());
    }
}

#[test]
fn a_flap_back_to_the_same_underlay_keeps_the_tunnel() {
    let mut h = Harness::new();
    h.launch_connected();
    h.set_path(offline());
    assert_eq!(connection_effects(&h.set_path(online())), []);
    assert!(h.connected().is_some());
}

#[test]
fn an_unknown_underlay_is_adopted_then_watched() {
    let mut h = Harness::with(settings(), None);
    h.launch_connected();
    assert_eq!(connection_effects(&h.set_path(online())), []);
    assert_eq!(
        stop_reasons(&h.set_path(online_via(ethernet()))),
        [StopReason::Reconnect]
    );
}

#[test]
fn path_reports_are_ignored_while_connecting_and_disconnecting() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    assert_eq!(connection_effects(&h.set_path(online_via(ethernet()))), []);
    assert_eq!(connection_effects(&h.set_path(offline())), []);
    assert!(h.attempt().is_some());
    let mut stopping = Harness::new();
    stopping.launch_connected();
    stopping.send(Input::Disconnect);
    assert_eq!(
        connection_effects(&stopping.set_path(online_via(ethernet()))),
        []
    );
    assert_eq!(after(&stopping), Some(AfterStop::Idle));
}
