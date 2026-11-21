use std::time::Duration;

use crate::connection::classify::Outcome;
use crate::connection::ledger::Failure;
use crate::connection::policy;

use super::{AttemptReason, ConnectionManager, Phase, RetryReason};

impl ConnectionManager {
    pub(super) fn apply(&mut self, outcome: Outcome) {
        match outcome {
            // The chain's one immediate retry belongs to a gateway; a new one starts its own.
            Outcome::CertificateRetry => {
                let used = self.run_is_for_current_gateway();
                self.begin_attempt(AttemptReason::CertificateRetry, used, false);
            }
            Outcome::ServerSessionEnded => self.schedule_retry(
                RetryReason::ServerSessionEnded,
                policy::SERVER_SESSION_END_DELAY,
            ),
            Outcome::NetworkError => self.network_failure(RetryReason::Network),
            Outcome::NetworkSetupFailed => self.network_failure(RetryReason::NetworkSetupFailed),
            Outcome::AuthFailure(failure) => self.auth_failure(failure),
            Outcome::Fatal(failure) => self.fail(failure),
        }
    }

    /// Reconnects at once through the gates of any retry, but once only until a tunnel comes up
    /// again: the Helper ends itself on internal faults, and one that did so on every run would
    /// otherwise repeat VPN logins and OTP emails without end.
    pub(super) fn forced_termination(&mut self) {
        if self.forced_retry_used {
            self.network_failure(RetryReason::Network);
            return;
        }
        self.forced_retry_used = true;
        self.begin_attempt(
            AttemptReason::Retry(RetryReason::ForcedTermination),
            false,
            false,
        );
    }

    pub(super) fn network_failure(&mut self, reason: RetryReason) {
        self.network_failure_streak += 1;
        self.schedule_retry(reason, policy::network_backoff(self.network_failure_streak));
    }

    pub(super) fn auth_failure(&mut self, failure: Failure) {
        self.ledger.record_auth_failure(failure, self.now);
        if self.ledger.consecutive_auth_failures >= policy::MAX_CONSECUTIVE_AUTH_FAILURES {
            self.fail(failure);
        } else {
            self.schedule_retry(
                RetryReason::AuthFailure(failure),
                policy::AUTH_FAILURE_RETRY_DELAY,
            );
        }
    }

    pub(super) fn schedule_retry(&mut self, reason: RetryReason, delay: Duration) {
        if !self.wants_connection {
            self.phase = Phase::Idle;
        } else if self.sleeping {
            self.wait_for_network(RetryReason::Sleep, Duration::ZERO);
        } else if self.path.as_ref().is_some_and(|path| !path.satisfied) {
            // An authentication retry is bound by the ledger's cooldown instead of a new wait.
            let delay = if matches!(reason, RetryReason::AuthFailure(_)) {
                Duration::ZERO
            } else {
                delay
            };
            self.wait_for_network(reason, delay);
        } else {
            self.phase = Phase::WaitingRetry {
                until: self.now + delay,
                reason,
            };
        }
    }

    pub(super) fn wait_for_network(&mut self, reason: RetryReason, delay_after_path: Duration) {
        self.phase = Phase::WaitingNetwork {
            reason,
            delay_after_path,
        };
    }

    pub(super) fn fail(&mut self, failure: Failure) {
        self.phase = Phase::Failed(failure);
    }
}
