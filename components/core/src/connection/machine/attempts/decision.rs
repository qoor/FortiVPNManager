use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fvm_contract::app_helper::{AttemptId, EndFacts, SessionId, StopReason};

use crate::connection::certs::merge_trusted_certs;
use crate::connection::classify::Outcome;
use crate::connection::ledger::Failure;
use crate::connection::machine::otp::decision::OtpClose;
use crate::connection::machine::{
    AfterStop, Attempt, AttemptReason, ConnectionManager, Phase, RetryReason,
    SECONDS_FROM_1970_TO_2026, Session, Stage,
};
use crate::connection::policy::{self, elapsed};
use crate::connection::schedule::Schedule;

/// A new attempt, as decided.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) struct NewAttempt {
    pub(super) reason: AttemptReason,
    pub(super) cert_retry_used: bool,
    pub(super) deadline: SystemTime,
    pub(super) session: SessionId,
}

/// Whether an attempt starts, or which gate holds it: the wish to connect, the schedule, the
/// sleep, the attempt limit, the cooldown after an authentication failure, or a missing network
/// path. `kept` are the recorded attempts still within the window.
pub(in crate::connection::machine) enum AttemptStart {
    NotWanted,
    GiveUp,
    HoldForAway {
        until: SystemTime,
    },
    WaitForWake,
    TooMany {
        kept: Vec<SystemTime>,
    },
    Cooldown {
        kept: Vec<SystemTime>,
        until: SystemTime,
        reason: RetryReason,
    },
    WaitForNetwork {
        kept: Vec<SystemTime>,
        reason: RetryReason,
    },
    Begin {
        kept: Vec<SystemTime>,
        attempt: NewAttempt,
        readiness_verified: bool,
    },
}

/// The stop of a session to connect again.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) struct Restart {
    pub(super) stop: StopReason,
    pub(super) after: AfterStop,
}

/// What happens to the renewal of a session that is kept.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum KeepRenewal {
    /// The planned reconnect is still ahead: the renewal ends.
    End,
    /// The renewal waits for its next cause.
    Pause,
}

/// Whether a replacement for the session starts, or what holds it.
pub(in crate::connection::machine) enum ReplacementStart {
    /// The session ends too soon for a replacement, or the network is gone: break before make.
    Restart { session: Session, restart: Restart },
    /// The attempt limit holds the replacement.
    Keep {
        session: Session,
        kept: Vec<SystemTime>,
        keep: KeepRenewal,
    },
    Cooldown {
        session: Session,
        kept: Vec<SystemTime>,
        until: SystemTime,
    },
    Begin {
        session: Session,
        kept: Vec<SystemTime>,
        attempt: NewAttempt,
    },
}

/// How a replacement's run ended, for the session it was to replace.
pub(in crate::connection::machine) enum ReplacementEnd {
    CertificateRetry {
        session: Session,
        cert_retry_used: bool,
    },
    /// A counted authentication failure at `at`; `tries` are the counted preparations so far.
    AuthFailure {
        session: Session,
        failure: Failure,
        at: SystemTime,
        tries: u32,
        then: AfterAuthFailure,
    },
    Failed(Session),
}

pub(in crate::connection::machine) enum AfterAuthFailure {
    PrepareAgain,
    Keep(KeepRenewal),
}

/// What follows a replacement that failed.
pub(in crate::connection::machine) enum ReplacementFailure {
    TryAgain {
        session: Session,
        tries: u32,
    },
    /// Counted preparations are used up: the next try waits.
    Wait {
        session: Session,
        tries: u32,
        later_failures: u32,
        next_try: SystemTime,
    },
}

/// How an attempt under way is abandoned.
pub(in crate::connection::machine) struct AttemptAbort {
    pub(super) id: AttemptId,
    pub(super) stop: StopReason,
    pub(super) after: AfterStop,
    /// The run is asked to stop, unless a stop was asked for already.
    pub(super) stop_run: bool,
    pub(super) otp_close: Option<OtpClose>,
    pub(super) then: AbortThen,
}

pub(in crate::connection::machine) enum AbortThen {
    /// The session the attempt was to replace stops too; `replacement` is the attempt's run.
    StopReplaced {
        session: Session,
        replacement: Option<AttemptId>,
    },
    /// The attempt's run stops; `after` follows its exit.
    AwaitExit,
    /// Nothing runs: `after` follows at once.
    Settle,
}

/// What the attempt's time limit ends.
pub(in crate::connection::machine) enum AttemptDeadline {
    Ignore,
    /// The readiness check or the Gmail login hung; nothing runs yet.
    BeforeRun {
        otp_close: Option<OtpClose>,
        replacement: bool,
    },
    StopRun {
        otp_close: Option<OtpClose>,
    },
}

/// What a due retry timer starts.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum RetryDue {
    Ignore,
    Attempt(RetryReason),
    Replacement,
}

impl ConnectionManager {
    pub(in crate::connection::machine) fn attempt_start(
        &self,
        reason: AttemptReason,
        cert_retry_used: bool,
        readiness_verified: bool,
        now: SystemTime,
    ) -> AttemptStart {
        if !self.wants_connection {
            return AttemptStart::NotWanted;
        }
        if reason.is_automatic_reconnect()
            && let Some(away_limit) = self
                .settings
                .connection
                .schedule
                .as_ref()
                .filter(|schedule| !schedule.is_active(now, &self.time_zone))
                .map(Schedule::away_limit)
        {
            if self.reconnect_limits.reconnect_suspended || self.away_limit_reached(now) {
                return AttemptStart::GiveUp;
            }
            if let Some(away_since) = self.away_since {
                return AttemptStart::HoldForAway {
                    until: away_since + away_limit,
                };
            }
        }
        if self.system.sleeping {
            return AttemptStart::WaitForWake;
        }
        let kept = self.ledger.kept_attempts(now);
        if !reason.is_user_action() && kept.len() >= policy::MAX_AUTH_ATTEMPTS_PER_WINDOW {
            return AttemptStart::TooMany { kept };
        }
        if let Some(until) = self.ledger.cooldown_end()
            && now < until
        {
            let reason = match reason {
                AttemptReason::Retry(RetryReason::AuthFailure(failure)) => {
                    RetryReason::AuthFailure(failure)
                }
                _ => RetryReason::AuthCooldown,
            };
            return AttemptStart::Cooldown {
                kept,
                until,
                reason,
            };
        }
        if self.path.as_ref().is_some_and(|path| !path.satisfied) {
            let reason = match reason {
                AttemptReason::Retry(reason) => reason,
                AttemptReason::AfterSleep => RetryReason::Sleep,
                _ => RetryReason::Network,
            };
            return AttemptStart::WaitForNetwork { kept, reason };
        }
        AttemptStart::Begin {
            kept,
            attempt: self.new_attempt(reason, cert_retry_used, now),
            readiness_verified,
        }
    }

    fn new_attempt(
        &self,
        reason: AttemptReason,
        cert_retry_used: bool,
        now: SystemTime,
    ) -> NewAttempt {
        NewAttempt {
            reason,
            cert_retry_used,
            deadline: now + policy::ATTEMPT_TIMEOUT,
            session: self.next_session_id(now),
        }
    }

    /// Tenths of a second since 2026 from the clock, or one past the last one when the clock gives
    /// no later value.
    fn next_session_id(&self, now: SystemTime) -> SessionId {
        let since_2026 = elapsed(
            UNIX_EPOCH + Duration::from_secs(SECONDS_FROM_1970_TO_2026),
            now,
        );
        let from_clock = since_2026.as_secs() * 10;
        match self.last_session {
            Some(SessionId(last)) if from_clock <= last => SessionId(last + 1),
            _ => SessionId(from_clock),
        }
    }

    pub(in crate::connection::machine) fn replacement_start(
        &self,
        session: Session,
        cert_retry_used: bool,
        now: SystemTime,
    ) -> ReplacementStart {
        let renewal = session
            .renewal
            .expect("a replacement is prepared for a renewal");
        let reason = renewal.reason;
        let session_end = session.since
            + Duration::from_secs(u64::from(self.settings.connection.session_max_minutes) * 60);
        let replaceable = self.path.as_ref().is_none_or(|path| path.satisfied)
            && elapsed(now, session_end) >= policy::ATTEMPT_TIMEOUT;
        if !replaceable {
            return ReplacementStart::Restart {
                session,
                restart: restart_for(reason),
            };
        }
        let kept = self.ledger.kept_attempts(now);
        if renewal.tries < policy::MAX_COUNTED_PREPARATIONS
            && !reason.is_user_action()
            && kept.len() >= policy::MAX_AUTH_ATTEMPTS_PER_WINDOW
        {
            let keep = keep_renewal(&session, now);
            return ReplacementStart::Keep {
                session,
                kept,
                keep,
            };
        }
        if let Some(until) = self.ledger.cooldown_end()
            && now < until
        {
            return ReplacementStart::Cooldown {
                session,
                kept,
                until,
            };
        }
        ReplacementStart::Begin {
            session,
            kept,
            attempt: self.new_attempt(reason, cert_retry_used, now),
        }
    }

    pub(in crate::connection::machine) fn replacement_end(
        &self,
        session: Session,
        outcome: Outcome,
        now: SystemTime,
    ) -> ReplacementEnd {
        match outcome {
            Outcome::CertificateRetry => ReplacementEnd::CertificateRetry {
                session,
                cert_retry_used: self.run_is_for_current_gateway(),
            },
            Outcome::AuthFailure(
                failure @ (Failure::AuthenticationRejected | Failure::OtpRejected),
            ) => {
                let renewal = session
                    .renewal
                    .expect("a replacement is prepared for a renewal");
                let tries = (renewal.tries + 1).min(policy::MAX_COUNTED_PREPARATIONS);
                let then = if tries < policy::MAX_COUNTED_PREPARATIONS {
                    AfterAuthFailure::PrepareAgain
                } else {
                    AfterAuthFailure::Keep(keep_renewal(&session, now))
                };
                ReplacementEnd::AuthFailure {
                    session,
                    failure,
                    at: now,
                    tries,
                    then,
                }
            }
            _ => ReplacementEnd::Failed(session),
        }
    }
}

pub(in crate::connection::machine) fn replacement_failure(
    session: Session,
    now: SystemTime,
) -> ReplacementFailure {
    let renewal = session
        .renewal
        .expect("a replacement is prepared for a renewal");
    let counted = renewal.tries < policy::MAX_COUNTED_PREPARATIONS;
    let tries = if counted {
        renewal.tries + 1
    } else {
        renewal.tries
    };
    if counted && tries < policy::MAX_COUNTED_PREPARATIONS {
        return ReplacementFailure::TryAgain { session, tries };
    }
    let later_failures = renewal.later_failures + 1;
    ReplacementFailure::Wait {
        session,
        tries,
        later_failures,
        next_try: now + policy::network_backoff(later_failures),
    }
}

pub(in crate::connection::machine) fn attempt_abort(
    mut attempt: Attempt,
    stop: StopReason,
    after: AfterStop,
    now: SystemTime,
) -> AttemptAbort {
    let search =
        after == AfterStop::Update || !matches!(stop, StopReason::AppQuit | StopReason::Sleep);
    let running = attempt.stage == Stage::Run;
    let otp_close = attempt.otp_close(false, search, now);
    let then = match attempt.replacing.take() {
        Some(session) => AbortThen::StopReplaced {
            session,
            replacement: running.then(|| attempt.id.clone()),
        },
        None if running => AbortThen::AwaitExit,
        None => AbortThen::Settle,
    };
    AttemptAbort {
        stop_run: running && attempt.stop_requested.is_none(),
        id: attempt.id,
        stop,
        after,
        otp_close,
        then,
    }
}

impl ConnectionManager {
    pub(in crate::connection::machine) fn attempt_deadline(
        &self,
        now: SystemTime,
    ) -> AttemptDeadline {
        let Phase::Connecting(attempt) = &self.phase else {
            return AttemptDeadline::Ignore;
        };
        if attempt.stop_requested.is_some() {
            return AttemptDeadline::Ignore;
        }
        let otp_close = attempt.otp_close(false, true, now);
        if attempt.stage == Stage::Run {
            AttemptDeadline::StopRun { otp_close }
        } else {
            AttemptDeadline::BeforeRun {
                otp_close,
                replacement: attempt.replacing.is_some(),
            }
        }
    }

    pub(in crate::connection::machine) fn retry_due(&self) -> RetryDue {
        match &self.phase {
            Phase::WaitingRetry { reason, .. } => RetryDue::Attempt(*reason),
            Phase::Connected(_) => RetryDue::Replacement,
            _ => RetryDue::Ignore,
        }
    }

    pub(in crate::connection::machine) fn proactive_reconnect_due(&self, now: SystemTime) -> bool {
        matches!(self.phase, Phase::Connected(_)) && self.automatic_reconnect_allowed(now)
    }

    /// The run that just ended talked to the gateway the settings name now.
    pub(in crate::connection) fn run_is_for_current_gateway(&self) -> bool {
        self.run_account
            .as_ref()
            .is_none_or(|run| run.same_gateway(&self.settings.account))
    }

    /// The certificate values to trust once a run ended, when they change.
    pub(in crate::connection::machine) fn learned_certificates(
        &self,
        facts: &EndFacts,
    ) -> Option<Vec<String>> {
        // A run started before a server address or port change saw the old gateway's certificate.
        if facts.cert_rejected_digests.is_empty() || !self.run_is_for_current_gateway() {
            return None;
        }
        let merged = merge_trusted_certs(
            &facts.cert_rejected_digests,
            &self.settings.account.trusted_certs,
        );
        (merged != self.settings.account.trusted_certs).then_some(merged)
    }
}

pub(in crate::connection::machine) fn restart_for(reason: AttemptReason) -> Restart {
    Restart {
        stop: if reason == AttemptReason::Proactive {
            StopReason::ProactiveReconnect
        } else {
            StopReason::Reconnect
        },
        after: AfterStop::Reconnect(reason),
    }
}

fn keep_renewal(session: &Session, now: SystemTime) -> KeepRenewal {
    if session
        .planned_reconnect
        .is_some_and(|planned| planned > now)
    {
        KeepRenewal::End
    } else {
        KeepRenewal::Pause
    }
}
