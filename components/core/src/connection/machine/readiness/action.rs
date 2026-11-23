use crate::connection::Effect;
use crate::connection::machine::{AttemptReason, ConnectionManager, Phase, Step};
use crate::connection::readiness::FollowApproval;

use super::decision::{AfterReadiness, ReadinessChecked, Registration};

impl ConnectionManager {
    pub(in crate::connection::machine) fn ask_readiness(&mut self, asked: bool) -> Vec<Step> {
        if !asked {
            self.effects.push(Effect::CheckReadiness);
        }
        Vec::new()
    }

    pub(in crate::connection::machine) fn take_readiness(
        &mut self,
        checked: ReadinessChecked,
    ) -> Vec<Step> {
        self.readiness.helper = Some(checked.facts.helper);
        self.readiness.setup_needed = Some(checked.missing.clone());
        if let Some(open_settings) = checked.first {
            self.readiness.settings_open_decided = true;
            if open_settings {
                self.effects.push(Effect::OpenSettings);
            }
        }
        match checked.then {
            AfterReadiness::Nothing => Vec::new(),
            AfterReadiness::PrepareOtp => vec![Step::PrepareOtp],
            AfterReadiness::ReplacementFailed => {
                let Phase::Connecting(attempt) = &mut self.phase else {
                    unreachable!("decided for an attempt under way");
                };
                let session = attempt
                    .replacing
                    .take()
                    .expect("decided with a session to replace");
                vec![Step::ReplacementFailed(session)]
            }
            AfterReadiness::SetupRequired => {
                self.phase = Phase::SetupRequired(checked.missing);
                Vec::new()
            }
            AfterReadiness::BeginAttempt => vec![Step::BeginAttempt {
                reason: AttemptReason::SetupCompleted,
                cert_retry_used: false,
                readiness_verified: true,
            }],
            AfterReadiness::Idle => {
                self.phase = Phase::Idle;
                Vec::new()
            }
        }
    }

    pub(in crate::connection::machine) fn register(
        &mut self,
        registration: Registration,
    ) -> Vec<Step> {
        match registration {
            Registration::Registered(follow) => {
                self.readiness.register_error = None;
                self.readiness.follow_approval = Some(follow);
            }
            Registration::Failed(error) => self.readiness.register_error = Some(error),
        }
        Vec::new()
    }

    pub(in crate::connection::machine) fn follow_approval(&mut self, follow: FollowApproval) {
        self.readiness.follow_approval = Some(follow);
    }

    pub(in crate::connection::machine) fn approval_follow_due(
        &mut self,
        follow: Option<FollowApproval>,
    ) -> Vec<Step> {
        self.readiness.follow_approval = follow;
        vec![Step::CheckReadiness]
    }
}
