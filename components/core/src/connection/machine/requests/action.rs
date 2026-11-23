use std::time::SystemTime;

use fvm_contract::app_helper::StopReason;
use jiff::tz::TimeZone;

use crate::connection::Effect;
use crate::connection::machine::{
    AfterStop, AttemptReason, ConnectionManager, Phase, Quit, Step, UpdateStop,
};

use super::decision::{
    ConnectRequest, DisconnectRequest, LaunchStart, Launched, ReconnectNowRequest, SettingsChange,
    StopRequest,
};

impl ConnectionManager {
    pub(in crate::connection::machine) fn launch(&mut self, launched: Launched) -> Vec<Step> {
        if let Some(restored) = launched.restored {
            self.saved_ledger = restored.clone();
            self.ledger = restored;
        }
        self.ledger.recent_auth_attempts = launched.kept_attempts;
        self.schedule_boundary = launched.schedule_boundary;
        self.effects.push(Effect::ReadScreenState);
        if launched.open_settings {
            self.readiness.settings_open_decided = true;
            self.effects.push(Effect::OpenSettings);
        }
        let mut steps = match launched.start {
            LaunchStart::Wait => {
                self.wants_connection = false;
                vec![Step::CheckReadiness]
            }
            LaunchStart::Fail(failure) => {
                self.wants_connection = true;
                self.fail(failure);
                Vec::new()
            }
            LaunchStart::Attempt => {
                self.wants_connection = true;
                vec![Step::begin_attempt(AttemptReason::Launch)]
            }
        };
        if launched.reconnect {
            steps.push(Step::ConnectRequested(AttemptReason::User));
        }
        steps
    }

    pub(in crate::connection::machine) fn change_settings(
        &mut self,
        change: SettingsChange,
    ) -> Vec<Step> {
        self.settings = change.settings;
        let recheck_account = change.account.is_some_and(|account| {
            if account.drop_certs {
                self.effects.push(Effect::ClearTrustedCerts);
            }
            if account.forget_gateway_ip {
                self.system.gateway_ip = None;
            }
            if let Some(ledger) = account.new_ledger {
                self.ledger = ledger;
            }
            account.recheck
        });
        if let Some(replan) = change.replan {
            let Phase::Connected(session) = &mut self.phase else {
                unreachable!("decided for the connected session");
            };
            session.planned_reconnect = replan.planned_reconnect;
            if replan.end_renewal {
                session.renewal = None;
            }
        }
        [
            (recheck_account, Step::CheckReadiness),
            (change.schedule_changed, Step::ScheduleChanged),
            (change.recheck_otp, Step::CheckReadiness),
        ]
        .into_iter()
        .filter_map(|(wanted, step)| wanted.then_some(step))
        .collect()
    }

    pub(in crate::connection::machine) fn change_time_zone(
        &mut self,
        time_zone: TimeZone,
    ) -> Vec<Step> {
        self.time_zone = time_zone;
        vec![Step::ResetScheduleBoundary, Step::RejudgeHeldReconnect]
    }

    pub(in crate::connection::machine) fn connect(
        &mut self,
        request: ConnectRequest,
        reason: AttemptReason,
    ) -> Vec<Step> {
        match request {
            ConnectRequest::Begin => {
                self.phase = Phase::Idle;
                self.wants_connection = true;
                self.reset_for_user_action();
                vec![Step::begin_attempt(reason)]
            }
            ConnectRequest::AfterStop { reset } => {
                self.wants_connection = true;
                if reset {
                    self.reset_for_user_action();
                    self.stop_under_way().after = AfterStop::Reconnect(reason);
                }
                Vec::new()
            }
            ConnectRequest::ResumeHeld => {
                self.phase = Phase::Idle;
                self.reconnect_limits.reconnect_suspended = false;
                vec![Step::begin_attempt(reason)]
            }
            ConnectRequest::Unsuspend => {
                self.reconnect_limits.reconnect_suspended = false;
                Vec::new()
            }
        }
    }

    pub(in crate::connection::machine) fn disconnect(
        &mut self,
        request: DisconnectRequest,
    ) -> Vec<Step> {
        self.wants_connection = false;
        match request {
            DisconnectRequest::EndWait => self.phase = Phase::Idle,
            DisconnectRequest::AbortAttempt => {
                return vec![self.abort_attempt_step(StopReason::UserDisconnect, AfterStop::Idle)];
            }
            DisconnectRequest::StopSession => {
                self.stop_connected_session(StopReason::UserDisconnect, AfterStop::Idle);
            }
            DisconnectRequest::StayIdle(stays) => {
                if stays {
                    self.stop_under_way().after = AfterStop::Idle;
                }
            }
            DisconnectRequest::Nothing => {}
        }
        Vec::new()
    }

    pub(in crate::connection::machine) fn reconnect_now(
        &mut self,
        request: ReconnectNowRequest,
    ) -> Vec<Step> {
        match request {
            ReconnectNowRequest::Renew => {
                let Phase::Connected(session) = self.take_phase() else {
                    unreachable!("decided for the connected session");
                };
                self.reset_for_user_action();
                crate::connection::machine::attempts::action::renew(
                    session,
                    AttemptReason::ReconnectNow,
                )
            }
            ReconnectNowRequest::Begin => {
                self.phase = Phase::Idle;
                self.wants_connection = true;
                self.reset_for_user_action();
                vec![Step::begin_attempt(AttemptReason::ReconnectNow)]
            }
            ReconnectNowRequest::Retarget => {
                let attempt = self.attempt_under_way();
                attempt.reason = AttemptReason::ReconnectNow;
                let renewal = attempt
                    .replacing
                    .as_mut()
                    .and_then(|session| session.renewal.as_mut())
                    .expect("a replacement is prepared for a renewal");
                renewal.reason = AttemptReason::ReconnectNow;
                renewal.tries = 0;
                renewal.later_failures = 0;
                self.reset_for_user_action();
                Vec::new()
            }
            ReconnectNowRequest::Nothing => Vec::new(),
        }
    }

    pub(in crate::connection::machine) fn quit(
        &mut self,
        (first_asked_at, request): (Option<SystemTime>, StopRequest),
    ) -> Vec<Step> {
        self.wants_connection = false;
        if let Some(at) = first_asked_at {
            self.quit = Quit::Requested { at };
        }
        match request {
            StopRequest::AbortAttempt => {
                return vec![self.abort_attempt_step(StopReason::AppQuit, AfterStop::Quit)];
            }
            StopRequest::StopSession => {
                self.stop_connected_session(StopReason::AppQuit, AfterStop::Quit);
            }
            StopRequest::AfterStop => self.stop_under_way().after = AfterStop::Quit,
            StopRequest::EndWait | StopRequest::Done => self.quit = Quit::Terminated,
        }
        Vec::new()
    }

    pub(in crate::connection::machine) fn stop_for_update(
        &mut self,
        request: Option<(SystemTime, StopRequest)>,
    ) -> Vec<Step> {
        let Some((requested_at, request)) = request else {
            return Vec::new();
        };
        self.wants_connection = false;
        self.update_stop = Some(UpdateStop {
            requested_at,
            stopped: false,
        });
        match request {
            StopRequest::AbortAttempt => {
                vec![self.abort_attempt_step(StopReason::AppQuit, AfterStop::Update)]
            }
            StopRequest::StopSession => {
                self.stop_connected_session(StopReason::AppQuit, AfterStop::Update);
                Vec::new()
            }
            StopRequest::AfterStop => {
                self.stop_under_way().after = AfterStop::Update;
                Vec::new()
            }
            StopRequest::EndWait => {
                self.phase = Phase::Idle;
                self.settle(AfterStop::Update)
            }
            StopRequest::Done => vec![Step::StoppedForUpdate],
        }
    }

    pub(in crate::connection::machine) fn report_stopped_for_update(&mut self, complete: bool) {
        if !complete {
            return;
        }
        self.update_stop
            .as_mut()
            .expect("decided for the stop for an update")
            .stopped = true;
        self.effects.push(Effect::StoppedForUpdate);
    }

    pub(in crate::connection::machine) fn fail_update(
        &mut self,
        failure: Option<bool>,
        reconnect: bool,
    ) -> Vec<Step> {
        let Some(stop_stays_idle) = failure else {
            return Vec::new();
        };
        self.update_stop = None;
        if stop_stays_idle {
            self.stop_under_way().after = AfterStop::Idle;
        }
        if reconnect {
            vec![Step::ConnectRequested(AttemptReason::User)]
        } else {
            Vec::new()
        }
    }

    pub(in crate::connection::machine) fn end_at_quit_deadline(
        &mut self,
        ends_quit: bool,
    ) -> Vec<Step> {
        if ends_quit {
            self.quit = Quit::Terminated;
            Vec::new()
        } else {
            vec![Step::StoppedForUpdate]
        }
    }

    /// Whether the screen is locked decides presence; the answer judges.
    pub(in crate::connection::machine) fn app_activated(&mut self) -> Vec<Step> {
        self.effects.push(Effect::ReadScreenState);
        vec![Step::CheckReadiness]
    }

    fn reset_for_user_action(&mut self) {
        self.ledger.reset_for_user_action();
        self.reconnect_limits.network_failure_streak = 0;
        self.reconnect_limits.forced_retry_used = false;
        self.reconnect_limits.reconnect_suspended = false;
    }

    fn abort_attempt_step(&mut self, stop: StopReason, after: AfterStop) -> Step {
        let Phase::Connecting(attempt) = self.take_phase() else {
            unreachable!("decided for an attempt under way");
        };
        Step::AbortAttempt {
            attempt,
            stop,
            after,
        }
    }

    fn stop_connected_session(&mut self, stop: StopReason, after: AfterStop) {
        let Phase::Connected(session) = self.take_phase() else {
            unreachable!("decided for the connected session");
        };
        self.stop_session(session, stop, after, None);
    }

    fn stop_under_way(&mut self) -> &mut crate::connection::machine::Stop {
        let Phase::Disconnecting(stop) = &mut self.phase else {
            unreachable!("decided for a stop under way");
        };
        stop
    }
}
