use std::time::SystemTime;

use fvm_contract::app_helper::StopReason;
use fvm_contract::app_internal::SecretText;

use crate::connection::ledger::Failure;
use crate::connection::machine::{
    Attempt, ConnectionManager, OtpSession, Phase, RetryReason, Stage, Step,
};
use crate::connection::{Effect, Note};

use super::decision::{
    AfterOtpFailure, OtpClose, OtpDeadline, OtpPreparation, OtpSessionFailure, ShownIn, SpamFinding,
};

impl Attempt {
    /// Marks the session closed as decided and hands the close on.
    pub(in crate::connection::machine) fn otp_closed(
        &mut self,
        close: Option<OtpClose>,
    ) -> Option<OtpClose> {
        if close.is_some() {
            self.otp_session = OtpSession::Closed;
        }
        close
    }
}

impl ConnectionManager {
    /// Hands over the close of an ended attempt's OTP session. A search for its unused code holds
    /// the next login, which the OTP preparation decides, until it ends or its time limit.
    pub(in crate::connection::machine) fn push_otp_close(&mut self, close: Option<OtpClose>) {
        let Some(close) = close else {
            return;
        };
        let (effect, stale) = close.into_parts();
        if stale.is_some() {
            self.stale_otp = stale;
        }
        self.effects.push(effect);
    }

    pub(in crate::connection::machine) fn start_run(&mut self, ready: bool) -> Vec<Step> {
        if !ready {
            return Vec::new();
        }
        let Phase::Connecting(attempt) = &mut self.phase else {
            unreachable!("decided for an attempt under way");
        };
        attempt.stage = Stage::Run;
        let account = attempt.account.clone();
        let start = Effect::StartRun {
            attempt: attempt.id.clone(),
            session: attempt.session,
            account: attempt.account.clone(),
            replacing: attempt
                .replacing
                .as_ref()
                .map(|session| session.attempt.clone()),
        };
        self.run_account = Some(account);
        self.effects.push(start);
        Vec::new()
    }

    pub(in crate::connection::machine) fn end_otp_session(
        &mut self,
        failure: OtpSessionFailure,
    ) -> Vec<Step> {
        match failure {
            OtpSessionFailure::Ignore => Vec::new(),
            OtpSessionFailure::StaleSearchEnded => {
                // Nothing looks for the code email any more.
                self.stale_otp = None;
                vec![Step::StaleOtpWaitOver(Note::StaleOtpSessionEnded)]
            }
            OtpSessionFailure::BeforeRun(then) => {
                let Phase::Connecting(attempt) = &mut self.phase else {
                    unreachable!("decided for an attempt under way");
                };
                attempt.otp_session = OtpSession::Closed;
                match then {
                    AfterOtpFailure::ReplacementFailed => {
                        let session = attempt.replacing.take().expect("decided with a session");
                        vec![Step::ReplacementFailed(session)]
                    }
                    AfterOtpFailure::Fail(failure) => {
                        self.fail(failure);
                        Vec::new()
                    }
                    AfterOtpFailure::NetworkFailure => {
                        vec![Step::NetworkFailure(RetryReason::Network)]
                    }
                }
            }
            OtpSessionFailure::DuringRun { failure, stop } => {
                let Phase::Connecting(attempt) = &mut self.phase else {
                    unreachable!("decided for an attempt under way");
                };
                attempt.otp_session = OtpSession::Closed;
                attempt.otp_failure = Some(failure);
                if stop {
                    attempt.stop_requested = Some(StopReason::OtpUnavailable);
                    let stop = Effect::StopRun {
                        attempt: attempt.id.clone(),
                        reason: StopReason::OtpUnavailable,
                    };
                    self.effects.push(stop);
                }
                Vec::new()
            }
        }
    }

    pub(in crate::connection::machine) fn submit_otp(
        &mut self,
        usable: bool,
        code: SecretText,
    ) -> Vec<Step> {
        if !usable {
            return Vec::new();
        }
        let Phase::Connecting(attempt) = &mut self.phase else {
            unreachable!("decided for an attempt under way");
        };
        // The session moves the used emails to the trash and closes itself after the submission.
        attempt.otp_session = OtpSession::Submitted;
        let submit = Effect::SubmitOtp {
            attempt: attempt.id.clone(),
            code,
        };
        self.effects.push(submit);
        Vec::new()
    }

    pub(in crate::connection::machine) fn note_spam(&mut self, finding: SpamFinding) -> Vec<Step> {
        let spam = Failure::OtpNotReceived { in_spam: true };
        match finding {
            SpamFinding::Ignore => {}
            SpamFinding::Attempt => {
                let Phase::Connecting(attempt) = &mut self.phase else {
                    unreachable!("decided for an attempt under way");
                };
                attempt.otp_in_spam = true;
            }
            SpamFinding::ShownFailure { shown_in, ledger } => {
                match (shown_in, &mut self.phase) {
                    (ShownIn::Nowhere, _) => {}
                    (ShownIn::Failure, phase) => *phase = Phase::Failed(spam),
                    (
                        ShownIn::WaitReason,
                        Phase::WaitingRetry { reason, .. } | Phase::WaitingNetwork { reason, .. },
                    ) => *reason = RetryReason::AuthFailure(spam),
                    (ShownIn::WaitReason, _) => unreachable!("decided for a wait"),
                }
                if ledger {
                    self.ledger.last_auth_failure = Some(spam);
                }
            }
        }
        Vec::new()
    }

    pub(in crate::connection::machine) fn prepare_otp(
        &mut self,
        preparation: OtpPreparation,
    ) -> Vec<Step> {
        let Phase::Connecting(attempt) = &mut self.phase else {
            unreachable!("decided for an attempt under way");
        };
        let effect = match preparation {
            OtpPreparation::WaitForStaleOtp { since } => {
                attempt.stage = Stage::WaitingForStaleOtp { since };
                Effect::Note {
                    attempt: Some(attempt.id.clone()),
                    note: Note::WaitingForStaleOtp,
                }
            }
            OtpPreparation::Open => {
                attempt.stage = Stage::PreparingOtp;
                attempt.otp_session = OtpSession::Open;
                Effect::OpenOtpSession {
                    attempt: attempt.id.clone(),
                }
            }
        };
        self.effects.push(effect);
        Vec::new()
    }

    pub(in crate::connection::machine) fn end_stale_otp_wait(
        &mut self,
        trashed: bool,
    ) -> Vec<Step> {
        if !trashed {
            return Vec::new();
        }
        self.stale_otp = None;
        vec![Step::StaleOtpWaitOver(Note::StaleOtpTrashed)]
    }

    pub(in crate::connection::machine) fn stop_stale_otp_search(&mut self) -> Vec<Step> {
        let Some(stale) = self.stale_otp.take() else {
            return Vec::new();
        };
        self.effects.push(Effect::CloseOtpSession {
            attempt: stale.attempt,
            check_spam: false,
        });
        vec![Step::StaleOtpWaitOver(Note::StaleOtpWaitRanOut)]
    }

    pub(in crate::connection::machine) fn go_on_after_stale_otp(
        &mut self,
        deadline: Option<SystemTime>,
        note: Note,
    ) -> Vec<Step> {
        let Some(deadline) = deadline else {
            return Vec::new();
        };
        let Phase::Connecting(attempt) = &mut self.phase else {
            unreachable!("decided for an attempt under way");
        };
        attempt.deadline = deadline;
        let note = Effect::Note {
            attempt: Some(attempt.id.clone()),
            note,
        };
        self.effects.push(note);
        vec![Step::PrepareOtp]
    }

    pub(in crate::connection::machine) fn stop_at_otp_deadline(
        &mut self,
        deadline: OtpDeadline,
    ) -> Vec<Step> {
        let OtpDeadline::StopRun(otp_close) = deadline else {
            return Vec::new();
        };
        let Phase::Connecting(attempt) = &mut self.phase else {
            unreachable!("decided for an attempt under way");
        };
        attempt.stop_requested = Some(StopReason::OtpTimeout);
        let stop = Effect::StopRun {
            attempt: attempt.id.clone(),
            reason: StopReason::OtpTimeout,
        };
        let close = attempt.otp_closed(otp_close);
        self.effects.push(stop);
        self.push_otp_close(close);
        Vec::new()
    }
}
