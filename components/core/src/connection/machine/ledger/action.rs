use std::time::SystemTime;

use crate::connection::ledger::{Failure, FailureLedger};

impl FailureLedger {
    pub(in crate::connection) fn record_auth_failure(&mut self, failure: Failure, at: SystemTime) {
        self.consecutive_auth_failures += 1;
        self.last_auth_failure_at = Some(at);
        self.last_auth_failure = Some(failure);
    }

    /// A tunnel up proves the credentials work.
    pub(in crate::connection) fn reset_consecutive_failures(&mut self) {
        self.consecutive_auth_failures = 0;
    }

    /// The user's Connect or Reconnect Now starts the count and the window over. The time of the
    /// last failure stays, so its cooldown still holds.
    pub(in crate::connection) fn reset_for_user_action(&mut self) {
        self.consecutive_auth_failures = 0;
        self.recent_auth_attempts.clear();
    }
}
