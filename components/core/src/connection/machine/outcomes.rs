pub(super) mod action;
pub(super) mod decision;

use crate::connection::classify::Outcome;

use super::{ConnectionManager, RetryReason, Step};

impl ConnectionManager {
    pub(super) fn apply(&mut self, outcome: Outcome) {
        self.run_now(Step::ApplyOutcome(outcome));
    }

    pub(super) fn forced_termination_now(&mut self) {
        self.run_now(Step::ForcedTermination);
    }

    pub(super) fn network_failure_now(&mut self, reason: RetryReason) {
        self.run_now(Step::NetworkFailure(reason));
    }
}
