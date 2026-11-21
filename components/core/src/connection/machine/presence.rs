use std::time::SystemTime;

use super::{AttemptReason, ConnectionManager, Phase, RetryReason};

impl ConnectionManager {
    pub(in crate::connection) fn presence_changed(&mut self, away: bool, at: SystemTime) {
        if away {
            self.away_since.get_or_insert(at);
            return;
        }
        // The away time may have run out before the app could act on it, such as right after a
        // wake.
        self.check_away_limit();
        self.away_since = None;
        self.rejudge_held_reconnect();
    }

    /// A new, changed or removed schedule applies from its next start or end; nothing connects at
    /// once. A held reconnect is judged again under the new rules.
    pub(super) fn schedule_changed(&mut self) {
        self.reset_schedule_boundary();
        if self.settings.connection.schedule.is_none() {
            self.reconnect_suspended = false;
        } else {
            self.check_away_limit();
        }
        self.rejudge_held_reconnect();
    }

    /// A reconnect held for an away user passes the gate again: it may go ahead, wait with a new
    /// limit, or be given up.
    pub(super) fn rejudge_held_reconnect(&mut self) {
        if self.phase.is_holding() {
            self.begin_attempt(AttemptReason::Retry(RetryReason::UserAway), false, false);
        }
    }

    /// Marks the latest start or end at or before now as handled, so only later ones act.
    pub(super) fn reset_schedule_boundary(&mut self) {
        self.schedule_boundary = self
            .settings
            .connection
            .schedule
            .as_ref()
            .and_then(|schedule| schedule.latest_boundary(self.now, &self.time_zone))
            .map(|boundary| boundary.at);
    }

    /// Acts on the latest boundary only; earlier ones were missed in a sleep.
    pub(super) fn schedule_boundary_due(&mut self) {
        let Some(latest) = self
            .settings
            .connection
            .schedule
            .as_ref()
            .and_then(|schedule| schedule.latest_boundary(self.now, &self.time_zone))
        else {
            return;
        };
        if self
            .schedule_boundary
            .is_some_and(|handled| latest.at <= handled)
        {
            return;
        }
        self.schedule_boundary = Some(latest.at);
        match latest.edge {
            crate::connection::schedule::Edge::Start => {
                self.reconnect_suspended = false;
                self.connect_requested(AttemptReason::Scheduled);
            }
            crate::connection::schedule::Edge::End => self.check_away_limit(),
        }
    }

    /// Stops automatic reconnects once the away limit is reached; a waiting one is dropped.
    pub(super) fn check_away_limit(&mut self) {
        if self.reconnect_suspended || !self.away_limit_reached() {
            return;
        }
        self.reconnect_suspended = true;
        if matches!(
            self.phase,
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. }
        ) {
            self.give_up_reconnecting();
        }
    }

    /// Like Disconnect without stopping anything: the next connection waits for the user or the
    /// schedule.
    pub(super) fn give_up_reconnecting(&mut self) {
        self.reconnect_suspended = true;
        self.wants_connection = false;
        self.phase = Phase::Idle;
    }

    pub(in crate::connection) fn within_schedule(&self) -> bool {
        self.settings
            .connection
            .schedule
            .as_ref()
            .is_none_or(|schedule| schedule.is_active(self.now, &self.time_zone))
    }

    /// Always inside the scheduled hours; outside them only while the user is present and
    /// reconnects were not suspended.
    pub(in crate::connection) fn automatic_reconnect_allowed(&self) -> bool {
        self.within_schedule() || (!self.reconnect_suspended && self.away_since.is_none())
    }

    /// Outside the scheduled hours, the user has been away for the schedule's away time.
    pub(super) fn away_limit_reached(&self) -> bool {
        let (Some(schedule), Some(away_since)) =
            (&self.settings.connection.schedule, self.away_since)
        else {
            return false;
        };
        !schedule.is_active(self.now, &self.time_zone)
            && self.now >= away_since + schedule.away_limit()
    }
}
