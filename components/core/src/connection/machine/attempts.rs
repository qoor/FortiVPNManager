use std::fmt::Write as _;
use std::time::{Duration, UNIX_EPOCH};

use fvm_contract::app_helper::{AttemptId, EndFacts, SessionId, StopReason};

use crate::connection::Effect;
use crate::connection::certs::merge_trusted_certs;
use crate::connection::classify::Outcome;
use crate::connection::ledger::Failure;
use crate::connection::policy::{self, elapsed};

use super::{
    AfterStop, Attempt, AttemptReason, ConnectionManager, Phase, Renewal, RetryReason,
    SECONDS_FROM_1970_TO_2026, Session, Stage, Stop,
};

impl ConnectionManager {
    /// Starts an attempt unless a gate holds it: the wish to connect, the schedule, the sleep,
    /// the attempt limit, the cooldown after an authentication failure, or a missing network path.
    pub(in crate::connection) fn begin_attempt(
        &mut self,
        reason: AttemptReason,
        cert_retry_used: bool,
        readiness_verified: bool,
    ) {
        if !self.wants_connection {
            self.phase = Phase::Idle;
            return;
        }
        if reason.is_automatic_reconnect()
            && let Some(away_limit) = self
                .settings
                .connection
                .schedule
                .as_ref()
                .filter(|schedule| !schedule.is_active(self.now, &self.time_zone))
                .map(crate::connection::Schedule::away_limit)
        {
            if self.reconnect_suspended || self.away_limit_reached() {
                self.give_up_reconnecting();
                return;
            }
            if let Some(away_since) = self.away_since {
                self.phase = Phase::WaitingRetry {
                    until: away_since + away_limit,
                    reason: RetryReason::UserAway,
                };
                return;
            }
        }
        if self.sleeping {
            self.wait_for_network(RetryReason::Sleep, Duration::ZERO);
            return;
        }
        self.ledger.prune(self.now);
        if !reason.is_user_action()
            && self.ledger.recent_auth_attempts.len() >= policy::MAX_AUTH_ATTEMPTS_PER_WINDOW
        {
            self.fail(Failure::TooManyReconnects);
            return;
        }
        if let Some(cooldown_end) = self.ledger.cooldown_end()
            && self.now < cooldown_end
        {
            let reason = match reason {
                AttemptReason::Retry(RetryReason::AuthFailure(failure)) => {
                    RetryReason::AuthFailure(failure)
                }
                _ => RetryReason::AuthCooldown,
            };
            self.phase = Phase::WaitingRetry {
                until: cooldown_end,
                reason,
            };
            return;
        }
        if self.path.as_ref().is_some_and(|path| !path.satisfied) {
            let reason = match reason {
                AttemptReason::Retry(reason) => reason,
                AttemptReason::AfterSleep => RetryReason::Sleep,
                _ => RetryReason::Network,
            };
            self.wait_for_network(reason, Duration::ZERO);
            return;
        }

        let id = self.next_attempt_id();
        let attempt = Attempt {
            id,
            session: self.next_session_id(),
            reason,
            cert_retry_used,
            deadline: self.now + policy::ATTEMPT_TIMEOUT,
            stage: Stage::CheckingReadiness,
            milestone: None,
            credentials_sent_at: None,
            otp_prompted_at: None,
            otp_submitted: false,
            otp_session_open: false,
            otp_failure: None,
            otp_in_spam: false,
            stop_requested: None,
            account: self.settings.account.clone(),
            replacing: None,
        };
        self.phase = Phase::Connecting(Box::new(attempt));
        if readiness_verified {
            self.prepare_otp();
        } else {
            self.check_readiness();
        }
    }

    /// The seed with the attempt count in its last eight bytes.
    pub(super) fn next_attempt_id(&mut self) -> AttemptId {
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

    pub(super) fn next_session_id(&mut self) -> SessionId {
        let since_2026 = elapsed(
            UNIX_EPOCH + Duration::from_secs(SECONDS_FROM_1970_TO_2026),
            self.now,
        );
        let from_clock = since_2026.as_secs() * 10;
        let session = match self.last_session {
            Some(SessionId(last)) if from_clock <= last => SessionId(last + 1),
            _ => SessionId(from_clock),
        };
        self.last_session = Some(session);
        session
    }

    pub(super) fn abort_attempt(
        &mut self,
        mut attempt: Attempt,
        stop: StopReason,
        after: AfterStop,
    ) {
        let search =
            after == AfterStop::Update || !matches!(stop, StopReason::AppQuit | StopReason::Sleep);
        let running = attempt.stage == Stage::Run;
        if running && attempt.stop_requested.is_none() {
            self.effects.push(Effect::StopRun {
                attempt: attempt.id.clone(),
                reason: stop,
            });
        }
        self.push_otp_close(attempt.close_otp_session(false, search));
        match attempt.replacing.take() {
            Some(session) => {
                self.stop_session(session, stop, after, running.then_some(attempt.id));
            }
            None if running => {
                self.phase = Phase::Disconnecting(Stop {
                    attempt: attempt.id,
                    after,
                    replacement: None,
                });
            }
            None => self.settle(after),
        }
    }

    /// Break before make: stop the tunnel; `after` follows its exit.
    pub(super) fn stop_session(
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

    pub(super) fn restart_session(&mut self, session: Session, reason: AttemptReason) {
        let stop = if reason == AttemptReason::Proactive {
            StopReason::ProactiveReconnect
        } else {
            StopReason::Reconnect
        };
        self.stop_session(session, stop, AfterStop::Reconnect(reason), None);
    }

    pub(super) fn renew(&mut self, mut session: Session, reason: AttemptReason) {
        session.renewal = Some(Renewal {
            reason,
            tries: 0,
            later_failures: 0,
            next_try: None,
        });
        self.prepare_replacement(session, false);
    }

    pub(super) fn prepare_replacement(&mut self, mut session: Session, cert_retry_used: bool) {
        let renewal = session
            .renewal
            .as_mut()
            .expect("a replacement is prepared for a renewal");
        let reason = renewal.reason;
        let session_end = session.since
            + Duration::from_secs(u64::from(self.settings.connection.session_max_minutes) * 60);
        let replaceable = self.path.as_ref().is_none_or(|path| path.satisfied)
            && elapsed(self.now, session_end) >= policy::ATTEMPT_TIMEOUT;
        if !replaceable {
            self.restart_session(session, reason);
            return;
        }
        self.ledger.prune(self.now);
        if renewal.tries < policy::MAX_COUNTED_PREPARATIONS
            && !reason.is_user_action()
            && self.ledger.recent_auth_attempts.len() >= policy::MAX_AUTH_ATTEMPTS_PER_WINDOW
        {
            self.keep_session(session);
            return;
        }
        if let Some(cooldown_end) = self.ledger.cooldown_end()
            && self.now < cooldown_end
        {
            renewal.next_try = Some(cooldown_end);
            self.phase = Phase::Connected(session);
            return;
        }
        renewal.next_try = None;
        let attempt = Attempt {
            id: self.next_attempt_id(),
            session: self.next_session_id(),
            reason,
            cert_retry_used,
            deadline: self.now + policy::ATTEMPT_TIMEOUT,
            stage: Stage::CheckingReadiness,
            milestone: None,
            credentials_sent_at: None,
            otp_prompted_at: None,
            otp_submitted: false,
            otp_session_open: false,
            otp_failure: None,
            otp_in_spam: false,
            stop_requested: None,
            account: self.settings.account.clone(),
            replacing: Some(session),
        };
        self.phase = Phase::Connecting(Box::new(attempt));
        self.check_readiness();
    }

    pub(super) fn replacement_ended(&mut self, mut session: Session, outcome: Outcome) {
        match outcome {
            Outcome::CertificateRetry => {
                let used = self.run_is_for_current_gateway();
                self.prepare_replacement(session, used);
            }
            Outcome::AuthFailure(
                failure @ (Failure::AuthenticationRejected | Failure::OtpRejected),
            ) => {
                self.ledger.record_auth_failure(failure, self.now);
                let renewal = session
                    .renewal
                    .as_mut()
                    .expect("a replacement is prepared for a renewal");
                renewal.tries = (renewal.tries + 1).min(policy::MAX_COUNTED_PREPARATIONS);
                if renewal.tries < policy::MAX_COUNTED_PREPARATIONS {
                    self.prepare_replacement(session, false);
                } else {
                    self.keep_session(session);
                }
            }
            _ => self.replacement_failed(session),
        }
    }

    pub(in crate::connection) fn replacement_failed(&mut self, mut session: Session) {
        let renewal = session
            .renewal
            .as_mut()
            .expect("a replacement is prepared for a renewal");
        if renewal.tries < policy::MAX_COUNTED_PREPARATIONS {
            renewal.tries += 1;
            if renewal.tries < policy::MAX_COUNTED_PREPARATIONS {
                self.prepare_replacement(session, false);
                return;
            }
        }
        renewal.later_failures += 1;
        renewal.next_try = Some(self.now + policy::network_backoff(renewal.later_failures));
        self.phase = Phase::Connected(session);
    }

    pub(super) fn keep_session(&mut self, mut session: Session) {
        if session
            .planned_reconnect
            .is_some_and(|planned| planned > self.now)
        {
            session.renewal = None;
        } else {
            let renewal = session
                .renewal
                .as_mut()
                .expect("a replacement is prepared for a renewal");
            renewal.next_try = None;
        }
        self.phase = Phase::Connected(session);
    }

    pub(super) fn settle(&mut self, after: AfterStop) {
        match after {
            AfterStop::Idle => self.phase = Phase::Idle,
            AfterStop::Reconnect(reason) => self.begin_attempt(reason, false, false),
            AfterStop::ResumeAfterSleep => {
                self.begin_attempt(AttemptReason::AfterSleep, false, false);
            }
            AfterStop::Quit => {
                self.phase = Phase::Idle;
                self.terminated = true;
            }
            AfterStop::Update => {
                self.phase = Phase::Idle;
                self.stopped_for_update();
            }
        }
    }

    /// The run that just ended talked to the gateway the settings name now.
    pub(super) fn run_is_for_current_gateway(&self) -> bool {
        self.run_account
            .as_ref()
            .is_none_or(|run| run.same_gateway(&self.settings.account))
    }

    pub(super) fn learn_certificates(&mut self, facts: &EndFacts) {
        // A run started before a server address or port change saw the old gateway's certificate.
        if facts.cert_rejected_digests.is_empty() || !self.run_is_for_current_gateway() {
            return;
        }
        let merged = merge_trusted_certs(
            &facts.cert_rejected_digests,
            &self.settings.account.trusted_certs,
        );
        if merged == self.settings.account.trusted_certs {
            return;
        }
        self.settings.account.trusted_certs.clone_from(&merged);
        self.effects
            .push(Effect::SaveTrustedCerts { certs: merged });
    }
}
