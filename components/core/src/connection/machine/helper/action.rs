use crate::connection::Effect;
use crate::connection::machine::{ConnectionManager, Phase, RetryReason, Session, Step};

use super::decision::{
    AfterAttemptExit, AfterLoss, AfterMilestone, AfterRejection, AttemptExit, Connected,
    ConnectionLoss, MilestoneReached, RunExit, RunRejection, StopEnded,
};

impl ConnectionManager {
    pub(in crate::connection::machine) fn reach_milestone(
        &mut self,
        reached: MilestoneReached,
    ) -> Vec<Step> {
        let MilestoneReached::Reached {
            milestone,
            counted,
            credentials_sent_at,
            then,
        } = reached
        else {
            return Vec::new();
        };
        if let Some(attempts) = counted {
            self.ledger.recent_auth_attempts = attempts;
        }
        let Phase::Connecting(attempt) = &mut self.phase else {
            unreachable!("decided for an attempt under way");
        };
        attempt.milestone = Some(milestone);
        if credentials_sent_at.is_some() {
            attempt.credentials_sent_at = credentials_sent_at;
        }
        match then {
            AfterMilestone::Nothing => Vec::new(),
            AfterMilestone::SearchOtp { prompted_at } => {
                attempt.otp_prompted_at = Some(prompted_at);
                let search = Effect::SearchOtp {
                    attempt: attempt.id.clone(),
                    prompted_at,
                };
                self.effects.push(search);
                Vec::new()
            }
            AfterMilestone::EnterConnected { tunnel_ip } => {
                vec![Step::EnterConnected { tunnel_ip }]
            }
        }
    }

    pub(in crate::connection::machine) fn enter_connected(
        &mut self,
        connected: Connected,
    ) -> Vec<Step> {
        let Phase::Connecting(mut attempt) = self.take_phase() else {
            unreachable!("a tunnel comes up for an attempt");
        };
        let close = attempt.otp_closed(connected.otp_close);
        self.push_otp_close(close);
        self.reconnect_limits.network_failure_streak = 0;
        self.ledger.reset_consecutive_failures();
        self.reconnect_limits.forced_retry_used = false;
        self.phase = Phase::Connected(Session {
            attempt: attempt.id,
            since: connected.since,
            tunnel_ip: connected.tunnel_ip,
            planned_reconnect: connected.planned_reconnect,
            underlay: connected.underlay,
            renewal: None,
            replaced: attempt.replacing.map(|replaced| replaced.attempt),
        });
        Vec::new()
    }

    pub(in crate::connection::machine) fn end_run(&mut self, exit: RunExit) -> Vec<Step> {
        match exit {
            RunExit::Ignore => Vec::new(),
            RunExit::Attempt(exit) => {
                let AttemptExit {
                    id,
                    learned,
                    otp_close,
                    then,
                } = *exit;
                let Phase::Connecting(mut attempt) = self.take_phase() else {
                    unreachable!("decided for the attempt's run");
                };
                self.learn_certificates(learned);
                self.push_otp_close(otp_close);
                match then {
                    AfterAttemptExit::RestartReplaced(restart) => {
                        let session = attempt.replacing.take().expect("decided with a session");
                        self.restart(session, restart);
                        Vec::new()
                    }
                    AfterAttemptExit::ReplacementEnded(outcome) => {
                        let session = attempt.replacing.take().expect("decided with a session");
                        vec![Step::ReplacementEnded { session, outcome }]
                    }
                    AfterAttemptExit::Outcome(outcome) => {
                        self.last_failure_attempt = Some(id);
                        vec![Step::ApplyOutcome(outcome)]
                    }
                }
            }
            RunExit::ReplacedDuringAttempt => {
                let Phase::Connecting(attempt) = &mut self.phase else {
                    unreachable!("decided for an attempt under way");
                };
                attempt.replacing = None;
                Vec::new()
            }
            RunExit::ReplacedWhileConnected => {
                let Phase::Connected(session) = &mut self.phase else {
                    unreachable!("decided for the connected session");
                };
                session.replaced = None;
                Vec::new()
            }
            RunExit::Session {
                id,
                learned,
                outcome,
            } => {
                self.phase = Phase::Idle;
                self.learn_certificates(learned);
                self.last_failure_attempt = Some(id);
                vec![Step::ApplyOutcome(outcome)]
            }
            RunExit::Stop { learned, then } => {
                self.learn_certificates(learned);
                self.end_stopped_run(then)
            }
        }
    }

    fn end_stopped_run(&mut self, ended: StopEnded) -> Vec<Step> {
        match ended {
            StopEnded::AwaitReplacement { next } => {
                let Phase::Disconnecting(stop) = &mut self.phase else {
                    unreachable!("decided for a stop");
                };
                stop.replacement = None;
                if let Some(next) = next {
                    stop.attempt = next;
                }
                Vec::new()
            }
            StopEnded::Settle(after) => {
                self.phase = Phase::Idle;
                self.settle(after)
            }
        }
    }

    pub(in crate::connection::machine) fn reject_run(
        &mut self,
        rejection: RunRejection,
    ) -> Vec<Step> {
        match rejection {
            RunRejection::Ignore => Vec::new(),
            RunRejection::Attempt { otp_close, then } => {
                let Phase::Connecting(mut attempt) = self.take_phase() else {
                    unreachable!("decided for the attempt's run");
                };
                self.push_otp_close(otp_close);
                match then {
                    AfterRejection::ReplacementFailed => {
                        let session = attempt.replacing.take().expect("decided with a session");
                        vec![Step::ReplacementFailed(session)]
                    }
                    AfterRejection::Fail(failure) => {
                        self.fail(failure);
                        Vec::new()
                    }
                    AfterRejection::NetworkFailure => {
                        vec![Step::NetworkFailure(RetryReason::Network)]
                    }
                }
            }
            RunRejection::Stop(ended) => self.end_stopped_run(ended),
        }
    }

    pub(in crate::connection::machine) fn lose_connection(
        &mut self,
        loss: ConnectionLoss,
    ) -> Vec<Step> {
        match loss {
            ConnectionLoss::Ignore => Vec::new(),
            ConnectionLoss::DropReplaced => {
                let Phase::Connecting(attempt) = &mut self.phase else {
                    unreachable!("decided for an attempt under way");
                };
                attempt.replacing = None;
                Vec::new()
            }
            ConnectionLoss::EndRun { otp_close, then } => {
                self.phase = Phase::Idle;
                self.push_otp_close(otp_close);
                match then {
                    AfterLoss::Fail(failure) => {
                        self.fail(failure);
                        Vec::new()
                    }
                    AfterLoss::ForcedTermination => vec![Step::ForcedTermination],
                }
            }
            ConnectionLoss::EndSession => {
                self.phase = Phase::Idle;
                vec![Step::ForcedTermination]
            }
            ConnectionLoss::Settle(after) => {
                self.phase = Phase::Idle;
                self.settle(after)
            }
        }
    }
}
