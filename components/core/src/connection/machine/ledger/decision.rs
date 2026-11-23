use std::time::SystemTime;

use crate::connection::ledger::FailureLedger;
use crate::connection::policy;

impl FailureLedger {
    /// No attempt may start before it.
    pub(in crate::connection) fn cooldown_end(&self) -> Option<SystemTime> {
        self.last_auth_failure_at
            .map(|at| at + policy::AUTH_FAILURE_RETRY_DELAY)
    }

    /// The recorded attempts still within the window at `now`, oldest first.
    pub(in crate::connection) fn kept_attempts(&self, now: SystemTime) -> Vec<SystemTime> {
        match now.checked_sub(policy::AUTH_ATTEMPT_WINDOW) {
            Some(start) => self
                .recent_auth_attempts
                .iter()
                .copied()
                .filter(|at| *at > start)
                .collect(),
            None => self.recent_auth_attempts.clone(),
        }
    }

    /// The recorded attempts once an attempt reached the gateway at `at`.
    pub(in crate::connection) fn attempts_with(&self, at: SystemTime) -> Vec<SystemTime> {
        let mut attempts = self.kept_attempts(at);
        attempts.push(at);
        attempts
    }
}
