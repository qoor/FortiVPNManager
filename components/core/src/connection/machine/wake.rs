use std::time::Duration;

use fvm_contract::app_helper::StopReason;

use crate::connection::Effect;
use crate::connection::policy;
use crate::connection::system::{PathSnapshot, WakeObservation};

use super::{AfterStop, AttemptReason, ConnectionManager, Phase, RetryReason, Session, Stage};

impl ConnectionManager {
    pub(in crate::connection) fn will_sleep(&mut self) {
        match self.take_phase() {
            Phase::Connecting(mut attempt) if attempt.replacing.is_some() => {
                let mut session = attempt.replacing.take().expect("matched above");
                if attempt.stage == Stage::Run && attempt.stop_requested.is_none() {
                    self.effects.push(Effect::StopRun {
                        attempt: attempt.id.clone(),
                        reason: StopReason::Sleep,
                    });
                }
                self.push_otp_close(attempt.close_otp_session(false, false));
                let renewal = session
                    .renewal
                    .as_mut()
                    .expect("a replacement is prepared for a renewal");
                renewal.next_try = Some(self.now);
                self.phase = Phase::Connected(session);
            }
            Phase::Connecting(attempt) => {
                self.abort_attempt(*attempt, StopReason::Sleep, AfterStop::ResumeAfterSleep);
            }
            // Already waiting for the user, who is away while the Mac sleeps.
            phase @ Phase::WaitingRetry {
                reason: RetryReason::UserAway,
                ..
            } => self.phase = phase,
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } => {
                self.wait_for_network(RetryReason::Sleep, Duration::ZERO);
            }
            Phase::Disconnecting(mut stop) => {
                if matches!(stop.after, AfterStop::Reconnect(_)) {
                    stop.after = AfterStop::ResumeAfterSleep;
                }
                self.phase = Phase::Disconnecting(stop);
            }
            // A tunnel that is up is kept across the sleep; its proactive timer waits.
            phase => self.phase = phase,
        }
    }

    pub(in crate::connection) fn did_wake(&mut self, slept_for: Duration) {
        self.sleeping = false;
        self.last_wake_at = Some(self.now);
        self.pending_wake = Some(slept_for);
        // The away time may have run out during the sleep; act before the user unlocks.
        self.check_away_limit();
    }

    pub(in crate::connection) fn wake_settled(&mut self, observation: &WakeObservation) {
        let slept_for = self.pending_wake.take().unwrap_or(Duration::ZERO);
        self.path = Some(PathSnapshot {
            satisfied: observation.path_satisfied,
            underlay: observation.underlay.clone(),
        });
        match &self.phase {
            // The wake rules judge a tunnel that was up across the sleep. One that came up after
            // the wake, when the path returned before the wake was judged, is left alone: the
            // observation may predate it. Ending the pending wake re-arms its proactive timer.
            Phase::Connected(session) => {
                if self.last_wake_at.is_none_or(|wake| session.since < wake) {
                    let session = session.clone();
                    self.evaluate_wake(session, observation, slept_for);
                }
            }
            Phase::WaitingNetwork {
                reason,
                delay_after_path,
            } if observation.path_satisfied && !self.sleeping => {
                self.resume_from_network_wait(*reason, *delay_after_path);
            }
            _ => {}
        }
    }

    /// The rules for a tunnel that was up across the sleep, in order. While automatic reconnects
    /// are held by the schedule, a working tunnel is kept rather than replaced; a dead or
    /// stranded one is still stopped, and the reconnect after it waits or is given up.
    pub(super) fn evaluate_wake(
        &mut self,
        mut session: Session,
        observation: &WakeObservation,
        slept_for: Duration,
    ) {
        let may_reconnect = self.automatic_reconnect_allowed();
        if !observation.path_satisfied {
            // The network did not come back: keep the tunnel and judge it later.
            let earliest = self.now + policy::MINIMUM_PLANNED_LEAD;
            session.planned_reconnect = session.planned_reconnect.map(|at| at.max(earliest));
            self.phase = Phase::Connected(session);
            self.rearm_proactive_timer();
            return;
        }
        let min_sleep =
            Duration::from_secs(u64::from(self.settings.connection.min_sleep_minutes) * 60);
        let planned_passed = may_reconnect
            && session
                .planned_reconnect
                .is_some_and(|planned| planned <= self.now);
        let reason = if !observation.session_running
            || !observation.tunnel_up
            || observation.routing_incomplete
        {
            Some(AttemptReason::TunnelLost)
        } else if let (Some(old), Some(new)) = (&session.underlay, &observation.underlay)
            && old != new
        {
            Some(AttemptReason::NetworkChanged)
        } else if may_reconnect
            && self.settings.connection.reconnect_after_sleep
            && slept_for >= min_sleep
        {
            Some(AttemptReason::AfterSleep)
        } else {
            None
        };
        match (planned_passed, reason) {
            (true, Some(_)) => self.restart_session(session, AttemptReason::Proactive),
            (false, Some(reason)) => self.restart_session(session, reason),
            (true, None) if session.renewal.is_none() => {
                self.renew(session, AttemptReason::Proactive);
            }
            (_, None) => {
                if session.underlay.is_none() {
                    session.underlay.clone_from(&observation.underlay);
                }
                self.phase = Phase::Connected(session);
                self.rearm_proactive_timer();
            }
        }
    }

    pub(in crate::connection) fn path_changed(
        &mut self,
        snapshot: PathSnapshot,
        routing_incomplete: bool,
    ) {
        self.path = Some(snapshot.clone());
        if self.sleeping {
            return;
        }
        let session = match &mut self.phase {
            Phase::WaitingNetwork {
                reason,
                delay_after_path,
            } if snapshot.satisfied => {
                let (reason, delay) = (*reason, *delay_after_path);
                self.resume_from_network_wait(reason, delay);
                return;
            }
            // While a wake is being judged, its observation decides instead.
            Phase::Connected(session) if self.pending_wake.is_none() => session,
            Phase::Connecting(attempt) => match &mut attempt.replacing {
                Some(session) => session,
                None => return,
            },
            // An attempt under way or a stop ignores the path; the other phases act on the next
            // attempt.
            _ => return,
        };
        let Some(underlay) = snapshot.underlay.filter(|_| snapshot.satisfied) else {
            return;
        };
        match &session.underlay {
            Some(old) if *old != underlay => match self.take_phase() {
                Phase::Connected(session) => {
                    self.restart_session(session, AttemptReason::NetworkChanged);
                }
                Phase::Connecting(attempt) => self.abort_attempt(
                    *attempt,
                    StopReason::Reconnect,
                    AfterStop::Reconnect(AttemptReason::NetworkChanged),
                ),
                _ => unreachable!("matched above"),
            },
            Some(_) => {}
            None => session.underlay = Some(underlay),
        }
        if routing_incomplete && matches!(self.phase, Phase::Connected(_)) {
            let Phase::Connected(session) = self.take_phase() else {
                unreachable!("matched above")
            };
            self.restart_session(session, AttemptReason::TunnelLost);
        }
    }

    pub(super) fn resume_from_network_wait(&mut self, reason: RetryReason, delay: Duration) {
        if delay > Duration::ZERO {
            self.phase = Phase::WaitingRetry {
                until: self.now + delay,
                reason,
            };
        } else if reason == RetryReason::Sleep {
            self.begin_attempt(AttemptReason::AfterSleep, false, false);
        } else {
            self.begin_attempt(AttemptReason::Retry(reason), false, false);
        }
    }
}
