use std::time::SystemTime;

use crate::connection::machine::{AttemptReason, ConnectionManager, Phase, RetryReason, Step};
use crate::connection::policy;
use crate::connection::schedule::Edge;

/// Whether the away limit stops automatic reconnects now.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum AwayLimit {
    NotReached,
    /// Reconnects are suspended; a waiting one is given up.
    Reached {
        give_up: bool,
    },
}

/// How the user's presence changed.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum PresenceChange {
    /// Away since `since`, when not away already.
    Away { since: Option<SystemTime> },
    /// Back. The away time may have run out before the app could act on it, such as right after
    /// a wake.
    Back { limit: AwayLimit },
}

/// The look at the screen while it is locked, in case the unlock went unreported.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum LockPoll {
    Stop,
    Start { at: SystemTime },
    Keep,
}

/// What a report of the screen, the displays or the sleep changes. The user is away while the
/// screen is locked, the displays sleep or the system sleeps.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) struct PresenceUpdate {
    pub(super) lock_poll: LockPoll,
    /// Away or back, when that changed, as of `at`.
    pub(super) away: Option<bool>,
    pub(super) at: SystemTime,
}

/// A new, changed or removed schedule applies from its next start or end; nothing connects at
/// once.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) struct ScheduleChange {
    pub(super) boundary: Option<SystemTime>,
    /// `None` once the schedule is removed, which lifts a suspension.
    pub(super) limit: Option<AwayLimit>,
}

/// What a due schedule boundary does. Only the latest one acts; earlier ones were missed in a
/// sleep.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum BoundaryDue {
    Ignore,
    Start { at: SystemTime },
    End { at: SystemTime, limit: AwayLimit },
}

impl ConnectionManager {
    pub(in crate::connection::machine) fn presence_change(
        &self,
        away: bool,
        at: SystemTime,
        now: SystemTime,
    ) -> PresenceChange {
        if away {
            PresenceChange::Away {
                since: self.away_since.is_none().then_some(at),
            }
        } else {
            PresenceChange::Back {
                limit: self.away_limit(now),
            }
        }
    }

    pub(in crate::connection::machine) fn presence_update(
        &self,
        at: Option<SystemTime>,
        now: SystemTime,
    ) -> PresenceUpdate {
        let lock_poll = if !self.system.screen_locked {
            LockPoll::Stop
        } else if self.system.lock_poll_at.is_none() {
            LockPoll::Start {
                at: now + policy::LOCK_POLL,
            }
        } else {
            LockPoll::Keep
        };
        let away = self.system.screen_locked || self.system.displays_asleep || self.system.sleeping;
        PresenceUpdate {
            lock_poll,
            away: (away != self.away_since.is_some()).then_some(away),
            at: at.unwrap_or(now),
        }
    }

    /// Stops automatic reconnects once the away limit is reached; a waiting one is dropped.
    pub(in crate::connection::machine) fn away_limit(&self, now: SystemTime) -> AwayLimit {
        if self.reconnect_limits.reconnect_suspended || !self.away_limit_reached(now) {
            return AwayLimit::NotReached;
        }
        AwayLimit::Reached {
            give_up: matches!(
                self.phase,
                Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. }
            ),
        }
    }

    /// A reconnect held for an away user passes the gate again: it may go ahead, wait with a new
    /// limit, or be given up.
    pub(in crate::connection::machine) fn held_reconnect(&self) -> Vec<Step> {
        if self.phase.is_holding() {
            vec![Step::begin_attempt(AttemptReason::Retry(
                RetryReason::UserAway,
            ))]
        } else {
            Vec::new()
        }
    }

    pub(in crate::connection::machine) fn schedule_change(
        &self,
        now: SystemTime,
    ) -> ScheduleChange {
        ScheduleChange {
            boundary: self.schedule_boundary_at(now),
            limit: self
                .settings
                .connection
                .schedule
                .is_some()
                .then(|| self.away_limit(now)),
        }
    }

    /// The latest start or end at or before `now`, which counts as handled so only later ones
    /// act.
    pub(in crate::connection::machine) fn schedule_boundary_at(
        &self,
        now: SystemTime,
    ) -> Option<SystemTime> {
        self.settings
            .connection
            .schedule
            .as_ref()
            .and_then(|schedule| schedule.latest_boundary(now, &self.time_zone))
            .map(|boundary| boundary.at)
    }

    pub(in crate::connection::machine) fn boundary_due(&self, now: SystemTime) -> BoundaryDue {
        let Some(latest) = self
            .settings
            .connection
            .schedule
            .as_ref()
            .and_then(|schedule| schedule.latest_boundary(now, &self.time_zone))
        else {
            return BoundaryDue::Ignore;
        };
        if self
            .schedule_boundary
            .is_some_and(|handled| latest.at <= handled)
        {
            return BoundaryDue::Ignore;
        }
        match latest.edge {
            Edge::Start => BoundaryDue::Start { at: latest.at },
            Edge::End => BoundaryDue::End {
                at: latest.at,
                limit: self.away_limit(now),
            },
        }
    }

    pub(in crate::connection) fn within_schedule(&self, now: SystemTime) -> bool {
        self.settings
            .connection
            .schedule
            .as_ref()
            .is_none_or(|schedule| schedule.is_active(now, &self.time_zone))
    }

    /// Always inside the scheduled hours; outside them only while the user is present and
    /// reconnects were not suspended.
    pub(in crate::connection) fn automatic_reconnect_allowed(&self, now: SystemTime) -> bool {
        self.within_schedule(now)
            || (!self.reconnect_limits.reconnect_suspended && self.away_since.is_none())
    }

    /// Outside the scheduled hours, the user has been away for the schedule's away time.
    pub(in crate::connection::machine) fn away_limit_reached(&self, now: SystemTime) -> bool {
        let (Some(schedule), Some(away_since)) =
            (&self.settings.connection.schedule, self.away_since)
        else {
            return false;
        };
        !schedule.is_active(now, &self.time_zone) && now >= away_since + schedule.away_limit()
    }
}

/// The next look at the locked screen.
pub(in crate::connection::machine) fn next_lock_poll(now: SystemTime) -> SystemTime {
    now + policy::LOCK_POLL
}
