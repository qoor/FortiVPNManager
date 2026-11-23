use std::time::SystemTime;

use fvm_contract::app_helper::{AttemptId, Milestone};

use crate::connection::ledger::Failure;
use crate::connection::machine::{
    Attempt, ConnectionManager, OtpSession, Phase, RetryReason, Stage, StaleOtp,
};
use crate::connection::policy::{self, elapsed};
use crate::connection::{Effect, OtpFailure};

/// How an attempt's open OTP session closes.
pub(in crate::connection::machine) struct OtpClose {
    effect: Box<Effect>,
    /// The next login waits for the unused code email the session goes on looking for.
    stale: Option<StaleOtp>,
}

/// What the end of an OTP session ends.
pub(in crate::connection::machine) enum OtpSessionFailure {
    Ignore,
    /// The session looking for an earlier attempt's unused code ended.
    StaleSearchEnded,
    /// The VPN client was never started: a Gmail credential problem costs no VPN login.
    BeforeRun(AfterOtpFailure),
    /// No OTP can arrive any more: the run stops now instead of at the deadline; the exit
    /// classifies.
    DuringRun {
        failure: OtpFailure,
        stop: bool,
    },
}

pub(in crate::connection::machine) enum AfterOtpFailure {
    ReplacementFailed,
    Fail(Failure),
    NetworkFailure,
}

/// Where a code email found in the spam folder belongs.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum SpamFinding {
    Ignore,
    /// The attempt under way: its exit classifies with it.
    Attempt,
    /// The failure shown already, which it renames where it shows.
    ShownFailure {
        shown_in: ShownIn,
        ledger: bool,
    },
}

#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum ShownIn {
    Nowhere,
    Failure,
    WaitReason,
}

/// What the OTP time limit stops.
pub(in crate::connection::machine) enum OtpDeadline {
    Ignore,
    StopRun(Option<OtpClose>),
}

/// How the OTP session of an attempt whose readiness holds is prepared.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum OtpPreparation {
    /// An earlier attempt's unused code email is not in the trash yet.
    WaitForStaleOtp {
        since: SystemTime,
    },
    Open,
}

impl Attempt {
    /// How the session closes if it is open. Once the credentials went out and no code was used,
    /// the gateway may have sent a code email that arrives late, within the next prompt's arrival
    /// tolerance. With `search`, the session goes on looking for that email to trash it, from the
    /// prompt or else from when the credentials went out (user decision 2026-11-16). The callers
    /// turn it off on the app's quit and the Mac's sleep, at the OTP time limit and after a
    /// rejected login.
    pub(in crate::connection::machine) fn otp_close(
        &self,
        check_spam: bool,
        search: bool,
        now: SystemTime,
    ) -> Option<OtpClose> {
        if self.otp_session != OtpSession::Open {
            return None;
        }
        Some(match self.credentials_sent_at {
            Some(sent_at) if search && self.milestone <= Some(Milestone::OtpPrompted) => OtpClose {
                effect: Box::new(Effect::TrashStaleOtp {
                    attempt: self.id.clone(),
                    since: self.otp_prompted_at.unwrap_or(sent_at),
                }),
                stale: Some(StaleOtp {
                    attempt: self.id.clone(),
                    until: now + policy::STALE_OTP_WAIT,
                }),
            },
            _ => OtpClose {
                effect: Box::new(Effect::CloseOtpSession {
                    attempt: self.id.clone(),
                    check_spam,
                }),
                stale: None,
            },
        })
    }
}

impl OtpClose {
    pub(super) fn into_parts(self) -> (Effect, Option<StaleOtp>) {
        (*self.effect, self.stale)
    }
}

impl ConnectionManager {
    pub(in crate::connection::machine) fn otp_session_ready(&self, id: &AttemptId) -> bool {
        matches!(&self.phase, Phase::Connecting(attempt)
            if attempt.id == *id && attempt.stage == Stage::PreparingOtp)
    }

    pub(in crate::connection::machine) fn otp_session_failure(
        &self,
        id: &AttemptId,
        failure: OtpFailure,
    ) -> OtpSessionFailure {
        if self.stale_otp_session(id) {
            return OtpSessionFailure::StaleSearchEnded;
        }
        let Phase::Connecting(attempt) = &self.phase else {
            return OtpSessionFailure::Ignore;
        };
        if attempt.id != *id || attempt.otp_session != OtpSession::Open {
            return OtpSessionFailure::Ignore;
        }
        match attempt.stage {
            Stage::PreparingOtp => {
                OtpSessionFailure::BeforeRun(match (attempt.replacing.is_some(), failure) {
                    (true, _) => AfterOtpFailure::ReplacementFailed,
                    (false, OtpFailure::Authentication) => {
                        AfterOtpFailure::Fail(Failure::GmailRejected)
                    }
                    (false, OtpFailure::Network) => AfterOtpFailure::NetworkFailure,
                })
            }
            Stage::Run => OtpSessionFailure::DuringRun {
                failure,
                stop: attempt.stop_requested.is_none(),
            },
            Stage::CheckingReadiness | Stage::WaitingForStaleOtp { .. } => {
                unreachable!("no OTP session before it is prepared")
            }
        }
    }

    pub(in crate::connection::machine) fn otp_code_usable(&self, id: &AttemptId) -> bool {
        matches!(&self.phase, Phase::Connecting(attempt)
            if attempt.id == *id
                && attempt.stage == Stage::Run
                && attempt.otp_prompted_at.is_some()
                && attempt.otp_session != OtpSession::Submitted
                && attempt.stop_requested.is_none())
    }

    pub(in crate::connection::machine) fn spam_finding(&self, id: &AttemptId) -> SpamFinding {
        if let Phase::Connecting(attempt) = &self.phase
            && attempt.id == *id
        {
            return SpamFinding::Attempt;
        }
        if self.last_failure_attempt.as_ref() != Some(id) {
            return SpamFinding::Ignore;
        }
        let plain = Failure::OtpNotReceived { in_spam: false };
        SpamFinding::ShownFailure {
            shown_in: match &self.phase {
                Phase::Failed(failure) if *failure == plain => ShownIn::Failure,
                Phase::WaitingRetry { reason, .. } | Phase::WaitingNetwork { reason, .. }
                    if *reason == RetryReason::AuthFailure(plain) =>
                {
                    ShownIn::WaitReason
                }
                _ => ShownIn::Nowhere,
            },
            ledger: self.ledger.last_auth_failure == Some(plain),
        }
    }

    /// Readiness holds: the OTP session opens before the VPN client starts, once an earlier
    /// attempt's unused code email is in the trash or the wait for it is over.
    pub(in crate::connection::machine) fn otp_preparation(
        &self,
        now: SystemTime,
    ) -> OtpPreparation {
        let Phase::Connecting(_) = &self.phase else {
            unreachable!("only an attempt prepares its OTP session");
        };
        if self
            .stale_otp
            .as_ref()
            .is_some_and(|stale| now < stale.until)
        {
            OtpPreparation::WaitForStaleOtp { since: now }
        } else {
            OtpPreparation::Open
        }
    }

    /// The session of `id` is the one looking for an earlier attempt's unused code.
    pub(in crate::connection::machine) fn stale_otp_session(&self, id: &AttemptId) -> bool {
        self.stale_otp
            .as_ref()
            .is_some_and(|stale| stale.attempt == *id)
    }

    /// The login waiting for the stale code goes on, its time limit moved by the wait: the new
    /// limit, while a login waits.
    pub(in crate::connection::machine) fn stale_otp_wait_over(
        &self,
        now: SystemTime,
    ) -> Option<SystemTime> {
        let Phase::Connecting(attempt) = &self.phase else {
            return None;
        };
        let Stage::WaitingForStaleOtp { since } = attempt.stage else {
            return None;
        };
        Some(attempt.deadline + elapsed(since, now))
    }

    pub(in crate::connection::machine) fn otp_deadline(&self, now: SystemTime) -> OtpDeadline {
        let Phase::Connecting(attempt) = &self.phase else {
            return OtpDeadline::Ignore;
        };
        if attempt.stage != Stage::Run
            || attempt.otp_prompted_at.is_none()
            || attempt.otp_session == OtpSession::Submitted
            || attempt.stop_requested.is_some()
        {
            return OtpDeadline::Ignore;
        }
        // The spam check tells at once whether the code went there; no search for it.
        OtpDeadline::StopRun(attempt.otp_close(true, false, now))
    }
}
