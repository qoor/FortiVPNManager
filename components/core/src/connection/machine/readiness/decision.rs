use std::time::SystemTime;

use fvm_contract::app_internal::port::RegistrationError;

use crate::connection::machine::{ConnectionManager, Phase, Stage};
use crate::connection::readiness::{FollowApproval, HelperFacts, ReadinessFacts, SetupNeed};
use crate::connection::{Effect, policy};

/// What a readiness check found, and what it leads to.
pub(in crate::connection::machine) struct ReadinessChecked {
    pub(super) facts: ReadinessFacts,
    pub(super) missing: Vec<SetupNeed>,
    /// Settings open by themselves once per launch, for the first check only: setup that becomes
    /// required later in the run opens nothing. `Some` at the first check, with whether they
    /// open.
    pub(super) first: Option<bool>,
    pub(super) then: AfterReadiness,
}

pub(in crate::connection::machine) enum AfterReadiness {
    Nothing,
    PrepareOtp,
    ReplacementFailed,
    SetupRequired,
    BeginAttempt,
    Idle,
}

/// The Helper's registration and the readiness checks that follow its approval.
pub(in crate::connection::machine) enum Registration {
    Registered(FollowApproval),
    Failed(RegistrationError),
}

impl ReadinessFacts {
    fn missing(&self, app_build: &str) -> Vec<SetupNeed> {
        let checks = [
            (self.config_valid, SetupNeed::ConfigFile),
            (self.account_complete, SetupNeed::AccountDetails),
            (self.secrets.vpn_password, SetupNeed::VpnPassword),
            (self.secrets.gmail_credential, SetupNeed::GmailCredential),
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
    pub(in crate::connection::machine) fn readiness_check_asked(&self) -> bool {
        self.effects.contains(&Effect::CheckReadiness)
    }

    pub(in crate::connection::machine) fn readiness_checked(
        &self,
        facts: ReadinessFacts,
    ) -> ReadinessChecked {
        let missing = facts.missing(&self.app_build);
        let then = match &self.phase {
            Phase::Connecting(attempt) if attempt.stage == Stage::CheckingReadiness => {
                if missing.is_empty() {
                    AfterReadiness::PrepareOtp
                } else if attempt.replacing.is_some() {
                    AfterReadiness::ReplacementFailed
                } else {
                    AfterReadiness::SetupRequired
                }
            }
            Phase::Idle
            | Phase::SetupRequired(_)
            | Phase::WaitingRetry { .. }
            | Phase::WaitingNetwork { .. } => {
                if !missing.is_empty() {
                    AfterReadiness::SetupRequired
                } else if !matches!(self.phase, Phase::SetupRequired(_)) {
                    AfterReadiness::Nothing
                } else if self.wants_connection {
                    AfterReadiness::BeginAttempt
                } else {
                    AfterReadiness::Idle
                }
            }
            // Applies to the next attempt; a failure is cleared only by the user.
            Phase::Connecting(_)
            | Phase::Connected(_)
            | Phase::Disconnecting(_)
            | Phase::Failed(_) => AfterReadiness::Nothing,
        };
        let first = (self.readiness.setup_needed.is_none()
            && !self.readiness.settings_open_decided)
            .then_some(!missing.is_empty());
        ReadinessChecked {
            facts,
            missing,
            first,
            then,
        }
    }

    /// The next readiness check while following the approval, until the time is up.
    pub(in crate::connection::machine) fn approval_follow(
        &self,
        now: SystemTime,
    ) -> Option<FollowApproval> {
        self.readiness
            .follow_approval
            .filter(|follow| now < follow.until)
            .map(|follow| FollowApproval {
                next: now + policy::APPROVAL_FOLLOW_POLL,
                ..follow
            })
    }
}

pub(in crate::connection::machine) fn registration(
    result: Result<(), RegistrationError>,
    now: SystemTime,
) -> Registration {
    match result {
        Ok(()) => Registration::Registered(follow_approval(now)),
        Err(error) => Registration::Failed(error),
    }
}

/// Readiness is checked every two seconds for a while after the Helper's registration or its
/// approval settings opened, so the approval shows without the user doing anything.
pub(in crate::connection::machine) fn follow_approval(now: SystemTime) -> FollowApproval {
    FollowApproval {
        until: now + policy::APPROVAL_FOLLOW,
        next: now + policy::APPROVAL_FOLLOW_POLL,
    }
}
