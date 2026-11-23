use std::time::{Duration, SystemTime};

use fvm_contract::app_helper::StopReason;
use fvm_contract::app_internal::event::PathInterface;

use crate::connection::machine::{
    AfterStop, AttemptReason, ConnectionManager, Phase, RetryReason, Step,
};
use crate::connection::system::{PathSnapshot, RawPath, SettleStep, WakeObservation};
use crate::connection::{Effect, NetworkProbe};

use super::decision::{
    AfterPath, AfterWake, PathLook, PathReport, ProbeAnswer, ProbePlan, Resume, Sleep, Wake,
    WakeEvaluation, WakeLook, WakeSettled,
};

impl ConnectionManager {
    pub(in crate::connection::machine) fn sleep_reported(&mut self, at: SystemTime) -> Vec<Step> {
        self.system.sleep_reported_at = Some(at);
        self.system.wake_settle = None;
        self.system.sleeping = true;
        vec![Step::UpdatePresence(Some(at)), Step::WillSleep]
    }

    pub(in crate::connection::machine) fn go_to_sleep(&mut self, sleep: Sleep) -> Vec<Step> {
        match sleep {
            Sleep::Keep => Vec::new(),
            Sleep::PauseReplacement {
                stop_run,
                otp_close,
                next_try,
            } => {
                let Phase::Connecting(mut attempt) = self.take_phase() else {
                    unreachable!("decided for a replacement under way");
                };
                let mut session = attempt.replacing.take().expect("decided with a session");
                if stop_run {
                    self.effects.push(Effect::StopRun {
                        attempt: attempt.id.clone(),
                        reason: StopReason::Sleep,
                    });
                }
                self.push_otp_close(otp_close);
                session
                    .renewal
                    .as_mut()
                    .expect("a replacement is prepared for a renewal")
                    .next_try = Some(next_try);
                self.phase = Phase::Connected(session);
                Vec::new()
            }
            Sleep::AbortAttempt => {
                let Phase::Connecting(attempt) = self.take_phase() else {
                    unreachable!("decided for an attempt under way");
                };
                vec![Step::AbortAttempt {
                    attempt,
                    stop: StopReason::Sleep,
                    after: AfterStop::ResumeAfterSleep,
                }]
            }
            Sleep::WaitForWake => {
                self.wait_for_network(RetryReason::Sleep, Duration::ZERO);
                Vec::new()
            }
            Sleep::ResumeAfterWake => {
                let Phase::Disconnecting(stop) = &mut self.phase else {
                    unreachable!("decided for a stop");
                };
                stop.after = AfterStop::ResumeAfterSleep;
                Vec::new()
            }
        }
    }

    pub(in crate::connection::machine) fn wake_up(&mut self, wake: Wake) -> Vec<Step> {
        self.system.sleep_reported_at = None;
        self.system.sleeping = false;
        self.last_wake_at = Some(wake.at);
        self.pending_wake = Some(wake.slept_for);
        self.apply_away_limit(wake.limit);
        // Whether the screen is locked after the wake decides presence; the answer judges it.
        self.effects.push(Effect::ReadScreenState);
        self.system.wake_settle = Some(wake.settle);
        vec![Step::WakeLookDue]
    }

    pub(in crate::connection::machine) fn look_after_wake(&mut self, look: WakeLook) -> Vec<Step> {
        match look {
            WakeLook::Ignore => Vec::new(),
            WakeLook::ReadRouter => {
                self.wake_settle_step(SettleStep::ReadingRouter);
                self.effects.push(Effect::ReadRouter);
                Vec::new()
            }
            WakeLook::LookAgain { at } => {
                self.wake_settle_step(SettleStep::Waiting(at));
                Vec::new()
            }
            WakeLook::JudgeWithoutNetwork => {
                self.system
                    .wake_settle
                    .as_mut()
                    .expect("a wake waits for the network")
                    .late = true;
                vec![
                    Step::WakeSettled(WakeObservation::NoNetwork),
                    Step::WakeLookDue,
                ]
            }
            WakeLook::ReportPath => {
                self.system.wake_settle = None;
                vec![Step::ReportPath]
            }
            WakeLook::Observe(plan) => {
                let (id, host) = self.probe(plan);
                self.wake_settle_step(SettleStep::Observing(id, host));
                Vec::new()
            }
        }
    }

    fn wake_settle_step(&mut self, step: SettleStep) {
        self.system
            .wake_settle
            .as_mut()
            .expect("a wake waits for the network")
            .step = step;
    }

    /// Asks for the route to the gateway; returns the probe's ID and the host it looks up.
    fn probe(&mut self, plan: ProbePlan) -> (u64, String) {
        self.system.next_probe += 1;
        self.effects.push(Effect::ProbeNetwork(NetworkProbe {
            id: self.system.next_probe,
            gateway: plan.gateway,
            session_running: plan.session_running,
        }));
        (self.system.next_probe, plan.host)
    }

    pub(in crate::connection::machine) fn record_path(
        &mut self,
        satisfied: bool,
        interfaces: Vec<PathInterface>,
        report: PathReport,
    ) -> Vec<Step> {
        self.system.raw_path = Some(RawPath {
            satisfied,
            interfaces,
        });
        self.system.path_settle_at = Some(report.settle_at);
        self.system.path_probe = None;
        if report.look_now {
            vec![Step::WakeLookDue]
        } else {
            Vec::new()
        }
    }

    pub(in crate::connection::machine) fn path_settled(&mut self) -> Vec<Step> {
        self.system.path_settle_at = None;
        vec![Step::ReportPath]
    }

    pub(in crate::connection::machine) fn look_at_path(&mut self, look: PathLook) -> Vec<Step> {
        match look {
            PathLook::Probe(plan) => {
                let probe = self.probe(plan);
                self.system.path_probe = Some(probe);
                Vec::new()
            }
            PathLook::Unsatisfied => vec![Step::PathChanged {
                snapshot: PathSnapshot {
                    satisfied: false,
                    underlay: None,
                },
                routing_incomplete: false,
            }],
        }
    }

    pub(in crate::connection::machine) fn take_probe_answer(
        &mut self,
        answer: ProbeAnswer,
    ) -> Vec<Step> {
        match answer {
            ProbeAnswer::Ignore => Vec::new(),
            ProbeAnswer::Path {
                gateway_ip,
                snapshot,
                routing_incomplete,
            } => {
                if gateway_ip.is_some() {
                    self.system.gateway_ip = gateway_ip;
                }
                self.system.path_probe = None;
                vec![Step::PathChanged {
                    snapshot,
                    routing_incomplete,
                }]
            }
            ProbeAnswer::Wake {
                gateway_ip,
                observation,
            } => {
                if gateway_ip.is_some() {
                    self.system.gateway_ip = gateway_ip;
                }
                self.system.wake_settle = None;
                vec![Step::WakeSettled(observation)]
            }
        }
    }

    pub(in crate::connection::machine) fn settle_wake(
        &mut self,
        settled: WakeSettled,
    ) -> Vec<Step> {
        self.pending_wake = None;
        self.path = Some(settled.snapshot);
        match settled.then {
            AfterWake::Nothing => Vec::new(),
            AfterWake::Session(evaluation) => self.apply_wake_evaluation(evaluation),
            AfterWake::Resume(resume) => self.resume(resume),
        }
    }

    fn apply_wake_evaluation(&mut self, evaluation: WakeEvaluation) -> Vec<Step> {
        match evaluation {
            WakeEvaluation::Keep(session) => {
                self.phase = Phase::Connected(session);
                self.rearm_proactive_timer();
                Vec::new()
            }
            WakeEvaluation::Restart { session, restart } => {
                self.restart(session, restart);
                Vec::new()
            }
            WakeEvaluation::Renew(session) => crate::connection::machine::attempts::action::renew(
                session,
                AttemptReason::Proactive,
            ),
        }
    }

    pub(in crate::connection::machine) fn resume(&mut self, resume: Resume) -> Vec<Step> {
        match resume {
            Resume::Wait { until, reason } => {
                self.phase = Phase::WaitingRetry { until, reason };
                Vec::new()
            }
            Resume::Attempt(reason) => vec![Step::begin_attempt(reason)],
        }
    }

    pub(in crate::connection::machine) fn change_path(
        &mut self,
        snapshot: PathSnapshot,
        after: AfterPath,
    ) -> Vec<Step> {
        self.path = Some(snapshot);
        match after {
            AfterPath::Nothing => Vec::new(),
            AfterPath::Resume(resume) => self.resume(resume),
            AfterPath::RestartSession(restart) => {
                let Phase::Connected(session) = self.take_phase() else {
                    unreachable!("decided for the connected session");
                };
                self.restart(session, restart);
                Vec::new()
            }
            AfterPath::AbortReplacement => {
                let Phase::Connecting(attempt) = self.take_phase() else {
                    unreachable!("decided for a replacement under way");
                };
                vec![Step::AbortAttempt {
                    attempt,
                    stop: StopReason::Reconnect,
                    after: AfterStop::Reconnect(AttemptReason::NetworkChanged),
                }]
            }
            AfterPath::SameRoute { adopt, restart } => {
                if adopt.is_some() {
                    let session = match &mut self.phase {
                        Phase::Connected(session) => session,
                        Phase::Connecting(attempt) => attempt
                            .replacing
                            .as_mut()
                            .expect("decided with a session to replace"),
                        _ => unreachable!("decided for a session"),
                    };
                    session.underlay = adopt;
                }
                if let Some(restart) = restart {
                    let Phase::Connected(session) = self.take_phase() else {
                        unreachable!("decided for the connected session");
                    };
                    self.restart(session, restart);
                }
                Vec::new()
            }
        }
    }
}
