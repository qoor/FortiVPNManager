pub(super) mod action;
pub(super) mod decision;

use fvm_contract::app_helper::{EndFacts, StopReason};

use crate::connection::classify::Outcome;

use super::{AfterStop, Attempt, AttemptReason, ConnectionManager, Session, Step};

impl ConnectionManager {
    pub(in crate::connection) fn begin_attempt(
        &mut self,
        reason: AttemptReason,
        cert_retry_used: bool,
        readiness_verified: bool,
    ) {
        self.run_now(Step::BeginAttempt {
            reason,
            cert_retry_used,
            readiness_verified,
        });
    }

    pub(super) fn replacement_ended(&mut self, session: Session, outcome: Outcome) {
        self.run_now(Step::ReplacementEnded { session, outcome });
    }

    pub(in crate::connection) fn replacement_failed(&mut self, session: Session) {
        self.run_now(Step::ReplacementFailed(session));
    }

    pub(super) fn abort_attempt(&mut self, attempt: Attempt, stop: StopReason, after: AfterStop) {
        self.run_now(Step::AbortAttempt {
            attempt: Box::new(attempt),
            stop,
            after,
        });
    }

    pub(super) fn restart_session(&mut self, session: Session, reason: AttemptReason) {
        self.restart(session, decision::restart_for(reason));
    }

    pub(super) fn renew_now(&mut self, session: Session, reason: AttemptReason) {
        let steps = action::renew(session, reason);
        self.run_all(steps);
    }

    pub(super) fn settle_now(&mut self, after: AfterStop) {
        let steps = self.settle(after);
        self.run_all(steps);
    }

    pub(super) fn learn_now(&mut self, facts: &EndFacts) {
        let learned = self.learned_certificates(facts);
        self.learn_certificates(learned);
    }
}
