use fvm_contract::app_helper::{AttemptId, StopReason};
use fvm_contract::app_internal::SecretText;

use crate::connection::ledger::Failure;
use crate::connection::policy::{self, elapsed};
use crate::connection::{Effect, Note, OtpFailure};

use super::{ConnectionManager, OtpSession, Phase, RetryReason, Stage, StaleOtp};

impl ConnectionManager {
    pub(super) fn otp_session_ready(&mut self, id: &AttemptId) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.id != *id || attempt.stage != Stage::PreparingOtp {
            return;
        }
        attempt.stage = Stage::Run;
        self.run_account = Some(attempt.account.clone());
        self.effects.push(Effect::StartRun {
            attempt: attempt.id.clone(),
            session: attempt.session,
            account: attempt.account.clone(),
            replacing: attempt
                .replacing
                .as_ref()
                .map(|session| session.attempt.clone()),
        });
    }

    pub(super) fn otp_session_failed(&mut self, id: &AttemptId, failure: OtpFailure) {
        if self
            .stale_otp
            .as_ref()
            .is_some_and(|stale| stale.attempt == *id)
        {
            // Nothing looks for the code email any more.
            self.stale_otp = None;
            self.stale_otp_wait_over(Note::StaleOtpSessionEnded);
            return;
        }
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.id != *id || attempt.otp_session != OtpSession::Open {
            return;
        }
        attempt.otp_session = OtpSession::Closed;
        match attempt.stage {
            // The VPN client was never started: a Gmail credential problem costs no VPN login.
            Stage::PreparingOtp => match (attempt.replacing.take(), failure) {
                (Some(session), _) => self.replacement_failed(session),
                (None, OtpFailure::Authentication) => self.fail(Failure::GmailRejected),
                (None, OtpFailure::Network) => self.network_failure(RetryReason::Network),
            },
            // No OTP can arrive any more: stop now instead of at the deadline; the exit classifies.
            Stage::Run => {
                attempt.otp_failure = Some(failure);
                if attempt.stop_requested.is_none() {
                    attempt.stop_requested = Some(StopReason::OtpUnavailable);
                    self.effects.push(Effect::StopRun {
                        attempt: attempt.id.clone(),
                        reason: StopReason::OtpUnavailable,
                    });
                }
            }
            Stage::CheckingReadiness | Stage::WaitingForStaleOtp { .. } => {
                unreachable!("no OTP session before it is prepared")
            }
        }
    }

    pub(super) fn otp_code_found(&mut self, id: &AttemptId, code: SecretText) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.id != *id
            || attempt.stage != Stage::Run
            || attempt.otp_prompted_at.is_none()
            || attempt.otp_session == OtpSession::Submitted
            || attempt.stop_requested.is_some()
        {
            return;
        }
        // The session moves the used emails to the trash and closes itself after the submission.
        attempt.otp_session = OtpSession::Submitted;
        self.effects.push(Effect::SubmitOtp {
            attempt: attempt.id.clone(),
            code,
        });
    }

    pub(super) fn otp_found_in_spam(&mut self, id: &AttemptId) {
        if let Phase::Connecting(attempt) = &mut self.phase
            && attempt.id == *id
        {
            attempt.otp_in_spam = true;
            return;
        }
        if self.last_failure_attempt.as_ref() != Some(id) {
            return;
        }
        let plain = Failure::OtpNotReceived { in_spam: false };
        let spam = Failure::OtpNotReceived { in_spam: true };
        match &mut self.phase {
            Phase::Failed(failure) if *failure == plain => *failure = spam,
            Phase::WaitingRetry { reason, .. } | Phase::WaitingNetwork { reason, .. }
                if *reason == RetryReason::AuthFailure(plain) =>
            {
                *reason = RetryReason::AuthFailure(spam);
            }
            _ => {}
        }
        if self.ledger.last_auth_failure == Some(plain) {
            self.ledger.last_auth_failure = Some(spam);
        }
    }

    /// Readiness holds: the OTP session opens before the VPN client starts, once an earlier
    /// attempt's unused code email is in the trash or the wait for it is over.
    pub(in crate::connection) fn prepare_otp(&mut self) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            unreachable!("only an attempt prepares its OTP session");
        };
        if self
            .stale_otp
            .as_ref()
            .is_some_and(|stale| self.now < stale.until)
        {
            attempt.stage = Stage::WaitingForStaleOtp { since: self.now };
            self.effects.push(Effect::Note {
                attempt: Some(attempt.id.clone()),
                note: Note::WaitingForStaleOtp,
            });
            return;
        }
        attempt.stage = Stage::PreparingOtp;
        attempt.otp_session = OtpSession::Open;
        self.effects.push(Effect::OpenOtpSession {
            attempt: attempt.id.clone(),
        });
    }

    pub(super) fn stale_otp_trashed(&mut self, id: &AttemptId) {
        if self
            .stale_otp
            .as_ref()
            .is_some_and(|stale| stale.attempt == *id)
        {
            self.stale_otp = None;
            self.stale_otp_wait_over(Note::StaleOtpTrashed);
        }
    }

    pub(super) fn stale_otp_wait_ran_out(&mut self) {
        let Some(stale) = self.stale_otp.take() else {
            return;
        };
        self.effects.push(Effect::CloseOtpSession {
            attempt: stale.attempt,
            check_spam: false,
        });
        self.stale_otp_wait_over(Note::StaleOtpWaitRanOut);
    }

    /// The login waiting for the stale code goes on, its time limit moved by the wait.
    pub(super) fn stale_otp_wait_over(&mut self, note: Note) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        let Stage::WaitingForStaleOtp { since } = attempt.stage else {
            return;
        };
        attempt.deadline += elapsed(since, self.now);
        self.effects.push(Effect::Note {
            attempt: Some(attempt.id.clone()),
            note,
        });
        self.prepare_otp();
    }

    /// Hands over the close of an ended attempt's OTP session. A search for its unused code holds
    /// the next login, which [`Self::prepare_otp`] decides, until it ends or its time limit.
    pub(super) fn push_otp_close(&mut self, close: Option<Effect>) {
        if let Some(Effect::TrashStaleOtp { attempt, .. }) = &close {
            self.stale_otp = Some(StaleOtp {
                attempt: attempt.clone(),
                until: self.now + policy::STALE_OTP_WAIT,
            });
        }
        self.effects.extend(close);
    }
}
