use std::net::Ipv4Addr;
use std::time::SystemTime;

use fvm_contract::app_helper::{AttemptId, EndFacts, HelperErrorCode, Milestone};

use crate::connection::OtpFailure;
use crate::connection::classify::{ExitContext, Outcome, classify};
use crate::connection::ledger::Failure;
use crate::connection::machine::attempts::decision::{Restart, restart_for};
use crate::connection::machine::otp::decision::OtpClose;
use crate::connection::machine::{
    AfterStop, ConnectionManager, OtpSession, Phase, Quit, RetryReason, Stage,
};
use crate::connection::policy;
use crate::connection::system::Underlay;

/// A milestone the attempt's run reached, and what it sets off.
pub(in crate::connection::machine) enum MilestoneReached {
    Ignore,
    Reached {
        milestone: Milestone,
        /// The recorded attempts once this one counts toward the attempt limit.
        counted: Option<Vec<SystemTime>>,
        /// When the credentials went out, once the certificate check passed.
        credentials_sent_at: Option<SystemTime>,
        then: AfterMilestone,
    },
}

pub(in crate::connection::machine) enum AfterMilestone {
    Nothing,
    SearchOtp { prompted_at: SystemTime },
    EnterConnected { tunnel_ip: Option<Ipv4Addr> },
}

/// The tunnel that came up for the attempt.
pub(in crate::connection::machine) struct Connected {
    pub(super) otp_close: Option<OtpClose>,
    pub(super) since: SystemTime,
    pub(super) tunnel_ip: Option<Ipv4Addr>,
    pub(super) planned_reconnect: Option<SystemTime>,
    pub(super) underlay: Option<Underlay>,
}

/// What a run's exit ends.
pub(in crate::connection::machine) enum RunExit {
    Ignore,
    /// The attempt's own run ended.
    Attempt(Box<AttemptExit>),
    /// The run the attempt was to replace ended first.
    ReplacedDuringAttempt,
    /// The run the session replaced ended.
    ReplacedWhileConnected,
    /// The connected session's run ended.
    Session {
        id: AttemptId,
        learned: Option<Vec<String>>,
        outcome: Outcome,
    },
    /// A run being stopped ended.
    Stop {
        learned: Option<Vec<String>>,
        then: StopEnded,
    },
}

/// How the attempt's own run ended.
pub(in crate::connection::machine) struct AttemptExit {
    pub(super) id: AttemptId,
    pub(super) learned: Option<Vec<String>>,
    pub(super) otp_close: Option<OtpClose>,
    pub(super) then: AfterAttemptExit,
}

pub(in crate::connection::machine) enum AfterAttemptExit {
    /// The new run was handed the replaced run's address: break before make.
    RestartReplaced(Restart),
    ReplacementEnded(Outcome),
    Outcome(Outcome),
}

/// What follows the end of a run being stopped.
pub(in crate::connection::machine) enum StopEnded {
    /// The stop's replacement still runs; it becomes the run awaited when `next` is set.
    AwaitReplacement {
        next: Option<AttemptId>,
    },
    Settle(AfterStop),
}

/// What a run the Helper did not take ends.
pub(in crate::connection::machine) enum RunRejection {
    Ignore,
    Attempt {
        otp_close: Option<OtpClose>,
        then: AfterRejection,
    },
    Stop(StopEnded),
}

pub(in crate::connection::machine) enum AfterRejection {
    ReplacementFailed,
    Fail(Failure),
    NetworkFailure,
}

/// What the loss of the connection that carries the runs ends. The connection stays inside the
/// Mac: it breaks only when the Helper process ends, killed or crashed, never because of the
/// network.
pub(in crate::connection::machine) enum ConnectionLoss {
    Ignore,
    /// The run the attempt was to replace is gone; the attempt goes on before its own run.
    DropReplaced,
    EndRun {
        otp_close: Option<OtpClose>,
        then: AfterLoss,
    },
    EndSession,
    Settle(AfterStop),
}

pub(in crate::connection::machine) enum AfterLoss {
    Fail(Failure),
    ForcedTermination,
}

impl ConnectionManager {
    pub(in crate::connection::machine) fn milestone_reached(
        &self,
        id: &AttemptId,
        milestone: Milestone,
        tunnel_ip: Option<Ipv4Addr>,
        now: SystemTime,
    ) -> MilestoneReached {
        let Phase::Connecting(attempt) = &self.phase else {
            return MilestoneReached::Ignore;
        };
        if attempt.stage != Stage::Run || attempt.id != *id || attempt.stop_requested.is_some() {
            return MilestoneReached::Ignore;
        }
        let previous = attempt.milestone;
        if previous.is_some_and(|previous| milestone <= previous) {
            return MilestoneReached::Ignore;
        }
        let credentials_sent = previous.unwrap_or(Milestone::Started)
            < Milestone::ConnectedToGateway
            && milestone >= Milestone::ConnectedToGateway;
        // The certificate check passed and the credentials go out next: this counts toward the
        // attempt limit.
        let counted = credentials_sent
            && attempt
                .replacing
                .as_ref()
                .and_then(|session| session.renewal)
                .is_none_or(|renewal| renewal.tries < policy::MAX_COUNTED_PREPARATIONS);
        MilestoneReached::Reached {
            milestone,
            counted: counted.then(|| self.ledger.attempts_with(now)),
            credentials_sent_at: credentials_sent.then_some(now),
            then: match milestone {
                Milestone::TunnelUp => AfterMilestone::EnterConnected { tunnel_ip },
                Milestone::OtpPrompted => AfterMilestone::SearchOtp { prompted_at: now },
                Milestone::Started
                | Milestone::ConnectedToGateway
                | Milestone::OtpSubmitted
                | Milestone::Authenticated => AfterMilestone::Nothing,
            },
        }
    }

    pub(in crate::connection::machine) fn connected(
        &self,
        tunnel_ip: Option<Ipv4Addr>,
        now: SystemTime,
    ) -> Connected {
        let Phase::Connecting(attempt) = &self.phase else {
            unreachable!("a tunnel comes up for an attempt");
        };
        Connected {
            // Still open only when the gateway asked for no OTP.
            otp_close: attempt.otp_close(false, true, now),
            since: now,
            tunnel_ip,
            planned_reconnect: policy::planned_reconnect(now, &self.settings.connection, now),
            underlay: self.path.as_ref().and_then(|path| path.underlay.clone()),
        }
    }

    pub(in crate::connection::machine) fn run_exit(
        &self,
        id: &AttemptId,
        facts: &EndFacts,
        now: SystemTime,
    ) -> RunExit {
        match &self.phase {
            Phase::Connecting(attempt) if attempt.stage == Stage::Run && attempt.id == *id => {
                let outcome = classify(&ExitContext {
                    facts,
                    seen_milestone: attempt.milestone,
                    cert_retry_used: attempt.cert_retry_used,
                    app_stop: attempt.stop_requested,
                    otp_failure: attempt.otp_failure,
                    otp_in_spam: attempt.otp_in_spam,
                });
                // No code email follows a login the gateway rejected, and its counted failure
                // keeps the next login 90 s away, past the prompt's arrival tolerance.
                let search = outcome != Outcome::AuthFailure(Failure::AuthenticationRejected);
                RunExit::Attempt(Box::new(AttemptExit {
                    id: id.clone(),
                    learned: self.learned_certificates(facts),
                    otp_close: attempt.otp_close(false, search, now),
                    then: match &attempt.replacing {
                        Some(_) if facts.tunnel_address_in_use => {
                            AfterAttemptExit::RestartReplaced(restart_for(attempt.reason))
                        }
                        Some(_) => AfterAttemptExit::ReplacementEnded(outcome),
                        None => AfterAttemptExit::Outcome(outcome),
                    },
                }))
            }
            Phase::Connecting(attempt)
                if attempt
                    .replacing
                    .as_ref()
                    .is_some_and(|session| session.attempt == *id) =>
            {
                RunExit::ReplacedDuringAttempt
            }
            Phase::Connected(session) if session.replaced.as_ref() == Some(id) => {
                RunExit::ReplacedWhileConnected
            }
            Phase::Connected(session) if session.attempt == *id => RunExit::Session {
                id: id.clone(),
                learned: self.learned_certificates(facts),
                outcome: classify(&ExitContext {
                    facts,
                    seen_milestone: Some(Milestone::TunnelUp),
                    cert_retry_used: true,
                    app_stop: None,
                    otp_failure: None,
                    otp_in_spam: false,
                }),
            },
            Phase::Disconnecting(_) => match self.stop_ended(id) {
                Some(then) => RunExit::Stop {
                    learned: self.learned_certificates(facts),
                    then,
                },
                None => RunExit::Ignore,
            },
            // An attempt already finished with.
            _ => RunExit::Ignore,
        }
    }

    /// What follows the end of run `id` while a stop is under way, if the stop waits for it.
    fn stop_ended(&self, id: &AttemptId) -> Option<StopEnded> {
        let Phase::Disconnecting(stop) = &self.phase else {
            return None;
        };
        if stop.attempt != *id && stop.replacement.as_ref() != Some(id) {
            return None;
        }
        Some(match &stop.replacement {
            Some(replacement) => StopEnded::AwaitReplacement {
                next: (replacement != id).then(|| replacement.clone()),
            },
            None => StopEnded::Settle(stop.after),
        })
    }

    pub(in crate::connection::machine) fn run_rejection(
        &self,
        id: &AttemptId,
        code: HelperErrorCode,
        now: SystemTime,
    ) -> RunRejection {
        match &self.phase {
            Phase::Connecting(attempt) if attempt.stage == Stage::Run && attempt.id == *id => {
                RunRejection::Attempt {
                    otp_close: attempt.otp_close(false, true, now),
                    then: if attempt.replacing.is_some() {
                        AfterRejection::ReplacementFailed
                    } else {
                        match (attempt.otp_failure, code) {
                            (Some(OtpFailure::Authentication), _) => {
                                AfterRejection::Fail(Failure::GmailRejected)
                            }
                            (
                                None,
                                HelperErrorCode::InvalidRequest | HelperErrorCode::NotAuthorized,
                            ) => AfterRejection::Fail(Failure::Helper { code }),
                            (Some(OtpFailure::Network), _)
                            | (
                                None,
                                HelperErrorCode::Busy
                                | HelperErrorCode::WrongPhase
                                | HelperErrorCode::UnknownAttempt
                                | HelperErrorCode::InternalError,
                            ) => AfterRejection::NetworkFailure,
                        }
                    },
                }
            }
            Phase::Disconnecting(_) => self
                .stop_ended(id)
                .map_or(RunRejection::Ignore, RunRejection::Stop),
            _ => RunRejection::Ignore,
        }
    }

    pub(in crate::connection::machine) fn connection_loss(
        &self,
        now: SystemTime,
    ) -> ConnectionLoss {
        match &self.phase {
            Phase::Connecting(attempt) if attempt.replacing.is_some() => {
                if attempt.stage == Stage::Run {
                    ConnectionLoss::EndRun {
                        otp_close: attempt.otp_close(false, true, now),
                        then: AfterLoss::ForcedTermination,
                    }
                } else {
                    ConnectionLoss::DropReplaced
                }
            }
            Phase::Connecting(attempt) if attempt.stage == Stage::Run => ConnectionLoss::EndRun {
                otp_close: attempt.otp_close(false, true, now),
                then: if attempt.otp_failure == Some(OtpFailure::Authentication) {
                    AfterLoss::Fail(Failure::GmailRejected)
                } else {
                    AfterLoss::ForcedTermination
                },
            },
            Phase::Connected(_) => ConnectionLoss::EndSession,
            Phase::Disconnecting(stop) => ConnectionLoss::Settle(stop.after),
            _ => ConnectionLoss::Ignore,
        }
    }

    /// Whether the run of attempt `id` is still to be started: no stop was decided since it was
    /// asked for. A stop decided before the run's Connect left would reach the Helper first and
    /// find nothing to stop, so such a run is dropped instead.
    #[must_use]
    pub fn wants_run(&self, id: &AttemptId) -> bool {
        matches!(&self.phase, Phase::Connecting(attempt)
            if attempt.id == *id && attempt.stage == Stage::Run && attempt.stop_requested.is_none())
    }

    /// The attempt the connection is about now: the one connecting, connected or being stopped.
    /// Each attempt begun has a new one.
    #[must_use]
    pub fn current_attempt(&self) -> Option<&AttemptId> {
        match &self.phase {
            Phase::Connecting(attempt) => Some(&attempt.id),
            Phase::Connected(session) => Some(&session.attempt),
            Phase::Disconnecting(stop) => Some(&stop.attempt),
            _ => None,
        }
    }

    /// Whether the app wants the VPN up: connecting, connected, stopping to connect again, or
    /// waiting to try again. Not while it waits for an away user outside the scheduled hours, nor
    /// once it stopped for good: disconnected, paused after failures, or quitting. The exception
    /// routes are in place exactly then (user decision, 2026-11-16).
    #[must_use]
    pub fn wants_vpn(&self) -> bool {
        if self.quit == Quit::Terminated {
            return false;
        }
        match &self.phase {
            Phase::Connecting(_) | Phase::Connected(_) | Phase::WaitingNetwork { .. } => true,
            Phase::WaitingRetry { reason, .. } => *reason != RetryReason::UserAway,
            Phase::Disconnecting(stop) => matches!(
                stop.after,
                AfterStop::Reconnect(_) | AfterStop::ResumeAfterSleep
            ),
            Phase::Idle | Phase::SetupRequired(_) | Phase::Failed(_) => false,
        }
    }

    /// Whether attempt `id`'s OTP session is still to be opened: it was not closed while the
    /// session's secrets were read.
    #[must_use]
    pub fn wants_otp_session(&self, id: &AttemptId) -> bool {
        matches!(&self.phase, Phase::Connecting(attempt)
            if attempt.id == *id && attempt.otp_session == OtpSession::Open)
    }

    /// Whether a readiness check that found the Helper at `helper_build` should have it exit when
    /// idle, so the system starts this build's Helper: only another build, and only while no run
    /// is going on.
    #[must_use]
    pub fn should_restart_helper(&self, helper_build: &str) -> bool {
        helper_build != self.app_build && !self.run_active()
    }

    /// A run's events are taken now.
    pub(in crate::connection) fn run_active(&self) -> bool {
        match &self.phase {
            Phase::Connecting(attempt) => {
                attempt.stage == Stage::Run || attempt.replacing.is_some()
            }
            Phase::Connected(_) | Phase::Disconnecting(_) => true,
            _ => false,
        }
    }
}
