use std::fmt::Write as _;
use std::time::Duration;

use fvm_contract::app_helper::{AttemptId, StopReason};

use crate::connection::Effect;
use crate::connection::ledger::Failure;
use crate::connection::machine::{
    AfterStop, Attempt, AttemptReason, ConnectionManager, OtpSession, Phase, Quit, Renewal,
    RetryReason, Session, Stage, Step, Stop,
};

use super::decision::{
    AbortThen, AfterAuthFailure, AttemptAbort, AttemptDeadline, AttemptStart, KeepRenewal,
    NewAttempt, ReplacementEnd, ReplacementFailure, ReplacementStart, Restart, RetryDue,
};

impl ConnectionManager {
    pub(in crate::connection::machine) fn start_attempt(
        &mut self,
        start: AttemptStart,
    ) -> Vec<Step> {
        match start {
            AttemptStart::NotWanted => self.phase = Phase::Idle,
            AttemptStart::GiveUp => self.give_up_reconnecting(),
            AttemptStart::HoldForAway { until } => {
                self.phase = Phase::WaitingRetry {
                    until,
                    reason: RetryReason::UserAway,
                };
            }
            AttemptStart::WaitForWake => self.wait_for_network(RetryReason::Sleep, Duration::ZERO),
            AttemptStart::TooMany { kept } => {
                self.ledger.recent_auth_attempts = kept;
                self.fail(Failure::TooManyReconnects);
            }
            AttemptStart::Cooldown {
                kept,
                until,
                reason,
            } => {
                self.ledger.recent_auth_attempts = kept;
                self.phase = Phase::WaitingRetry { until, reason };
            }
            AttemptStart::WaitForNetwork { kept, reason } => {
                self.ledger.recent_auth_attempts = kept;
                self.wait_for_network(reason, Duration::ZERO);
            }
            AttemptStart::Begin {
                kept,
                attempt,
                readiness_verified,
            } => {
                self.ledger.recent_auth_attempts = kept;
                let attempt = self.issue_attempt(attempt, None);
                self.phase = Phase::Connecting(Box::new(attempt));
                return vec![if readiness_verified {
                    Step::PrepareOtp
                } else {
                    Step::CheckReadiness
                }];
            }
        }
        Vec::new()
    }

    fn issue_attempt(&mut self, attempt: NewAttempt, replacing: Option<Session>) -> Attempt {
        self.last_session = Some(attempt.session);
        Attempt {
            id: self.next_attempt_id(),
            session: attempt.session,
            reason: attempt.reason,
            cert_retry_used: attempt.cert_retry_used,
            deadline: attempt.deadline,
            stage: Stage::CheckingReadiness,
            milestone: None,
            credentials_sent_at: None,
            otp_prompted_at: None,
            otp_session: OtpSession::Closed,
            otp_failure: None,
            otp_in_spam: false,
            stop_requested: None,
            account: self.settings.account.clone(),
            replacing,
        }
    }

    /// The seed with the attempt count in its last eight bytes.
    fn next_attempt_id(&mut self) -> AttemptId {
        self.attempts_issued += 1;
        let mut bytes = self.attempt_seed;
        for (byte, count) in bytes[8..]
            .iter_mut()
            .zip(self.attempts_issued.to_be_bytes())
        {
            *byte ^= count;
        }
        let hex: String = bytes.iter().fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02X}");
            hex
        });
        format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
        .parse()
        .expect("a UUID")
    }

    pub(in crate::connection::machine) fn start_replacement(
        &mut self,
        start: ReplacementStart,
    ) -> Vec<Step> {
        match start {
            ReplacementStart::Restart { session, restart } => self.restart(session, restart),
            ReplacementStart::Keep {
                session,
                kept,
                keep,
            } => {
                self.ledger.recent_auth_attempts = kept;
                self.keep_session(session, keep);
            }
            ReplacementStart::Cooldown {
                mut session,
                kept,
                until,
            } => {
                self.ledger.recent_auth_attempts = kept;
                session
                    .renewal
                    .as_mut()
                    .expect("a replacement is prepared for a renewal")
                    .next_try = Some(until);
                self.phase = Phase::Connected(session);
            }
            ReplacementStart::Begin {
                mut session,
                kept,
                attempt,
            } => {
                self.ledger.recent_auth_attempts = kept;
                session
                    .renewal
                    .as_mut()
                    .expect("a replacement is prepared for a renewal")
                    .next_try = None;
                let attempt = self.issue_attempt(attempt, Some(session));
                self.phase = Phase::Connecting(Box::new(attempt));
                return vec![Step::CheckReadiness];
            }
        }
        Vec::new()
    }

    pub(in crate::connection::machine) fn end_replacement(
        &mut self,
        end: ReplacementEnd,
    ) -> Vec<Step> {
        match end {
            ReplacementEnd::CertificateRetry {
                session,
                cert_retry_used,
            } => vec![Step::PrepareReplacement {
                session,
                cert_retry_used,
            }],
            ReplacementEnd::AuthFailure {
                mut session,
                failure,
                at,
                tries,
                then,
            } => {
                self.ledger.record_auth_failure(failure, at);
                session
                    .renewal
                    .as_mut()
                    .expect("a replacement is prepared for a renewal")
                    .tries = tries;
                match then {
                    AfterAuthFailure::PrepareAgain => vec![Step::PrepareReplacement {
                        session,
                        cert_retry_used: false,
                    }],
                    AfterAuthFailure::Keep(keep) => {
                        self.keep_session(session, keep);
                        Vec::new()
                    }
                }
            }
            ReplacementEnd::Failed(session) => vec![Step::ReplacementFailed(session)],
        }
    }

    pub(in crate::connection::machine) fn fail_replacement(
        &mut self,
        failure: ReplacementFailure,
    ) -> Vec<Step> {
        match failure {
            ReplacementFailure::TryAgain { mut session, tries } => {
                session
                    .renewal
                    .as_mut()
                    .expect("a replacement is prepared for a renewal")
                    .tries = tries;
                vec![Step::PrepareReplacement {
                    session,
                    cert_retry_used: false,
                }]
            }
            ReplacementFailure::Wait {
                mut session,
                tries,
                later_failures,
                next_try,
            } => {
                let renewal = session
                    .renewal
                    .as_mut()
                    .expect("a replacement is prepared for a renewal");
                renewal.tries = tries;
                renewal.later_failures = later_failures;
                renewal.next_try = Some(next_try);
                self.phase = Phase::Connected(session);
                Vec::new()
            }
        }
    }

    pub(in crate::connection::machine) fn keep_session(
        &mut self,
        mut session: Session,
        keep: KeepRenewal,
    ) {
        match keep {
            KeepRenewal::End => session.renewal = None,
            KeepRenewal::Pause => {
                session
                    .renewal
                    .as_mut()
                    .expect("a replacement is prepared for a renewal")
                    .next_try = None;
            }
        }
        self.phase = Phase::Connected(session);
    }

    pub(in crate::connection::machine) fn abort(&mut self, abort: AttemptAbort) -> Vec<Step> {
        if abort.stop_run {
            self.effects.push(Effect::StopRun {
                attempt: abort.id.clone(),
                reason: abort.stop,
            });
        }
        self.push_otp_close(abort.otp_close);
        match abort.then {
            AbortThen::StopReplaced {
                session,
                replacement,
            } => {
                self.stop_session(session, abort.stop, abort.after, replacement);
                Vec::new()
            }
            AbortThen::AwaitExit => {
                self.phase = Phase::Disconnecting(Stop {
                    attempt: abort.id,
                    after: abort.after,
                    replacement: None,
                });
                Vec::new()
            }
            AbortThen::Settle => self.settle(abort.after),
        }
    }

    /// Break before make: stop the tunnel; `after` follows its exit.
    pub(in crate::connection::machine) fn stop_session(
        &mut self,
        session: Session,
        reason: StopReason,
        after: AfterStop,
        replacement: Option<AttemptId>,
    ) {
        self.effects.push(Effect::StopRun {
            attempt: session.attempt.clone(),
            reason,
        });
        self.phase = Phase::Disconnecting(Stop {
            attempt: session.attempt,
            after,
            replacement,
        });
    }

    pub(in crate::connection::machine) fn restart(&mut self, session: Session, restart: Restart) {
        self.stop_session(session, restart.stop, restart.after, None);
    }

    pub(in crate::connection::machine) fn settle(&mut self, after: AfterStop) -> Vec<Step> {
        match after {
            AfterStop::Idle => self.phase = Phase::Idle,
            AfterStop::Reconnect(reason) => {
                return vec![Step::BeginAttempt {
                    reason,
                    cert_retry_used: false,
                    readiness_verified: false,
                }];
            }
            AfterStop::ResumeAfterSleep => {
                return vec![Step::BeginAttempt {
                    reason: AttemptReason::AfterSleep,
                    cert_retry_used: false,
                    readiness_verified: false,
                }];
            }
            AfterStop::Quit => {
                self.phase = Phase::Idle;
                self.quit = Quit::Terminated;
            }
            AfterStop::Update => {
                self.phase = Phase::Idle;
                return vec![Step::StoppedForUpdate];
            }
        }
        Vec::new()
    }

    pub(in crate::connection::machine) fn end_attempt_at_deadline(
        &mut self,
        deadline: AttemptDeadline,
    ) -> Vec<Step> {
        match deadline {
            AttemptDeadline::Ignore => Vec::new(),
            AttemptDeadline::BeforeRun {
                otp_close,
                replacement,
            } => {
                let Phase::Connecting(attempt) = &mut self.phase else {
                    unreachable!("decided for an attempt under way");
                };
                let close = attempt.otp_closed(otp_close);
                let replacing = replacement.then(|| {
                    attempt
                        .replacing
                        .take()
                        .expect("decided with a session to replace")
                });
                self.push_otp_close(close);
                vec![match replacing {
                    Some(session) => Step::ReplacementFailed(session),
                    None => Step::NetworkFailure(RetryReason::Network),
                }]
            }
            AttemptDeadline::StopRun { otp_close } => {
                let Phase::Connecting(attempt) = &mut self.phase else {
                    unreachable!("decided for an attempt under way");
                };
                attempt.stop_requested = Some(StopReason::AttemptTimeout);
                let stop = Effect::StopRun {
                    attempt: attempt.id.clone(),
                    reason: StopReason::AttemptTimeout,
                };
                let close = attempt.otp_closed(otp_close);
                self.effects.push(stop);
                self.push_otp_close(close);
                Vec::new()
            }
        }
    }

    pub(in crate::connection::machine) fn start_due_retry(&mut self, due: RetryDue) -> Vec<Step> {
        match due {
            RetryDue::Ignore => Vec::new(),
            RetryDue::Attempt(reason) => {
                self.phase = Phase::Idle;
                vec![Step::BeginAttempt {
                    reason: AttemptReason::Retry(reason),
                    cert_retry_used: false,
                    readiness_verified: false,
                }]
            }
            RetryDue::Replacement => {
                let Phase::Connected(session) = self.take_phase() else {
                    unreachable!("the retry was decided for a session");
                };
                vec![Step::PrepareReplacement {
                    session,
                    cert_retry_used: false,
                }]
            }
        }
    }

    pub(in crate::connection::machine) fn renew_when_due(&mut self, due: bool) -> Vec<Step> {
        if !due {
            return Vec::new();
        }
        let Phase::Connected(session) = self.take_phase() else {
            unreachable!("the proactive reconnect was decided for a session");
        };
        renew(session, AttemptReason::Proactive)
    }

    pub(in crate::connection::machine) fn learn_certificates(
        &mut self,
        learned: Option<Vec<String>>,
    ) {
        let Some(merged) = learned else {
            return;
        };
        self.settings.account.trusted_certs.clone_from(&merged);
        self.effects
            .push(Effect::SaveTrustedCerts { certs: merged });
    }
}

/// Starts a renewal of the session: its replacement is prepared next.
pub(in crate::connection::machine) fn renew(
    mut session: Session,
    reason: AttemptReason,
) -> Vec<Step> {
    session.renewal = Some(Renewal {
        reason,
        tries: 0,
        later_failures: 0,
        next_try: None,
    });
    vec![Step::PrepareReplacement {
        session,
        cert_retry_used: false,
    }]
}
