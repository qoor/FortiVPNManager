use std::time::SystemTime;

use crate::connection::Effect;
use crate::connection::machine::{AttemptReason, ConnectionManager, Phase, Step};

use super::decision::{
    AwayLimit, BoundaryDue, LockPoll, PresenceChange, PresenceUpdate, ScheduleChange,
};

impl ConnectionManager {
    pub(in crate::connection::machine) fn change_presence(
        &mut self,
        change: PresenceChange,
    ) -> Vec<Step> {
        match change {
            PresenceChange::Away { since } => {
                if since.is_some() {
                    self.away_since = since;
                }
                Vec::new()
            }
            PresenceChange::Back { limit } => {
                self.apply_away_limit(limit);
                self.away_since = None;
                vec![Step::RejudgeHeldReconnect]
            }
        }
    }

    pub(in crate::connection::machine) fn update_presence(
        &mut self,
        update: PresenceUpdate,
    ) -> Vec<Step> {
        match update.lock_poll {
            LockPoll::Stop => self.system.lock_poll_at = None,
            LockPoll::Start { at } => self.system.lock_poll_at = Some(at),
            LockPoll::Keep => {}
        }
        update
            .away
            .map(|away| Step::PresenceChanged {
                away,
                at: update.at,
            })
            .into_iter()
            .collect()
    }

    pub(in crate::connection::machine) fn apply_away_limit(&mut self, limit: AwayLimit) {
        if let AwayLimit::Reached { give_up } = limit {
            self.reconnect_limits.reconnect_suspended = true;
            if give_up {
                self.give_up_reconnecting();
            }
        }
    }

    /// Like Disconnect without stopping anything: the next connection waits for the user or the
    /// schedule.
    pub(in crate::connection::machine) fn give_up_reconnecting(&mut self) {
        self.reconnect_limits.reconnect_suspended = true;
        self.wants_connection = false;
        self.phase = Phase::Idle;
    }

    pub(in crate::connection::machine) fn change_schedule(
        &mut self,
        change: ScheduleChange,
    ) -> Vec<Step> {
        self.schedule_boundary = change.boundary;
        match change.limit {
            None => self.reconnect_limits.reconnect_suspended = false,
            Some(limit) => self.apply_away_limit(limit),
        }
        vec![Step::RejudgeHeldReconnect]
    }

    pub(in crate::connection::machine) fn pass_boundary(&mut self, due: BoundaryDue) -> Vec<Step> {
        match due {
            BoundaryDue::Ignore => Vec::new(),
            BoundaryDue::Start { at } => {
                self.schedule_boundary = Some(at);
                self.reconnect_limits.reconnect_suspended = false;
                vec![Step::ConnectRequested(AttemptReason::Scheduled)]
            }
            BoundaryDue::End { at, limit } => {
                self.schedule_boundary = Some(at);
                self.apply_away_limit(limit);
                Vec::new()
            }
        }
    }

    pub(in crate::connection::machine) fn poll_lock(&mut self, next: SystemTime) {
        self.system.lock_poll_at = Some(next);
        self.effects.push(Effect::ReadScreenState);
    }

    /// Records what the OS reported of the screen and the displays; presence follows, as of `at`
    /// or else the input's time.
    pub(in crate::connection::machine) fn screen_reported(
        &mut self,
        locked: Option<bool>,
        displays_asleep: Option<bool>,
        at: Option<SystemTime>,
    ) -> Vec<Step> {
        if let Some(locked) = locked {
            self.system.screen_locked = locked;
        }
        if let Some(displays_asleep) = displays_asleep {
            self.system.displays_asleep = displays_asleep;
        }
        vec![Step::UpdatePresence(at)]
    }

    /// Whether the screen is locked behind the displays decides; the answer judges.
    pub(in crate::connection::machine) fn displays_woke(&mut self) -> Vec<Step> {
        self.system.displays_asleep = false;
        self.effects.push(Effect::ReadScreenState);
        Vec::new()
    }

    pub(in crate::connection::machine) fn reset_schedule_boundary(
        &mut self,
        boundary: Option<SystemTime>,
    ) {
        self.schedule_boundary = boundary;
    }
}
