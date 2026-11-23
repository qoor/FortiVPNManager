use std::collections::BTreeMap;
use std::time::SystemTime;

use crate::connection::machine::{ConnectionManager, OtpSession, Phase, Quit, Stage};
use crate::connection::system::WakeSettle;
use crate::connection::{Input, TimerId, TimerKind, policy};

/// How one kind of timer changes after an input.
pub(in crate::connection::machine) enum TimerChange {
    /// No longer wanted; the runtime is told to cancel it unless it fired already.
    Remove { cancel: bool },
    Set { at: SystemTime },
}

impl ConnectionManager {
    /// The time `input` is judged at. A current timer is never early here. The runtime may sleep
    /// on a monotonic clock and stamp a slewed or stepped-back wall clock; judged a few
    /// milliseconds early, a retry bound by the cooldown would enter the very wait whose timer
    /// already fired, and nothing would move it on.
    pub(in crate::connection::machine) fn input_time(
        &self,
        input: &Input,
        now: SystemTime,
    ) -> SystemTime {
        match input {
            Input::TimerFired(timer) => match self.timers.get(&timer.kind) {
                Some(slot) if slot.generation == timer.generation && !slot.fired => {
                    now.max(slot.at)
                }
                _ => now,
            },
            _ => now,
        }
    }

    /// The kind of `timer` while it is the current one of its kind and has not fired; a replaced
    /// or cancelled one is ignored.
    pub(in crate::connection::machine) fn due_timer(&self, timer: TimerId) -> Option<TimerKind> {
        self.timers
            .get(&timer.kind)
            .filter(|slot| slot.generation == timer.generation && !slot.fired)
            .map(|_| timer.kind)
    }

    /// The timers to set or remove so that the runtime holds exactly the ones the state wants.
    pub(in crate::connection::machine) fn timer_changes(
        &self,
        now: SystemTime,
    ) -> Vec<(TimerKind, TimerChange)> {
        let wanted = self.wanted_timers(now);
        TimerKind::ALL
            .into_iter()
            .filter_map(|kind| {
                let change = match (self.timers.get(&kind), wanted.get(&kind)) {
                    (None, None) => None,
                    (Some(slot), None) => Some(TimerChange::Remove { cancel: !slot.fired }),
                    // A fired timer still wanted at the same time stays fired: no firing loop.
                    (Some(slot), Some(at)) if slot.at == *at && !self.rearms(kind) => None,
                    (_, Some(at)) => Some(TimerChange::Set { at: *at }),
                };
                change.map(|change| (kind, change))
            })
            .collect()
    }

    fn rearms(&self, kind: TimerKind) -> bool {
        kind == TimerKind::ProactiveReconnect && self.proactive_timer_rearm
    }

    fn wanted_timers(&self, now: SystemTime) -> BTreeMap<TimerKind, SystemTime> {
        let mut wanted = BTreeMap::new();
        if self.quit == Quit::Terminated {
            return wanted;
        }
        match &self.phase {
            Phase::Connecting(attempt) if attempt.stop_requested.is_none() => {
                wanted.insert(TimerKind::AttemptDeadline, attempt.deadline);
                if attempt.stage == Stage::Run
                    && attempt.otp_session != OtpSession::Submitted
                    && let Some(prompted) = attempt.otp_prompted_at
                {
                    wanted.insert(TimerKind::OtpDeadline, prompted + policy::OTP_DEADLINE);
                }
            }
            Phase::Connected(session) if !self.system.sleeping && self.pending_wake.is_none() => {
                let next = match &session.renewal {
                    None if self.automatic_reconnect_allowed(now) => session
                        .planned_reconnect
                        .map(|planned| (TimerKind::ProactiveReconnect, planned)),
                    Some(renewal)
                        if !renewal.reason.is_automatic_reconnect()
                            || self.automatic_reconnect_allowed(now) =>
                    {
                        renewal.next_try.map(|at| (TimerKind::Retry, at))
                    }
                    _ => None,
                };
                wanted.extend(next);
            }
            Phase::WaitingRetry { until, .. } => {
                wanted.insert(TimerKind::Retry, *until);
            }
            _ => {}
        }
        if let Some(stale) = &self.stale_otp {
            wanted.insert(TimerKind::StaleOtp, stale.until);
        }
        let stopping_for_update = self
            .update_stop
            .filter(|stop| !stop.stopped)
            .map(|stop| stop.requested_at);
        let quit_requested_at = match self.quit {
            Quit::Requested { at } => Some(at),
            Quit::NotRequested | Quit::Terminated => None,
        };
        if let Some(requested) = quit_requested_at.or(stopping_for_update) {
            wanted.insert(TimerKind::QuitDeadline, requested + policy::QUIT_TIMEOUT);
        }
        if let Some(schedule) = &self.settings.connection.schedule
            && self.quit == Quit::NotRequested
            && self.update_stop.is_none()
        {
            if let Some(handled) = self.schedule_boundary
                && let Some(next) = schedule.next_boundary(handled, &self.time_zone)
            {
                wanted.insert(TimerKind::Schedule, next.at);
            }
            if let Some(away_since) = self.away_since
                && !self.reconnect_limits.reconnect_suspended
            {
                wanted.insert(TimerKind::AwayLimit, away_since + schedule.away_limit());
            }
        }
        let observation = [
            (TimerKind::PathSettle, self.system.path_settle_at),
            (
                TimerKind::WakeSettle,
                self.system
                    .wake_settle
                    .as_ref()
                    .and_then(WakeSettle::next_look),
            ),
            (TimerKind::LockPoll, self.system.lock_poll_at),
            (
                TimerKind::ApprovalFollow,
                self.readiness
                    .follow_approval
                    .as_ref()
                    .map(|follow| follow.next),
            ),
        ];
        for (kind, at) in observation {
            if let Some(at) = at {
                wanted.insert(kind, at);
            }
        }
        wanted
    }
}
