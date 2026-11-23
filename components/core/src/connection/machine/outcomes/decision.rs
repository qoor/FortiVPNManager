use std::time::{Duration, SystemTime};

use crate::connection::classify::Outcome;
use crate::connection::ledger::Failure;
use crate::connection::machine::{AttemptReason, ConnectionManager, RetryReason, Step};
use crate::connection::policy;

/// Where the connection waits before the next attempt.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum RetryWait {
    /// Nothing is wanted any more.
    Idle,
    WaitForWake,
    /// The path is gone; `delay` follows once it is back.
    WaitForNetwork {
        reason: RetryReason,
        delay: Duration,
    },
    Wait {
        until: SystemTime,
        reason: RetryReason,
    },
}

/// A network failure, counted into the failures in a row, and the wait after it.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) struct NetworkFailure {
    pub(super) streak: u32,
    pub(super) wait: RetryWait,
}

/// A counted authentication failure at `at`, and what follows it.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) struct AuthFailure {
    pub(super) failure: Failure,
    pub(super) at: SystemTime,
    /// `None` once the failures in a row stop the retries.
    pub(super) retry: Option<RetryWait>,
}

impl ConnectionManager {
    /// What follows from how a run ended.
    pub(in crate::connection::machine) fn outcome_steps(&self, outcome: Outcome) -> Vec<Step> {
        vec![match outcome {
            // The chain's one immediate retry belongs to a gateway; a new one starts its own.
            Outcome::CertificateRetry => Step::BeginAttempt {
                reason: AttemptReason::CertificateRetry,
                cert_retry_used: self.run_is_for_current_gateway(),
                readiness_verified: false,
            },
            Outcome::ServerSessionEnded => Step::ScheduleRetry {
                reason: RetryReason::ServerSessionEnded,
                delay: policy::SERVER_SESSION_END_DELAY,
            },
            Outcome::NetworkError => Step::NetworkFailure(RetryReason::Network),
            Outcome::NetworkSetupFailed => Step::NetworkFailure(RetryReason::NetworkSetupFailed),
            Outcome::AuthFailure(failure) => Step::AuthFailure(failure),
            Outcome::Fatal(failure) => Step::Fail(failure),
        }]
    }

    /// Reconnects at once through the gates of any retry, but once only until a tunnel comes up
    /// again: the Helper ends itself on internal faults, and one that did so on every run would
    /// otherwise repeat VPN logins and OTP emails without end.
    pub(in crate::connection::machine) fn forced_retry_left(&self) -> bool {
        !self.reconnect_limits.forced_retry_used
    }

    pub(in crate::connection::machine) fn network_failure(
        &self,
        reason: RetryReason,
        now: SystemTime,
    ) -> NetworkFailure {
        let streak = self.reconnect_limits.network_failure_streak + 1;
        NetworkFailure {
            streak,
            wait: self.retry_wait(reason, policy::network_backoff(streak), now),
        }
    }

    pub(in crate::connection::machine) fn auth_failure(
        &self,
        failure: Failure,
        now: SystemTime,
    ) -> AuthFailure {
        let stops =
            self.ledger.consecutive_auth_failures + 1 >= policy::MAX_CONSECUTIVE_AUTH_FAILURES;
        AuthFailure {
            failure,
            at: now,
            retry: (!stops).then(|| {
                self.retry_wait(
                    RetryReason::AuthFailure(failure),
                    policy::AUTH_FAILURE_RETRY_DELAY,
                    now,
                )
            }),
        }
    }

    pub(in crate::connection::machine) fn retry_wait(
        &self,
        reason: RetryReason,
        delay: Duration,
        now: SystemTime,
    ) -> RetryWait {
        if !self.wants_connection {
            RetryWait::Idle
        } else if self.system.sleeping {
            RetryWait::WaitForWake
        } else if self.path.as_ref().is_some_and(|path| !path.satisfied) {
            // An authentication retry is bound by the ledger's cooldown instead of a new wait.
            let delay = if matches!(reason, RetryReason::AuthFailure(_)) {
                Duration::ZERO
            } else {
                delay
            };
            RetryWait::WaitForNetwork { reason, delay }
        } else {
            RetryWait::Wait {
                until: now + delay,
                reason,
            }
        }
    }
}
