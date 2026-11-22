use std::net::Ipv4Addr;

use fvm_contract::app_helper::{
    AttemptId, EndFacts, HelperErrorCode, HelperEvent, HelperEventKind, Milestone,
};

use crate::connection::classify::{ExitContext, Outcome, classify};
use crate::connection::ledger::Failure;
use crate::connection::policy;
use crate::connection::{Effect, OtpFailure};

use super::{
    AfterStop, Attempt, ConnectionManager, OtpSession, Phase, Quit, RetryReason, Session, Stage,
};

impl ConnectionManager {
    pub(super) fn helper_event(&mut self, event: HelperEvent) {
        match event.kind {
            HelperEventKind::Milestone {
                milestone,
                tunnel_ip,
            } => self.milestone_reached(&event.attempt_id, milestone, tunnel_ip),
            HelperEventKind::Exited { facts } => self.exited(&event.attempt_id, &facts),
        }
    }

    pub(super) fn milestone_reached(
        &mut self,
        id: &AttemptId,
        milestone: Milestone,
        tunnel_ip: Option<Ipv4Addr>,
    ) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.stage != Stage::Run || attempt.id != *id || attempt.stop_requested.is_some() {
            return;
        }
        let previous = attempt.milestone;
        if previous.is_some_and(|previous| milestone <= previous) {
            return;
        }
        attempt.milestone = Some(milestone);
        if previous.unwrap_or(Milestone::Started) < Milestone::ConnectedToGateway
            && milestone >= Milestone::ConnectedToGateway
        {
            // The certificate check passed and the credentials go out next: this counts toward
            // the attempt limit.
            let counted = attempt
                .replacing
                .as_ref()
                .and_then(|session| session.renewal)
                .is_none_or(|renewal| renewal.tries < policy::MAX_COUNTED_PREPARATIONS);
            if counted {
                self.ledger.record_auth_attempt(self.now);
            }
            attempt.credentials_sent_at = Some(self.now);
        }
        match milestone {
            Milestone::TunnelUp => {
                let Phase::Connecting(attempt) = self.take_phase() else {
                    unreachable!("matched above");
                };
                self.enter_connected(*attempt, tunnel_ip);
            }
            Milestone::OtpPrompted => {
                attempt.otp_prompted_at = Some(self.now);
                self.effects.push(Effect::SearchOtp {
                    attempt: attempt.id.clone(),
                    prompted_at: self.now,
                });
            }
            Milestone::Started
            | Milestone::ConnectedToGateway
            | Milestone::OtpSubmitted
            | Milestone::Authenticated => {}
        }
    }

    pub(super) fn enter_connected(&mut self, mut attempt: Attempt, tunnel_ip: Option<Ipv4Addr>) {
        // Still open only when the gateway asked for no OTP.
        self.push_otp_close(attempt.close_otp_session(false, true));
        self.reconnect_limits.network_failure_streak = 0;
        self.ledger.reset_consecutive_failures();
        self.reconnect_limits.forced_retry_used = false;
        self.phase = Phase::Connected(Session {
            attempt: attempt.id,
            since: self.now,
            tunnel_ip,
            planned_reconnect: policy::planned_reconnect(
                self.now,
                &self.settings.connection,
                self.now,
            ),
            underlay: self.path.as_ref().and_then(|path| path.underlay.clone()),
            renewal: None,
            replaced: attempt.replacing.map(|replaced| replaced.attempt),
        });
    }

    pub(super) fn exited(&mut self, id: &AttemptId, facts: &EndFacts) {
        match self.take_phase() {
            Phase::Connecting(mut attempt) if attempt.stage == Stage::Run && attempt.id == *id => {
                self.learn_certificates(facts);
                let outcome = classify(&ExitContext {
                    facts,
                    seen_milestone: attempt.milestone,
                    cert_retry_used: attempt.cert_retry_used,
                    app_stop: attempt.stop_requested,
                    otp_failure: attempt.otp_failure,
                    otp_in_spam: attempt.otp_in_spam,
                });
                // No code email follows a login the gateway rejected, and its counted failure keeps
                // the next login 90 s away, past the prompt's arrival tolerance.
                let search = outcome != Outcome::AuthFailure(Failure::AuthenticationRejected);
                self.push_otp_close(attempt.close_otp_session(false, search));
                match attempt.replacing.take() {
                    Some(session) if facts.tunnel_address_in_use => {
                        self.restart_session(session, attempt.reason);
                    }
                    Some(session) => self.replacement_ended(session, outcome),
                    None => {
                        self.last_failure_attempt = Some(id.clone());
                        self.apply(outcome);
                    }
                }
            }
            Phase::Connecting(mut attempt)
                if attempt
                    .replacing
                    .as_ref()
                    .is_some_and(|session| session.attempt == *id) =>
            {
                attempt.replacing = None;
                self.phase = Phase::Connecting(attempt);
            }
            Phase::Connected(mut session) if session.replaced.as_ref() == Some(id) => {
                session.replaced = None;
                self.phase = Phase::Connected(session);
            }
            Phase::Connected(session) if session.attempt == *id => {
                self.learn_certificates(facts);
                self.last_failure_attempt = Some(id.clone());
                let outcome = classify(&ExitContext {
                    facts,
                    seen_milestone: Some(Milestone::TunnelUp),
                    cert_retry_used: true,
                    app_stop: None,
                    otp_failure: None,
                    otp_in_spam: false,
                });
                self.apply(outcome);
            }
            Phase::Disconnecting(mut stop)
                if stop.attempt == *id || stop.replacement.as_ref() == Some(id) =>
            {
                self.learn_certificates(facts);
                match stop.replacement.take() {
                    Some(replacement) => {
                        if replacement != *id {
                            stop.attempt = replacement;
                        }
                        self.phase = Phase::Disconnecting(stop);
                    }
                    None => self.settle(stop.after),
                }
            }
            // An attempt already finished with.
            phase => self.phase = phase,
        }
    }

    pub(super) fn run_rejected(&mut self, id: &AttemptId, code: HelperErrorCode) {
        match self.take_phase() {
            Phase::Connecting(mut attempt) if attempt.stage == Stage::Run && attempt.id == *id => {
                self.push_otp_close(attempt.close_otp_session(false, true));
                if let Some(session) = attempt.replacing.take() {
                    self.replacement_failed(session);
                    return;
                }
                match (attempt.otp_failure, code) {
                    (Some(OtpFailure::Authentication), _) => self.fail(Failure::GmailRejected),
                    (None, HelperErrorCode::InvalidRequest | HelperErrorCode::NotAuthorized) => {
                        self.fail(Failure::Helper { code });
                    }
                    (Some(OtpFailure::Network), _)
                    | (
                        None,
                        HelperErrorCode::Busy
                        | HelperErrorCode::WrongPhase
                        | HelperErrorCode::UnknownAttempt
                        | HelperErrorCode::InternalError,
                    ) => self.network_failure(RetryReason::Network),
                }
            }
            Phase::Disconnecting(mut stop)
                if stop.attempt == *id || stop.replacement.as_ref() == Some(id) =>
            {
                match stop.replacement.take() {
                    Some(replacement) => {
                        if replacement != *id {
                            stop.attempt = replacement;
                        }
                        self.phase = Phase::Disconnecting(stop);
                    }
                    None => self.settle(stop.after),
                }
            }
            phase => self.phase = phase,
        }
    }

    /// The connection stays inside the Mac: it breaks only when the Helper process ends, killed
    /// or crashed, never because of the network.
    pub(super) fn helper_connection_lost(&mut self) {
        match self.take_phase() {
            Phase::Connecting(mut attempt) if attempt.replacing.is_some() => {
                attempt.replacing = None;
                if attempt.stage == Stage::Run {
                    self.push_otp_close(attempt.close_otp_session(false, true));
                    self.forced_termination();
                } else {
                    self.phase = Phase::Connecting(attempt);
                }
            }
            Phase::Connecting(mut attempt) if attempt.stage == Stage::Run => {
                self.push_otp_close(attempt.close_otp_session(false, true));
                if attempt.otp_failure == Some(OtpFailure::Authentication) {
                    self.fail(Failure::GmailRejected);
                } else {
                    self.forced_termination();
                }
            }
            Phase::Connected(_) => self.forced_termination(),
            Phase::Disconnecting(stop) => self.settle(stop.after),
            phase => self.phase = phase,
        }
    }

    /// Whether the run of attempt `id` is still to be started: no stop was decided since it was
    /// asked for. A stop decided before the run's Connect left would reach the Helper first and
    /// find nothing to stop, so such a run is dropped instead.
    #[must_use]
    pub fn wants_run(&self, id: &AttemptId) -> bool {
        matches!(&self.phase, Phase::Connecting(attempt)
            if attempt.id == *id && attempt.stage == Stage::Run && attempt.stop_requested.is_none())
    }

    /// The attempt the connection is about now: the one connecting, connected or being stopped.
    /// Each attempt begun has a new one.
    #[must_use]
    pub fn current_attempt(&self) -> Option<&AttemptId> {
        match &self.phase {
            Phase::Connecting(attempt) => Some(&attempt.id),
            Phase::Connected(session) => Some(&session.attempt),
            Phase::Disconnecting(stop) => Some(&stop.attempt),
            _ => None,
        }
    }

    /// Whether the app wants the VPN up: connecting, connected, stopping to connect again, or
    /// waiting to try again. Not while it waits for an away user outside the scheduled hours, nor
    /// once it stopped for good: disconnected, paused after failures, or quitting. The exception
    /// routes are in place exactly then (user decision, 2026-11-16).
    #[must_use]
    pub fn wants_vpn(&self) -> bool {
        if self.quit == Quit::Terminated {
            return false;
        }
        match &self.phase {
            Phase::Connecting(_) | Phase::Connected(_) | Phase::WaitingNetwork { .. } => true,
            Phase::WaitingRetry { reason, .. } => *reason != RetryReason::UserAway,
            Phase::Disconnecting(stop) => matches!(
                stop.after,
                AfterStop::Reconnect(_) | AfterStop::ResumeAfterSleep
            ),
            Phase::Idle | Phase::SetupRequired(_) | Phase::Failed(_) => false,
        }
    }

    /// Whether attempt `id`'s OTP session is still to be opened: it was not closed while the
    /// session's secrets were read.
    #[must_use]
    pub fn wants_otp_session(&self, id: &AttemptId) -> bool {
        matches!(&self.phase, Phase::Connecting(attempt)
            if attempt.id == *id && attempt.otp_session == OtpSession::Open)
    }

    /// Whether a readiness check that found the Helper at `helper_build` should have it exit when
    /// idle, so the system starts this build's Helper: only another build, and only while no run
    /// is going on.
    #[must_use]
    pub fn should_restart_helper(&self, helper_build: &str) -> bool {
        helper_build != self.app_build && !self.run_active()
    }

    /// A run's events are taken now.
    pub(in crate::connection) fn run_active(&self) -> bool {
        match &self.phase {
            Phase::Connecting(attempt) => {
                attempt.stage == Stage::Run || attempt.replacing.is_some()
            }
            Phase::Connected(_) | Phase::Disconnecting(_) => true,
            _ => false,
        }
    }
}
