//! The scheduled hours: one window of local time on the selected days.

use std::time::{Duration, SystemTime};

use fvm_contract::app_internal::{TimeOfDay, Weekday};
use jiff::civil::{self, Date};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};

/// `connection.schedule` while it is on. The window's start connects like Connect; its end never
/// disconnects.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Schedule {
    pub days: Vec<Weekday>,
    pub start: TimeOfDay,
    /// After `start`.
    pub end: TimeOfDay,
    /// Away this long outside the window, automatic reconnects stop.
    pub away_minutes: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Edge {
    Start,
    End,
}

/// A window's start or end on one day.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Boundary {
    pub(super) at: SystemTime,
    pub(super) edge: Edge,
}

impl Schedule {
    pub(super) fn away_limit(&self) -> Duration {
        Duration::from_secs(u64::from(self.away_minutes) * 60)
    }

    /// From a start, inclusive, to its end, exclusive.
    pub(super) fn is_active(&self, at: SystemTime, time_zone: &TimeZone) -> bool {
        self.latest_boundary(at, time_zone)
            .is_some_and(|boundary| boundary.edge == Edge::Start)
    }

    /// `None` without a selected day.
    pub(super) fn latest_boundary(&self, at: SystemTime, time_zone: &TimeZone) -> Option<Boundary> {
        self.boundaries(at, time_zone)
            .into_iter()
            .rev()
            .find(|boundary| boundary.at <= at)
    }

    /// The first boundary strictly after `after`; `None` without a selected day.
    pub(super) fn next_boundary(
        &self,
        after: SystemTime,
        time_zone: &TimeZone,
    ) -> Option<Boundary> {
        self.boundaries(after, time_zone)
            .into_iter()
            .find(|boundary| boundary.at > after)
    }

    pub(super) fn next_start(&self, after: SystemTime, time_zone: &TimeZone) -> Option<SystemTime> {
        self.boundaries(after, time_zone)
            .into_iter()
            .find(|boundary| boundary.at > after && boundary.edge == Edge::Start)
            .map(|boundary| boundary.at)
    }

    /// Every boundary from 8 days before to 8 days after `around`, in order: a week and a day on
    /// either side always holds the neighbours of `around` when a day is selected.
    fn boundaries(&self, around: SystemTime, time_zone: &TimeZone) -> Vec<Boundary> {
        if self.days.is_empty() {
            return Vec::new();
        }
        let today = Timestamp::try_from(around)
            .expect("a time within the calendar")
            .to_zoned(time_zone.clone())
            .date();
        let mut boundaries = Vec::new();
        for offset in -8..=8 {
            let day = today
                .checked_add(offset.days())
                .expect("a day within the calendar");
            if !self.days.contains(&weekday(day)) {
                continue;
            }
            let start = local_time(day, self.start, time_zone);
            let end = local_time(day, self.end, time_zone);
            // A daylight saving change can move a start onto or past its end.
            if start < end {
                boundaries.push(Boundary {
                    at: start,
                    edge: Edge::Start,
                });
                boundaries.push(Boundary {
                    at: end,
                    edge: Edge::End,
                });
            }
        }
        boundaries
    }
}

/// `time` on `day`; a time a daylight saving change skips moves to the next valid one, and a
/// repeated one is its first occurrence.
fn local_time(day: Date, time: TimeOfDay, time_zone: &TimeZone) -> SystemTime {
    let hour = i8::try_from(time.hour).expect("an hour of the day");
    let minute = i8::try_from(time.minute).expect("a minute of the hour");
    let zoned = day
        .at(hour, minute, 0, 0)
        .to_zoned(time_zone.clone())
        .expect("a time within the calendar");
    SystemTime::from(zoned.timestamp())
}

fn weekday(day: Date) -> Weekday {
    match day.weekday() {
        civil::Weekday::Monday => Weekday::Mon,
        civil::Weekday::Tuesday => Weekday::Tue,
        civil::Weekday::Wednesday => Weekday::Wed,
        civil::Weekday::Thursday => Weekday::Thu,
        civil::Weekday::Friday => Weekday::Fri,
        civil::Weekday::Saturday => Weekday::Sat,
        civil::Weekday::Sunday => Weekday::Sun,
    }
}
