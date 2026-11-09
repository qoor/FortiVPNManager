//! Scheduled hours and time away.

use fvm_contract::app_helper::{HelperErrorCode, Milestone, StopReason};
use fvm_contract::app_internal::section::{ConnectionIcon, ScheduleOutlook};
use fvm_contract::app_internal::{TimeOfDay, Weekday};
use jiff::tz::TimeZone;

use super::super::machine::{AttemptReason, Phase, RetryReason};
use super::super::schedule::{Boundary, Edge};
use super::super::{Effect, Failure, Input, Schedule, TimerKind};
use super::harness::*;

fn reason(h: &Harness) -> Option<AttemptReason> {
    h.attempt().map(|attempt| attempt.reason)
}

fn harness_at(day: i8, hour: i8, minute: i8, schedule: Option<Schedule>) -> Harness {
    let mut settings = settings();
    settings.connection.schedule = schedule;
    let mut h = Harness::with_settings(settings);
    h.now = at(day, hour, minute, 0);
    h
}

fn hours_at(day: i8, hour: i8, minute: i8) -> Harness {
    harness_at(day, hour, minute, Some(weekday_hours()))
}

fn away(h: &mut Harness) -> Vec<Effect> {
    h.send(Input::ScreenLocked { at: h.now })
}

fn back(h: &mut Harness) -> Vec<Effect> {
    h.send(Input::ScreenUnlocked)
}

fn boundary(at: std::time::SystemTime, edge: Edge) -> Option<Boundary> {
    Some(Boundary { at, edge })
}

// MARK: Window math

#[test]
fn active_from_the_start_inclusive_to_the_end_exclusive_on_selected_days() {
    let schedule = weekday_hours();
    for (day, hour, minute, expected) in [
        (18, 8, 59, false),
        (18, 9, 0, true),
        (18, 12, 0, true),
        (18, 17, 59, true),
        (18, 18, 0, false),
        (22, 9, 30, true),
        (23, 10, 0, false),
        (24, 10, 0, false),
        (25, 9, 0, true),
    ] {
        assert_eq!(
            schedule.is_active(at(day, hour, minute, 0), &seoul()),
            expected,
            "{day} {hour}:{minute}"
        );
    }
}

#[test]
fn the_latest_boundary_looks_back_across_the_weekend() {
    let schedule = weekday_hours();
    assert_eq!(
        schedule.latest_boundary(at(23, 10, 0, 0), &seoul()),
        boundary(at(22, 18, 0, 0), Edge::End)
    );
    assert_eq!(
        schedule.latest_boundary(at(18, 9, 0, 0), &seoul()),
        boundary(at(18, 9, 0, 0), Edge::Start)
    );
}

#[test]
fn the_next_boundary_is_strictly_later() {
    let schedule = weekday_hours();
    assert_eq!(
        schedule.next_boundary(at(22, 18, 0, 0), &seoul()),
        boundary(at(25, 9, 0, 0), Edge::Start)
    );
    assert_eq!(
        schedule.next_boundary(at(18, 9, 0, 0), &seoul()),
        boundary(at(18, 18, 0, 0), Edge::End)
    );
    assert_eq!(
        schedule.next_start(at(18, 9, 0, 0), &seoul()),
        Some(at(19, 9, 0, 0))
    );
    assert_eq!(
        schedule.next_start(at(22, 12, 0, 0), &seoul()),
        Some(at(25, 9, 0, 0))
    );
}

#[test]
fn a_single_day_repeats_weekly() {
    let sundays = Schedule {
        days: vec![Weekday::Sun],
        start: TimeOfDay { hour: 0, minute: 0 },
        end: TimeOfDay {
            hour: 23,
            minute: 59,
        },
        away_minutes: 120,
    };
    assert_eq!(
        sundays.next_start(at(24, 0, 0, 0), &seoul()),
        Some(at(31, 0, 0, 0))
    );
    assert_eq!(
        sundays.latest_boundary(at(23, 12, 0, 0), &seoul()),
        boundary(at(17, 23, 59, 0), Edge::End)
    );
}

#[test]
fn no_days_are_never_active() {
    let none = Schedule {
        days: vec![],
        ..weekday_hours()
    };
    assert!(!none.is_active(at(18, 12, 0, 0), &seoul()));
    assert_eq!(none.latest_boundary(at(18, 12, 0, 0), &seoul()), None);
    assert_eq!(none.next_boundary(at(18, 12, 0, 0), &seoul()), None);
    assert_eq!(none.next_start(at(18, 12, 0, 0), &seoul()), None);
}

#[test]
fn times_stay_on_the_local_clock_across_daylight_saving_changes() {
    let new_york = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
    let sundays = Schedule {
        days: vec![Weekday::Sun],
        ..weekday_hours()
    };
    for (year, month, day) in [(2026, 3, 8), (2026, 11, 1)] {
        let midnight = in_zone(&new_york, year, month, day, 0, 0, 0);
        let start = sundays.next_boundary(midnight, &new_york).unwrap();
        let end = sundays.next_boundary(start.at, &new_york).unwrap();
        assert_eq!((start.edge, end.edge), (Edge::Start, Edge::End));
        assert_eq!(start.at, in_zone(&new_york, year, month, day, 9, 0, 0));
        assert_eq!(end.at, in_zone(&new_york, year, month, day, 18, 0, 0));
        assert_eq!(end.at.duration_since(start.at).unwrap(), secs(9.0 * 3600.0));
    }
}

// MARK: Start, end and launch

#[test]
fn a_launch_inside_the_hours_connects() {
    let mut h = hours_at(18, 10, 0);
    let effects = h.launch();
    assert_eq!(reason(&h), Some(AttemptReason::Launch));
    assert_eq!(
        timer_set(&effects, TimerKind::Schedule),
        Some(at(18, 18, 0, 0))
    );
}

#[test]
fn a_launch_outside_the_hours_waits_for_the_start() {
    let mut h = hours_at(18, 7, 0);
    let effects = h.launch();
    assert_eq!(h.cm.phase, Phase::Idle);
    assert!(!h.cm.wants_connection);
    assert_eq!(
        timer_set(&effects, TimerKind::Schedule),
        Some(at(18, 9, 0, 0))
    );
    h.fire(TimerKind::Schedule);
    assert!(h.cm.wants_connection);
    assert_eq!(reason(&h), Some(AttemptReason::Scheduled));
    h.run_to_connected();
    assert_eq!(h.timer(TimerKind::Schedule), Some(at(18, 18, 0, 0)));
}

#[test]
fn a_launch_without_auto_connect_stays_off_even_inside_the_hours() {
    let mut h = hours_at(18, 10, 0);
    h.launch_with(false, None);
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn the_start_clears_a_failure_like_connect() {
    let mut h = hours_at(18, 7, 0);
    h.launch();
    h.send(Input::Connect);
    let id = h.start_tunnel();
    h.send(Input::RunRejected(id, HelperErrorCode::ClientUnavailable));
    assert_eq!(
        h.cm.phase,
        Phase::Failed(Failure::Helper(HelperErrorCode::ClientUnavailable))
    );
    h.fire(TimerKind::Schedule);
    assert_eq!(reason(&h), Some(AttemptReason::Scheduled));
}

#[test]
fn the_end_keeps_the_tunnel_and_reconnects_while_the_user_is_present() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    let effects = h.fire(TimerKind::Schedule);
    assert_eq!(h.now, at(18, 18, 0, 0));
    assert!(stop_reasons(&effects).is_empty());
    assert!(h.connected().is_some());
    assert!(h.timer(TimerKind::ProactiveReconnect).is_some());
    h.advance(3600.0);
    h.exit(ended(Milestone::TunnelUp).up(7200.0));
    h.fire(TimerKind::Retry);
    assert_eq!(reason(&h), Some(retry(RetryReason::ServerSessionEnded)));
}

#[test]
fn a_manual_connect_outside_the_hours_keeps_reconnecting() {
    let mut h = hours_at(23, 10, 0);
    h.launch();
    assert_eq!(h.cm.phase, Phase::Idle);
    h.send(Input::Connect);
    h.run_to_connected();
    let id = h.connected().unwrap().attempt.clone();
    assert_eq!(
        stop_reasons(&h.fire(TimerKind::ProactiveReconnect)),
        [StopReason::ProactiveReconnect]
    );
    h.exit_for(
        &id,
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::ProactiveReconnect)
            .up(175.0 * 60.0),
    );
    assert_eq!(reason(&h), Some(AttemptReason::Proactive));
}

// MARK: Away outside the hours

#[test]
fn away_outside_the_hours_holds_the_reconnect_until_the_user_returns() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    h.advance(600.0);
    let effects = away(&mut h);
    assert!(cancels(&effects, TimerKind::ProactiveReconnect));
    assert_eq!(
        timer_set(&effects, TimerKind::AwayLimit),
        Some(at(18, 20, 10, 0))
    );
    h.advance(1800.0);
    h.exit(ended(Milestone::TunnelUp).up(5400.0));
    h.fire(TimerKind::Retry);
    assert_eq!(
        h.retry_wait(),
        Some((at(18, 20, 10, 0), RetryReason::UserAway))
    );
    assert_eq!(h.cm.connection_section().icon, ConnectionIcon::Disconnected);
    assert_eq!(
        h.cm.schedule_outlook(),
        Some(ScheduleOutlook::HoldingWhileAway {
            until: at(18, 20, 10, 0)
        })
    );
    assert!(!opens_otp_session(&h.effects) && !checks_readiness(&h.effects));
    h.advance(1200.0);
    back(&mut h);
    assert_eq!(reason(&h), Some(retry(RetryReason::UserAway)));
}

#[test]
fn away_inside_the_hours_changes_nothing() {
    let mut h = hours_at(18, 10, 0);
    h.launch_connected();
    h.advance(3600.0);
    away(&mut h);
    assert!(h.timer(TimerKind::ProactiveReconnect).is_some());
    h.exit(ended(Milestone::TunnelUp).up(3600.0));
    h.fire(TimerKind::Retry);
    assert_eq!(reason(&h), Some(retry(RetryReason::ServerSessionEnded)));
}

#[test]
fn the_away_limit_stops_reconnects_but_keeps_the_tunnel() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    h.advance(600.0);
    away(&mut h);
    let limit = h.fire(TimerKind::AwayLimit);
    assert_eq!(h.now, at(18, 20, 10, 0));
    assert!(stop_reasons(&limit).is_empty());
    assert!(h.connected().is_some());
    assert!(h.cm.reconnect_suspended);
    assert_eq!(h.cm.schedule_outlook(), Some(ScheduleOutlook::Suspended {}));
    // Back before the server ends the session: it is kept, but the next one does not replace it.
    h.advance(600.0);
    back(&mut h);
    assert_eq!(h.timer(TimerKind::ProactiveReconnect), None);
    h.advance(600.0);
    h.exit(ended(Milestone::TunnelUp).up(3.0 * 3600.0));
    h.fire(TimerKind::Retry);
    assert_eq!(h.cm.phase, Phase::Idle);
    assert!(!h.cm.wants_connection);
    assert_eq!(
        h.cm.schedule_outlook(),
        Some(ScheduleOutlook::Connects {
            at: at(19, 9, 0, 0)
        })
    );
}

#[test]
fn a_held_reconnect_is_given_up_at_the_limit() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    h.advance(600.0);
    away(&mut h);
    h.advance(1800.0);
    h.exit(ended(Milestone::TunnelUp).up(5400.0));
    h.fire(TimerKind::Retry);
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::UserAway)
    );
    h.fire(TimerKind::Retry);
    assert_eq!(h.cm.phase, Phase::Idle);
    assert!(!h.cm.wants_connection);
    assert!(h.cm.reconnect_suspended);
    back(&mut h);
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn away_time_before_the_end_counts_at_the_end() {
    let mut h = hours_at(18, 15, 0);
    h.launch_connected();
    h.now = at(18, 15, 30, 0);
    away(&mut h);
    if h.timer(TimerKind::AwayLimit).is_some() {
        h.fire(TimerKind::AwayLimit);
    }
    assert!(!h.cm.reconnect_suspended);
    h.fire(TimerKind::Schedule);
    assert_eq!(h.now, at(18, 18, 0, 0));
    assert!(h.cm.reconnect_suspended);
    assert!(h.connected().is_some());
}

#[test]
fn the_next_start_clears_the_suspension() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    away(&mut h);
    h.fire(TimerKind::AwayLimit);
    h.exit(ended(Milestone::TunnelUp).up(3.0 * 3600.0));
    h.fire(TimerKind::Retry);
    assert_eq!(h.cm.phase, Phase::Idle);
    h.fire(TimerKind::Schedule);
    assert_eq!(h.now, at(19, 9, 0, 0));
    assert!(!h.cm.reconnect_suspended);
    assert_eq!(reason(&h), Some(AttemptReason::Scheduled));
}

#[test]
fn reconnect_now_clears_the_suspension() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    away(&mut h);
    h.fire(TimerKind::AwayLimit);
    back(&mut h);
    assert_eq!(
        stop_reasons(&h.send(Input::ReconnectNow)),
        [StopReason::Reconnect]
    );
    assert!(!h.cm.reconnect_suspended);
}

// MARK: Sleep

#[test]
fn a_long_sleep_outside_the_hours_stops_reconnects_before_the_user_unlocks() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    h.now = at(18, 17, 30, 0);
    away(&mut h);
    h.sleep();
    h.now = at(18, 21, 0, 0);
    let wake = h.wake();
    assert!(stop_reasons(&wake).is_empty());
    if h.timer(TimerKind::Schedule).is_some_and(|due| due <= h.now) {
        h.fire(TimerKind::Schedule);
    }
    assert!(h.cm.reconnect_suspended);
    back(&mut h);
    let settled = h.settle(observe().dead());
    assert_eq!(stop_reasons(&settled), [StopReason::Reconnect]);
    h.exit_for(
        &id,
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::Reconnect)
            .up(3600.0),
    );
    assert_eq!(h.cm.phase, Phase::Idle);
    assert!(!h.cm.wants_connection);
}

#[test]
fn a_working_tunnel_is_kept_on_a_wake_while_reconnects_wait() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    let planned = h.connected().unwrap().planned_reconnect.unwrap();
    h.fire(TimerKind::Schedule);
    h.now = at(18, 19, 0, 0);
    away(&mut h);
    h.sleep();
    h.now = at(18, 20, 0, 0);
    h.wake();
    assert!(!h.cm.reconnect_suspended);
    let settled = h.settle(observe());
    assert!(stop_reasons(&settled).is_empty());
    assert!(h.connected().is_some());
    assert_eq!(h.timer(TimerKind::ProactiveReconnect), None);
    let back_effects = back(&mut h);
    assert_eq!(
        timer_set(&back_effects, TimerKind::ProactiveReconnect),
        Some(planned)
    );
    assert_eq!(
        stop_reasons(&h.fire(TimerKind::ProactiveReconnect)),
        [StopReason::ProactiveReconnect]
    );
}

#[test]
fn a_missed_weekend_applies_only_the_latest_boundary() {
    let mut h = hours_at(22, 17, 0);
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    h.send(Input::Disconnect);
    h.exit_for(
        &id,
        ended(Milestone::TunnelUp)
            .by_helper(StopReason::UserDisconnect)
            .up(600.0),
    );
    assert_eq!(h.cm.phase, Phase::Idle);
    away(&mut h);
    h.sleep();
    h.now = at(25, 9, 30, 0);
    h.wake();
    h.fire(TimerKind::Schedule);
    assert_eq!(h.now, at(25, 9, 30, 0));
    assert_eq!(reason(&h), Some(AttemptReason::Scheduled));
    assert_eq!(h.timer(TimerKind::Schedule), Some(at(25, 18, 0, 0)));
}

#[test]
fn sleep_keeps_a_held_reconnect_waiting_for_the_user() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    away(&mut h);
    h.exit(ended(Milestone::TunnelUp).up(3600.0));
    h.fire(TimerKind::Retry);
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::UserAway)
    );
    h.sleep();
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::UserAway)
    );
}

// MARK: Settings

#[test]
fn editing_the_schedule_acts_from_the_next_boundary() {
    let mut h = hours_at(18, 20, 0);
    h.launch();
    assert_eq!(h.cm.phase, Phase::Idle);
    let mut settings = settings();
    settings.connection.schedule = Some(Schedule {
        end: TimeOfDay {
            hour: 21,
            minute: 0,
        },
        ..weekday_hours()
    });
    let effects = h.send(Input::SettingsChanged(settings));
    assert_eq!(h.cm.phase, Phase::Idle);
    assert_eq!(
        timer_set(&effects, TimerKind::Schedule),
        Some(at(18, 21, 0, 0))
    );
}

#[test]
fn removing_the_schedule_resumes_a_held_reconnect() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    away(&mut h);
    h.exit(ended(Milestone::TunnelUp).up(3600.0));
    h.fire(TimerKind::Retry);
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::UserAway)
    );
    let effects = h.send(Input::SettingsChanged(settings()));
    assert_eq!(reason(&h), Some(retry(RetryReason::UserAway)));
    assert!(cancels(&effects, TimerKind::Schedule));
    assert!(cancels(&effects, TimerKind::AwayLimit));
}

#[test]
fn a_shorter_away_limit_applies_at_once() {
    let mut h = hours_at(18, 17, 0);
    h.launch_connected();
    h.fire(TimerKind::Schedule);
    away(&mut h);
    h.advance(3600.0);
    let mut settings = settings();
    settings.connection.schedule = Some(Schedule {
        away_minutes: 30,
        ..weekday_hours()
    });
    h.send(Input::SettingsChanged(settings));
    assert!(h.cm.reconnect_suspended);
    assert!(h.connected().is_some());
}

#[test]
fn a_time_zone_change_rereads_the_times() {
    let mut h = hours_at(18, 8, 0);
    h.launch();
    let utc = TimeZone::UTC;
    let effects = h.send(Input::TimeZoneChanged(utc.clone()));
    // Monday 08:00 in Seoul is Sunday 23:00 UTC; the next start is Monday 09:00 UTC.
    assert_eq!(
        timer_set(&effects, TimerKind::Schedule),
        Some(in_zone(&utc, 2030, 3, 18, 9, 0, 0))
    );
}

#[test]
fn the_outlook_follows_the_day() {
    let mut h = hours_at(18, 7, 0);
    h.launch();
    assert_eq!(
        h.cm.schedule_outlook(),
        Some(ScheduleOutlook::Connects {
            at: at(18, 9, 0, 0)
        })
    );
    h.fire(TimerKind::Schedule);
    h.run_to_connected();
    assert_eq!(
        h.cm.schedule_outlook(),
        Some(ScheduleOutlook::WithinHours {
            end: at(18, 18, 0, 0)
        })
    );
    h.fire(TimerKind::Schedule);
    assert_eq!(
        h.cm.schedule_outlook(),
        Some(ScheduleOutlook::OutsideHoursPresent {})
    );
    let mut none = harness_at(18, 7, 0, None);
    none.launch();
    assert_eq!(none.cm.schedule_outlook(), None);
    let mut no_days = harness_at(
        18,
        7,
        0,
        Some(Schedule {
            days: vec![],
            ..weekday_hours()
        }),
    );
    no_days.launch();
    assert_eq!(no_days.cm.phase, Phase::Idle);
    assert_eq!(
        no_days.cm.schedule_outlook(),
        Some(ScheduleOutlook::NoDays {})
    );
    assert_eq!(no_days.timer(TimerKind::Schedule), None);
}
