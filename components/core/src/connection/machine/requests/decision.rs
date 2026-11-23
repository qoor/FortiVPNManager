use std::time::SystemTime;

use crate::connection::certs::gateway_change_drops_certs;
use crate::connection::ledger::{Failure, FailureLedger};
use crate::connection::machine::{AfterStop, AttemptReason, ConnectionManager, Phase, Quit, Step};
use crate::connection::{Launch, Settings, policy};

/// The launch, with the ledger it keeps.
pub(in crate::connection::machine) struct Launched {
    /// The ledger state.json held, when it is the account's.
    pub(super) restored: Option<FailureLedger>,
    pub(super) kept_attempts: Vec<SystemTime>,
    pub(super) schedule_boundary: Option<SystemTime>,
    pub(super) open_settings: bool,
    pub(super) start: LaunchStart,
    /// The connection the update stopped is made again.
    pub(super) reconnect: bool,
}

pub(in crate::connection::machine) enum LaunchStart {
    /// Not connecting at launch: readiness is checked.
    Wait,
    /// The retries had stopped before the relaunch; only the user clears that.
    Fail(Failure),
    Attempt,
}

/// What new settings change.
pub(in crate::connection::machine) struct SettingsChange {
    pub(super) settings: Settings,
    pub(super) account: Option<AccountChange>,
    pub(super) schedule_changed: bool,
    pub(super) replan: Option<Replan>,
    pub(super) recheck_otp: bool,
}

/// What a changed account changes.
pub(in crate::connection::machine) struct AccountChange {
    /// The trusted values belong to the old gateway.
    pub(super) drop_certs: bool,
    pub(super) forget_gateway_ip: bool,
    pub(super) new_ledger: Option<FailureLedger>,
    pub(super) recheck: bool,
}

/// The connected session's planned reconnect after the session settings changed.
pub(in crate::connection::machine) struct Replan {
    pub(super) planned_reconnect: Option<SystemTime>,
    /// A renewal waiting for nothing ends with the old plan.
    pub(super) end_renewal: bool,
}

/// What Connect, or the start of the scheduled hours, does.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum ConnectRequest {
    Begin,
    /// A stop under way connects again after the exit; `reset` when it was to stay idle.
    AfterStop {
        reset: bool,
    },
    /// The reconnect held for an away user goes ahead.
    ResumeHeld,
    /// Already connecting or connected: only the suspension is lifted.
    Unsuspend,
}

#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum DisconnectRequest {
    /// A wait or a failure ends.
    EndWait,
    AbortAttempt,
    StopSession,
    /// A stop under way stays idle after the exit, unless it quits.
    StayIdle(bool),
    Nothing,
}

#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum ReconnectNowRequest {
    Renew,
    Begin,
    /// A replacement prepared for another reason becomes a Reconnect Now one.
    Retarget,
    /// Restarting an attempt under way could waste an OTP or a login.
    Nothing,
}

/// How a quit, or the stop for an update, ends what is under way.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum StopRequest {
    AbortAttempt,
    StopSession,
    /// A stop under way ends this way after the exit.
    AfterStop,
    /// A wait ends at once.
    EndWait,
    /// Nothing runs: the request completes at once.
    Done,
}

impl ConnectionManager {
    /// The manager takes no input after it approved the quit.
    pub(in crate::connection::machine) fn takes_inputs(&self) -> bool {
        self.quit != Quit::Terminated
    }

    /// Connect, Disconnect and Reconnect Now are ignored while the update stops the connection.
    fn takes_user_requests(&self) -> bool {
        self.update_stop.is_none()
    }

    /// The user's Connect, unless it is ignored.
    pub(in crate::connection::machine) fn user_connect(&self) -> Vec<Step> {
        if self.takes_user_requests() {
            vec![Step::ConnectRequested(AttemptReason::User)]
        } else {
            Vec::new()
        }
    }

    pub(in crate::connection::machine) fn launched(
        &self,
        launch: Launch,
        now: SystemTime,
    ) -> Launched {
        let restored = launch
            .restored_ledger
            .filter(|restored| restored.account_key == self.ledger.account_key);
        let ledger = restored.as_ref().unwrap_or(&self.ledger);
        let start = if !launch.auto_connect || !self.within_schedule(now) {
            LaunchStart::Wait
        } else if ledger.consecutive_auth_failures >= policy::MAX_CONSECUTIVE_AUTH_FAILURES {
            LaunchStart::Fail(
                ledger
                    .last_auth_failure
                    .unwrap_or(Failure::AuthenticationRejected),
            )
        } else {
            LaunchStart::Attempt
        };
        Launched {
            kept_attempts: ledger.kept_attempts(now),
            restored,
            schedule_boundary: self.schedule_boundary_at(now),
            open_settings: launch.import_report_shown,
            start,
            reconnect: launch.connected_before_update,
        }
    }

    pub(in crate::connection::machine) fn settings_change(
        &self,
        mut settings: Settings,
        now: SystemTime,
    ) -> SettingsChange {
        let old = &self.settings;
        let account_changed = settings.account != old.account;
        let drop_certs =
            account_changed && gateway_change_drops_certs(&old.account, &settings.account);
        if drop_certs {
            // The next connection learns the new gateway's certificate.
            settings.account.trusted_certs.clear();
        }
        let key = settings.account.ledger_key();
        let connection_changed = settings.connection != old.connection;
        let replans = connection_changed
            && (settings.connection.session_max_minutes != old.connection.session_max_minutes
                || settings.connection.proactive_reconnect != old.connection.proactive_reconnect
                || settings.connection.proactive_margin_minutes
                    != old.connection.proactive_margin_minutes);
        let replan = match &self.phase {
            Phase::Connected(session) if replans => Some(Replan {
                planned_reconnect: policy::planned_reconnect(
                    session.since,
                    &settings.connection,
                    now,
                ),
                end_renewal: session
                    .renewal
                    .is_some_and(|renewal| renewal.next_try.is_none()),
            }),
            _ => None,
        };
        SettingsChange {
            account: account_changed.then(|| AccountChange {
                drop_certs,
                forget_gateway_ip: settings.account.host != old.account.host,
                new_ledger: (key != self.ledger.account_key).then(|| FailureLedger::new(key)),
                recheck: !settings.account.same_identity(&old.account),
            }),
            schedule_changed: connection_changed
                && settings.connection.schedule != old.connection.schedule,
            replan,
            recheck_otp: settings.otp_method != old.otp_method,
            settings,
        }
    }

    pub(in crate::connection::machine) fn connect_request(&self) -> ConnectRequest {
        match &self.phase {
            Phase::Idle | Phase::SetupRequired(_) | Phase::Failed(_) => ConnectRequest::Begin,
            Phase::Disconnecting(stop) => ConnectRequest::AfterStop {
                reset: stop.after == AfterStop::Idle,
            },
            Phase::WaitingRetry {
                reason: crate::connection::machine::RetryReason::UserAway,
                ..
            } => ConnectRequest::ResumeHeld,
            _ => ConnectRequest::Unsuspend,
        }
    }

    /// What Disconnect stops; `None` while it is ignored.
    pub(in crate::connection::machine) fn disconnect_request(&self) -> Option<DisconnectRequest> {
        if !self.takes_user_requests() {
            return None;
        }
        Some(match &self.phase {
            Phase::Failed(_) | Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } => {
                DisconnectRequest::EndWait
            }
            Phase::Connecting(_) => DisconnectRequest::AbortAttempt,
            Phase::Connected(_) => DisconnectRequest::StopSession,
            Phase::Disconnecting(stop) => {
                DisconnectRequest::StayIdle(stop.after != AfterStop::Quit)
            }
            Phase::Idle | Phase::SetupRequired(_) => DisconnectRequest::Nothing,
        })
    }

    pub(in crate::connection::machine) fn reconnect_now_request(&self) -> ReconnectNowRequest {
        if !self.takes_user_requests() {
            return ReconnectNowRequest::Nothing;
        }
        match &self.phase {
            Phase::Connected(_) => ReconnectNowRequest::Renew,
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } | Phase::Failed(_) => {
                ReconnectNowRequest::Begin
            }
            Phase::Connecting(attempt)
                if attempt.replacing.is_some() && attempt.reason != AttemptReason::ReconnectNow =>
            {
                ReconnectNowRequest::Retarget
            }
            _ => ReconnectNowRequest::Nothing,
        }
    }

    /// How the quit ends what is under way, and when it was first asked for.
    pub(in crate::connection::machine) fn quit_request(
        &self,
        now: SystemTime,
    ) -> (Option<SystemTime>, StopRequest) {
        (
            (self.quit == Quit::NotRequested).then_some(now),
            self.stop_request(),
        )
    }

    /// When the stop for an update was asked for and how it ends what is under way; `None` once
    /// quitting.
    pub(in crate::connection::machine) fn update_stop_request(
        &self,
        now: SystemTime,
    ) -> Option<(SystemTime, StopRequest)> {
        (self.quit == Quit::NotRequested).then(|| (now, self.stop_request()))
    }

    fn stop_request(&self) -> StopRequest {
        match &self.phase {
            Phase::Connecting(_) => StopRequest::AbortAttempt,
            Phase::Connected(_) => StopRequest::StopSession,
            Phase::Disconnecting(_) => StopRequest::AfterStop,
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } => StopRequest::EndWait,
            Phase::Idle | Phase::SetupRequired(_) | Phase::Failed(_) => StopRequest::Done,
        }
    }

    /// The stop for the update is complete, unless it was reported already.
    pub(in crate::connection::machine) fn update_stop_complete(&self) -> bool {
        self.update_stop.is_some_and(|stop| !stop.stopped)
    }

    /// Whether the update's failure applies, and whether a stop under way stays idle after it.
    pub(in crate::connection::machine) fn update_failure(&self) -> Option<bool> {
        (self.quit == Quit::NotRequested).then(
            || matches!(&self.phase, Phase::Disconnecting(stop) if stop.after == AfterStop::Update),
        )
    }

    /// Whether the quit deadline ends a quit; otherwise it ends the stop for an update.
    pub(in crate::connection::machine) fn quit_deadline_ends_quit(&self) -> bool {
        self.quit != Quit::NotRequested
    }
}
