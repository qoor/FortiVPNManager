use std::time::{Duration, SystemTime};

use fvm_contract::app_helper::SessionId;
use jiff::tz::TimeZone;

use super::super::{Effect, Input};
use super::harness::*;

fn session_begun_at(h: &mut Harness, at: SystemTime) -> SessionId {
    h.now = at;
    let effects = h.send(Input::Connect);
    h.send(Input::Disconnect);
    match effects.first() {
        Some(Effect::AttemptBegun { session, .. }) => *session,
        _ => panic!("no attempt began: {effects:?}"),
    }
}

fn utc(year: i16, month: i8, day: i8, hour: i8, minute: i8, second: i8) -> SystemTime {
    in_zone(&TimeZone::UTC, year, month, day, hour, minute, second)
}

fn second() -> SystemTime {
    utc(2026, 11, 17, 16, 24, 7)
}

#[test]
fn a_session_id_is_ten_times_the_seconds_since_2026() {
    let mut h = Harness::new();
    let start = session_begun_at(&mut h, utc(2026, 1, 1, 0, 0, 0));
    assert_eq!(start.to_string(), "0000000000");
    let later = session_begun_at(&mut h, second() + Duration::from_millis(594));
    assert_eq!(later.to_string(), "0277070470");
}

#[test]
fn attempts_in_the_same_second_take_the_next_digits() {
    let mut h = Harness::new();
    let ids: Vec<SessionId> = [0, 400, 999]
        .into_iter()
        .map(|millis| session_begun_at(&mut h, second() + Duration::from_millis(millis)))
        .collect();
    assert_eq!(
        ids,
        [
            SessionId(277_070_470),
            SessionId(277_070_471),
            SessionId(277_070_472)
        ]
    );
}

#[test]
fn an_eleventh_attempt_in_a_second_takes_the_next_seconds_first_id() {
    let mut h = Harness::new();
    let ids: Vec<SessionId> = (0..11)
        .map(|_| session_begun_at(&mut h, second()))
        .collect();
    assert_eq!(
        ids,
        (277_070_470..=277_070_480)
            .map(SessionId)
            .collect::<Vec<_>>()
    );
    let next_second = second() + Duration::from_secs(1);
    assert_eq!(
        session_begun_at(&mut h, next_second),
        SessionId(277_070_481)
    );
    assert_eq!(
        session_begun_at(&mut h, next_second + Duration::from_secs(1)),
        SessionId(277_070_490)
    );
}

#[test]
fn a_clock_set_back_still_issues_a_greater_id() {
    let mut h = Harness::new();
    assert_eq!(session_begun_at(&mut h, second()), SessionId(277_070_470));
    let an_hour_earlier = second() - Duration::from_secs(3600);
    assert_eq!(
        session_begun_at(&mut h, an_hour_earlier),
        SessionId(277_070_471)
    );
    assert_eq!(
        session_begun_at(&mut h, an_hour_earlier + Duration::from_secs(1)),
        SessionId(277_070_472)
    );
    assert_eq!(
        session_begun_at(&mut h, second() + Duration::from_secs(10)),
        SessionId(277_070_570)
    );
}

#[test]
fn a_clock_before_2026_counts_from_zero() {
    let mut h = Harness::new();
    let before = utc(2025, 12, 31, 23, 59, 59);
    assert_eq!(session_begun_at(&mut h, before), SessionId(0));
    assert_eq!(session_begun_at(&mut h, before), SessionId(1));
}
