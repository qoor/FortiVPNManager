//! What the connection, schedule and readiness sections show (SPEC F-CM-10). Each is a function
//! of the state as of the last input: every time-bound change, such as a schedule boundary, comes
//! with a timer input.

use fvm_contract::app_helper::Milestone;
use fvm_contract::app_internal::ButtonState;
use fvm_contract::app_internal::section::{
    AfterDisconnect, ConnectProgress, ConnectionIcon, ConnectionSection, ConnectionStatus,
    FailureReason, HelperState, ProactiveReconnect, ReadinessSection, ScheduleOutlook, SetupItem,
    VpnClientInfo, WaitReason,
};

use super::ledger::Failure;
use super::machine::{AfterStop, AttemptReason, ConnectionManager, Phase, RetryReason, Stage};
use super::policy;
use super::readiness::{HelperFacts, SetupNeed};

impl ConnectionManager {
    pub fn connection_section(&self) -> ConnectionSection {
        let attention = matches!(self.phase, Phase::SetupRequired(_) | Phase::Failed(_));
        ConnectionSection {
            icon: self.icon(),
            status: self.status(),
            wait_reason: match &self.phase {
                Phase::WaitingRetry { reason, .. } => Some(self.wait_reason(*reason)),
                Phase::WaitingNetwork { reason, .. } if *reason != RetryReason::Network => {
                    Some(self.wait_reason(*reason))
                }
                _ => None,
            },
            connect: shown(!self.wants_connection || attention),
            disconnect: shown(self.wants_connection),
            reconnect_now: if matches!(self.phase, Phase::Connected(_) | Phase::WaitingRetry { .. })
            {
                ButtonState::Enabled
            } else {
                ButtonState::Disabled
            },
            account_applies_next_connection: self.account_applies_next_connection(),
        }
    }

    /// What the schedule does next; `None` while it is off.
    pub fn schedule_outlook(&self) -> Option<ScheduleOutlook> {
        let schedule = self.settings.connection.schedule.as_ref()?;
        let wants_connection = self.wants_connection && !matches!(self.phase, Phase::Failed(_));
        if !wants_connection {
            return Some(
                schedule
                    .next_start(self.now, &self.time_zone)
                    .map_or(ScheduleOutlook::NoDays {}, |at| ScheduleOutlook::Connects {
                        at,
                    }),
            );
        }
        if schedule.is_active(self.now, &self.time_zone)
            && let Some(next) = schedule.next_boundary(self.now, &self.time_zone)
            && next.edge == super::schedule::Edge::End
        {
            return Some(ScheduleOutlook::WithinHours { end: next.at });
        }
        Some(if self.reconnect_suspended {
            ScheduleOutlook::Suspended {}
        } else if let Some(away_since) = self.away_since {
            ScheduleOutlook::HoldingWhileAway {
                until: away_since + schedule.away_limit(),
            }
        } else {
            ScheduleOutlook::OutsideHoursPresent {}
        })
    }

    pub fn readiness_section(&self) -> ReadinessSection {
        let helper = match &self.helper {
            None => HelperState::Checking {},
            Some(HelperFacts::NotRegistered) => HelperState::NotRegistered {},
            Some(HelperFacts::RequiresApproval) => HelperState::RequiresApproval {},
            Some(HelperFacts::Unreachable(failure)) => HelperState::Unreachable {
                failure: failure.clone(),
            },
            Some(HelperFacts::Answered { build, .. }) if *build == self.app_build => {
                HelperState::Running {
                    build: build.clone(),
                }
            }
            Some(HelperFacts::Answered { build, .. }) => HelperState::VersionMismatch {
                helper_build: build.clone(),
                app_build: self.app_build.clone(),
            },
        };
        let client = match &self.helper {
            Some(HelperFacts::Answered {
                build,
                status: Some(status),
            }) if *build == self.app_build => Some(VpnClientInfo {
                path: status.client_path.clone(),
                version: status.client_version.clone(),
                untested_version: status.untested_version.clone(),
                problem: status.client_problem.clone(),
            }),
            _ => None,
        };
        ReadinessSection {
            register_helper: shown(matches!(helper, HelperState::NotRegistered {})),
            helper,
            register_error: self.register_error.clone(),
            client,
        }
    }

    fn icon(&self) -> ConnectionIcon {
        match &self.phase {
            Phase::Connected(_) => ConnectionIcon::Connected,
            // Waiting for an away user is not a reconnect under way.
            Phase::Idle => ConnectionIcon::Disconnected,
            phase if phase.is_holding() => ConnectionIcon::Disconnected,
            Phase::Connecting(_)
            | Phase::WaitingRetry { .. }
            | Phase::WaitingNetwork { .. }
            | Phase::Disconnecting(_) => ConnectionIcon::InProgress,
            Phase::SetupRequired(_) | Phase::Failed(_) => ConnectionIcon::Error,
        }
    }

    fn status(&self) -> ConnectionStatus {
        match &self.phase {
            Phase::Idle => ConnectionStatus::NotConnected {},
            Phase::SetupRequired(needs) => ConnectionStatus::SetupRequired {
                items: needs.iter().map(|need| self.setup_item(*need)).collect(),
            },
            Phase::Connecting(attempt) => ConnectionStatus::Connecting {
                reconnecting: match attempt.reason {
                    AttemptReason::Launch
                    | AttemptReason::User
                    | AttemptReason::SetupCompleted
                    | AttemptReason::CertificateRetry
                    | AttemptReason::Scheduled => false,
                    AttemptReason::ReconnectNow
                    | AttemptReason::Retry(_)
                    | AttemptReason::Proactive
                    | AttemptReason::NetworkChanged
                    | AttemptReason::TunnelLost
                    | AttemptReason::AfterSleep => true,
                },
                progress: match (attempt.stage, attempt.milestone) {
                    (Stage::CheckingReadiness, _) => ConnectProgress::CheckingReadiness,
                    (Stage::PreparingOtp, _) => ConnectProgress::ConnectingToGmail,
                    (Stage::Run, None | Some(Milestone::Spawned)) => {
                        ConnectProgress::ContactingServer
                    }
                    (Stage::Run, Some(Milestone::ConnectedToGateway)) => ConnectProgress::LoggingIn,
                    (Stage::Run, Some(Milestone::OtpPrompted)) => ConnectProgress::WaitingForOtp,
                    (Stage::Run, Some(Milestone::OtpSubmitted)) => ConnectProgress::VerifyingOtp,
                    (Stage::Run, Some(Milestone::Authenticated | Milestone::TunnelUp)) => {
                        ConnectProgress::OpeningTunnel
                    }
                },
            },
            Phase::Connected(session) => ConnectionStatus::Connected {
                since: session.since,
                tunnel_ip: session.tunnel_ip,
                proactive_reconnect: match session.planned_reconnect {
                    None => ProactiveReconnect::Off {},
                    Some(at) if self.automatic_reconnect_allowed() => ProactiveReconnect::At { at },
                    Some(_) => ProactiveReconnect::PausedBySchedule {},
                },
            },
            Phase::WaitingRetry {
                until,
                reason: RetryReason::UserAway,
            } => ConnectionStatus::ReconnectOnHold { until: *until },
            Phase::WaitingRetry { until, .. } => {
                ConnectionStatus::WaitingToReconnect { until: *until }
            }
            Phase::WaitingNetwork { .. } => ConnectionStatus::WaitingForNetwork {},
            Phase::Disconnecting(stop) => ConnectionStatus::Disconnecting {
                then: match stop.after {
                    AfterStop::Idle => AfterDisconnect::Idle,
                    AfterStop::Reconnect(_) => AfterDisconnect::Reconnect,
                    AfterStop::ResumeAfterSleep => AfterDisconnect::Sleep,
                    AfterStop::Quit => AfterDisconnect::Quit,
                },
            },
            Phase::Failed(failure) => ConnectionStatus::Failed {
                reason: self.failure_reason(*failure),
            },
        }
    }

    fn wait_reason(&self, reason: RetryReason) -> WaitReason {
        match reason {
            RetryReason::Network => WaitReason::Network {},
            RetryReason::ServerSessionEnded => WaitReason::ServerEndedSession {},
            RetryReason::ForcedTermination => WaitReason::ForcedTermination {},
            RetryReason::AuthFailure(failure) => WaitReason::AuthFailure {
                reason: self.failure_reason(failure),
            },
            RetryReason::AuthCooldown => WaitReason::AuthCooldown {
                delay_seconds: policy::AUTH_FAILURE_RETRY_DELAY.as_secs(),
            },
            RetryReason::Sleep => WaitReason::Sleep {},
            RetryReason::UserAway => WaitReason::UserAway {},
        }
    }

    /// Named after the OTP method selected now.
    fn failure_reason(&self, failure: Failure) -> FailureReason {
        match failure {
            Failure::AuthenticationRejected => FailureReason::VpnLoginRejected {},
            Failure::OtpNotReceived { in_spam } => FailureReason::OtpNotReceived {
                in_spam,
                waited_seconds: policy::OTP_DEADLINE.as_secs(),
            },
            Failure::OtpRejected => FailureReason::OtpRejected {},
            Failure::TunnelSetupFailed => FailureReason::TunnelSetupFailed {},
            Failure::TunnelModeRejected => FailureReason::TunnelModeRejected {},
            Failure::SessionDroppedEarly => FailureReason::DroppedEarly {
                within_seconds: policy::STABLE_SESSION.as_secs(),
            },
            Failure::CertificateRejectedAfterLogin => {
                FailureReason::CertificateChangedAfterLogin {}
            }
            Failure::GmailRejected => FailureReason::GmailRejected {
                method: self.settings.otp_method,
            },
            Failure::TooManyReconnects => FailureReason::TooManyReconnects {
                window_seconds: policy::AUTH_ATTEMPT_WINDOW.as_secs(),
            },
            Failure::Network => FailureReason::Network {},
            Failure::Helper(code) => FailureReason::Helper { code },
        }
    }

    fn setup_item(&self, need: SetupNeed) -> SetupItem {
        match need {
            SetupNeed::ConfigFile => SetupItem::ConfigFile {},
            SetupNeed::AccountDetails => SetupItem::AccountDetails {},
            SetupNeed::VpnPassword => SetupItem::VpnPassword {},
            SetupNeed::GmailCredential => SetupItem::GmailCredential {
                method: self.settings.otp_method,
            },
            SetupNeed::HelperApproval => SetupItem::HelperApproval {},
            SetupNeed::HelperVersion => SetupItem::HelperVersion {},
        }
    }

    /// The account in the settings differs from the one the attempt or session under way started
    /// with; account edits never touch a running tunnel.
    fn account_applies_next_connection(&self) -> bool {
        let started_with = match &self.phase {
            Phase::Connecting(attempt) => Some(&attempt.account),
            Phase::Connected(_) | Phase::Disconnecting(_) => self.run_account.as_ref(),
            _ => None,
        };
        started_with.is_some_and(|account| !account.same_login(&self.settings.account))
    }
}

fn shown(visible: bool) -> ButtonState {
    if visible {
        ButtonState::Enabled
    } else {
        ButtonState::Hidden
    }
}
