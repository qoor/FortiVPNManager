//! Whether everything a connection needs is in place, and following the Helper's registration
//! and approval (SPEC F-CM-11).

use std::time::SystemTime;

use fvm_contract::app_helper::HelperStatus;
use fvm_contract::app_internal::port::{HelperChannelFailure, RegistrationError};

use super::machine::{AttemptReason, ConnectionManager, Phase, Stage};
use super::{Effect, policy};

/// What the runtime gathered for a readiness check. Secrets are only asked about, never read,
/// so the OS asks the user for nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReadinessFacts {
    /// The settings file is in use without an error.
    pub config_valid: bool,
    /// The server address, username and Gmail address are set, and the VPN client would accept
    /// the account values.
    pub account_complete: bool,
    /// The secret storage holds a VPN password for the account.
    pub vpn_password_stored: bool,
    /// The secret storage holds a Gmail app password, or with the Gmail API method the Google
    /// account is connected.
    pub gmail_credential_stored: bool,
    pub helper: HelperFacts,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HelperFacts {
    /// Not registered as a system service, or the system cannot find it.
    NotRegistered,
    /// Registered, waiting for the user's approval.
    RequiresApproval,
    /// Registered, but it did not answer the build check.
    Unreachable(HelperChannelFailure),
    /// It answered with `build`. With the app's build, `status` is its status if that answered.
    Answered {
        build: String,
        status: Option<HelperStatus>,
    },
}

/// Something to set up before connecting, in the order readiness checks it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SetupNeed {
    ConfigFile,
    AccountDetails,
    VpnPassword,
    GmailCredential,
    HelperApproval,
    /// The running Helper is another build, or it did not answer.
    HelperVersion,
}

/// Readiness checks every two seconds for a while after the Helper's registration or its
/// approval settings opened, so the approval shows without the user doing anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct FollowApproval {
    until: SystemTime,
    pub(super) next: SystemTime,
}

impl ReadinessFacts {
    fn missing(&self, app_build: &str) -> Vec<SetupNeed> {
        let checks = [
            (self.config_valid, SetupNeed::ConfigFile),
            (self.account_complete, SetupNeed::AccountDetails),
            (self.vpn_password_stored, SetupNeed::VpnPassword),
            (self.gmail_credential_stored, SetupNeed::GmailCredential),
        ];
        let mut missing: Vec<SetupNeed> = checks
            .into_iter()
            .filter(|(present, _)| !present)
            .map(|(_, need)| need)
            .collect();
        match &self.helper {
            HelperFacts::NotRegistered | HelperFacts::RequiresApproval => {
                missing.push(SetupNeed::HelperApproval);
            }
            HelperFacts::Unreachable(_) => missing.push(SetupNeed::HelperVersion),
            HelperFacts::Answered { build, .. } if build != app_build => {
                missing.push(SetupNeed::HelperVersion);
            }
            HelperFacts::Answered { .. } => {}
        }
        missing
    }
}

impl ConnectionManager {
    /// Asks once per input however many reasons came together.
    pub(super) fn check_readiness(&mut self) {
        if !self.effects.contains(&Effect::CheckReadiness) {
            self.effects.push(Effect::CheckReadiness);
        }
    }

    pub(super) fn readiness_checked(&mut self, facts: ReadinessFacts) {
        let missing = facts.missing(&self.app_build);
        self.helper = Some(facts.helper);
        let first = self.setup_needed.is_none();
        self.setup_needed = Some(missing.clone());
        // Settings open by themselves once per launch, for the first check only: setup that
        // becomes required later in the run opens nothing.
        if first && !self.settings_open_decided {
            self.settings_open_decided = true;
            if !missing.is_empty() {
                self.effects.push(Effect::OpenSettings);
            }
        }

        match &mut self.phase {
            Phase::Connecting(attempt) if attempt.stage == Stage::CheckingReadiness => {
                if missing.is_empty() {
                    let open = attempt.prepare_otp();
                    self.effects.push(open);
                } else {
                    self.phase = Phase::SetupRequired(missing);
                }
            }
            Phase::Idle
            | Phase::SetupRequired(_)
            | Phase::WaitingRetry { .. }
            | Phase::WaitingNetwork { .. } => {
                if !missing.is_empty() {
                    self.phase = Phase::SetupRequired(missing);
                } else if matches!(self.phase, Phase::SetupRequired(_)) {
                    if self.wants_connection {
                        self.begin_attempt(AttemptReason::SetupCompleted, false, true);
                    } else {
                        self.phase = Phase::Idle;
                    }
                }
            }
            // Applies to the next attempt; a failure is cleared only by the user.
            Phase::Connecting(_)
            | Phase::Connected(_)
            | Phase::Disconnecting(_)
            | Phase::Failed(_) => {}
        }
    }

    pub(super) fn helper_registered(&mut self, result: Result<(), RegistrationError>) {
        match result {
            Ok(()) => {
                self.register_error = None;
                self.follow_helper_approval();
            }
            Err(error) => self.register_error = Some(error),
        }
    }

    pub(super) fn follow_helper_approval(&mut self) {
        self.follow_approval = Some(FollowApproval {
            until: self.now + policy::APPROVAL_FOLLOW,
            next: self.now + policy::APPROVAL_FOLLOW_POLL,
        });
    }

    pub(super) fn approval_follow_due(&mut self) {
        self.check_readiness();
        self.follow_approval = self
            .follow_approval
            .filter(|follow| self.now < follow.until)
            .map(|follow| FollowApproval {
                next: self.now + policy::APPROVAL_FOLLOW_POLL,
                ..follow
            });
    }
}
