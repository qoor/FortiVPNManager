use std::time::Duration;

use crate::connection::ledger::Failure;
use crate::connection::machine::{AttemptReason, ConnectionManager, Phase, RetryReason, Step};

use super::decision::{AuthFailure, NetworkFailure, RetryWait};

impl ConnectionManager {
    pub(in crate::connection::machine) fn forced_termination(
        &mut self,
        retry_left: bool,
    ) -> Vec<Step> {
        if !retry_left {
            return vec![Step::NetworkFailure(RetryReason::Network)];
        }
        self.reconnect_limits.forced_retry_used = true;
        vec![Step::BeginAttempt {
            reason: AttemptReason::Retry(RetryReason::ForcedTermination),
            cert_retry_used: false,
            readiness_verified: false,
        }]
    }

    pub(in crate::connection::machine) fn count_network_failure(
        &mut self,
        failure: NetworkFailure,
    ) -> Vec<Step> {
        self.reconnect_limits.network_failure_streak = failure.streak;
        self.wait_to_retry(failure.wait);
        Vec::new()
    }

    pub(in crate::connection::machine) fn count_auth_failure(
        &mut self,
        failure: AuthFailure,
    ) -> Vec<Step> {
        self.ledger.record_auth_failure(failure.failure, failure.at);
        match failure.retry {
            Some(wait) => self.wait_to_retry(wait),
            None => self.fail(failure.failure),
        }
        Vec::new()
    }

    pub(in crate::connection::machine) fn wait_to_retry(&mut self, wait: RetryWait) {
        match wait {
            RetryWait::Idle => self.phase = Phase::Idle,
            RetryWait::WaitForWake => self.wait_for_network(RetryReason::Sleep, Duration::ZERO),
            RetryWait::WaitForNetwork { reason, delay } => self.wait_for_network(reason, delay),
            RetryWait::Wait { until, reason } => self.phase = Phase::WaitingRetry { until, reason },
        }
    }

    pub(in crate::connection::machine) fn wait_for_network(
        &mut self,
        reason: RetryReason,
        delay_after_path: Duration,
    ) {
        self.phase = Phase::WaitingNetwork {
            reason,
            delay_after_path,
        };
    }

    pub(in crate::connection::machine) fn fail(&mut self, failure: Failure) {
        self.phase = Phase::Failed(failure);
    }
}
