use std::mem;
use std::sync::mpsc;

use fvm_contract::app_helper::{
    AttemptId, EndFacts, HelperError, HelperErrorCode, HelperEvent, HelperEventKind, Milestone,
    OutputBatch, RunRequest, StopReason,
};
use fvm_contract::helper_internal::{FromEntry, LogCategory, LogLevel};
use zeroize::{Zeroize, Zeroizing};

use crate::recovery::RecordSlot;
use crate::session::{NotReplaceable, Replaced, Report, StartFailure};

use super::bypass_routes::Job;
use super::{Entry, Event, HeldRun, Lifecycle, error, to_json};

impl Entry {
    pub(super) fn connect(&mut self, app: u64, json: String) -> Result<(), HelperError> {
        let json = Zeroizing::new(json);
        let mut request: RunRequest = serde_json::from_str(&json).map_err(|_| {
            error(
                HelperErrorCode::InvalidRequest,
                "could not decode the connect request",
            )
        })?;
        let started = self.start_run(app, &mut request);
        // A started run took the password; otherwise it is wiped here.
        request.password.zeroize();
        started
    }

    fn start_run(&mut self, app: u64, request: &mut RunRequest) -> Result<(), HelperError> {
        let uid = self.app_uid(app)?;
        if self.runs.contains_key(&request.attempt_id) {
            return Err(error(
                HelperErrorCode::InvalidRequest,
                "the attempt ID belongs to a running connection",
            ));
        }
        if !matches!(self.lifecycle, Lifecycle::Serving) {
            return Err(error(
                HelperErrorCode::Busy,
                "the Helper is shutting down; try again shortly",
            ));
        }
        let replaced = match &request.replacing {
            Some(replacing) => Some(self.replaceable(replacing, uid)?),
            None => match self.runs.values().next() {
                Some(held) if held.owner_uid == uid => {
                    return Err(error(
                        HelperErrorCode::Busy,
                        format!("{} is already running", held.run.client_name()),
                    ));
                }
                Some(_) => {
                    return Err(error(
                        HelperErrorCode::NotAuthorized,
                        "a VPN run started by another user is active",
                    ));
                }
                None => None,
            },
        };
        let slot = if self
            .runs
            .values()
            .any(|held| held.slot == RecordSlot::First)
        {
            RecordSlot::Second
        } else {
            RecordSlot::First
        };
        let (start_session, start_after) = if replaced.is_none() && !self.unreverted.is_empty() {
            let (start_session, start_after) = mpsc::channel();
            (Some(start_session), Some(start_after))
        } else {
            (None, None)
        };
        let reports = self.sender.clone();
        let attempt = request.attempt_id.clone();
        let started = self
            .sessions
            .start(request, slot, replaced, start_after, move |report| {
                reports
                    .send(Event::Run(attempt.clone(), report))
                    .expect("the event loop runs as long as the process");
            });
        let run = started.map_err(|failure| match failure {
            StartFailure::InvalidRequest(reason) => error(HelperErrorCode::InvalidRequest, reason),
            StartFailure::RunDirectory(failure) => {
                self.os.log(
                    LogLevel::Error,
                    LogCategory::Helper,
                    format!("cannot prepare the run directory: {failure}"),
                );
                error(
                    HelperErrorCode::InternalError,
                    format!("could not prepare the run directory: {failure}"),
                )
            }
        })?;
        if let Some(start_session) = start_session {
            self.queue_bypass_job(Job::RevertAgain {
                kept: self.unreverted.clone(),
                answered: 0,
                left: Vec::new(),
                start_session,
            });
        }
        let replacing = match &request.replacing {
            Some(replacing) => format!(", replacing {}", self.runs[replacing].run.name()),
            None => String::new(),
        };
        self.os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!(
                "{} started for uid {uid} with {} {}{replacing}",
                run.name(),
                run.client_name(),
                self.sessions.version()
            ),
        );
        self.runs.insert(
            request.attempt_id.clone(),
            HeldRun {
                owner_app: app,
                owner_uid: uid,
                replacing: request.replacing.clone(),
                slot,
                run,
            },
        );
        Ok(())
    }

    fn replaceable(&self, replacing: &AttemptId, uid: u32) -> Result<Replaced, HelperError> {
        let held = self.runs.get(replacing).ok_or_else(|| {
            error(
                HelperErrorCode::UnknownAttempt,
                "the connection to replace is not running",
            )
        })?;
        if held.owner_uid != uid {
            return Err(error(
                HelperErrorCode::NotAuthorized,
                "the VPN run belongs to another user",
            ));
        }
        if self.runs.len() > 1 {
            return Err(error(
                HelperErrorCode::Busy,
                "a replacement connection is already running",
            ));
        }
        held.run.replaceable().map_err(|NotReplaceable::NotUp| {
            error(
                HelperErrorCode::WrongPhase,
                "the connection to replace is not up",
            )
        })
    }

    pub(super) fn submit_otp(
        &self,
        app: u64,
        attempt: &str,
        code: String,
    ) -> Result<(), HelperError> {
        let mut code = Zeroizing::new(code);
        let uid = self.app_uid(app)?;
        let held = self.controllable_run(attempt, uid)?;
        held.run.submit_otp(mem::take(&mut *code))
    }

    pub(super) fn disconnect(
        &self,
        app: u64,
        attempt: &str,
        reason: &str,
    ) -> Result<(), HelperError> {
        let uid = self.app_uid(app)?;
        let held = self.controllable_run(attempt, uid)?;
        let reason = reason.parse().unwrap_or(StopReason::UserDisconnect);
        let raw = serde_json::to_value(reason).expect("a stop reason serializes");
        let raw = raw
            .as_str()
            .expect("a stop reason serializes as its raw value");
        self.os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!("{}: stop requested ({raw})", held.run.name()),
        );
        held.run.stop(reason);
        Ok(())
    }

    fn controllable_run(&self, attempt: &str, uid: u32) -> Result<&HeldRun, HelperError> {
        let held = attempt
            .parse::<AttemptId>()
            .ok()
            .and_then(|id| self.runs.get(&id))
            .ok_or_else(|| {
                error(
                    HelperErrorCode::UnknownAttempt,
                    "not the active connection attempt",
                )
            })?;
        if held.owner_uid != uid {
            return Err(error(
                HelperErrorCode::NotAuthorized,
                "the VPN run belongs to another user",
            ));
        }
        Ok(held)
    }

    pub(super) fn run_report(&mut self, attempt: AttemptId, report: Report) {
        let app = self
            .runs
            .get(&attempt)
            .expect("reports come from a held run")
            .owner_app;
        match report {
            Report::Output(lines) => {
                let batch = OutputBatch {
                    attempt_id: attempt,
                    lines,
                };
                let json = to_json(&batch);
                self.os.send(&FromEntry::SendOutput { app, json });
            }
            Report::Event(kind) => {
                let event = HelperEvent {
                    attempt_id: attempt,
                    kind,
                };
                self.os.send(&FromEntry::SendEvent {
                    app,
                    json: to_json(&event),
                });
                match event.kind {
                    HelperEventKind::Milestone {
                        milestone: Milestone::TunnelUp,
                        ..
                    } => {
                        self.os.log(
                            LogLevel::Notice,
                            LogCategory::Helper,
                            format!("{}: tunnel up", self.runs[&event.attempt_id].run.name()),
                        );
                        self.stop_replaced(&event.attempt_id);
                    }
                    HelperEventKind::Milestone { .. } => {}
                    HelperEventKind::Exited { facts } => {
                        self.run_exited(&event.attempt_id, &facts);
                    }
                }
            }
            Report::ChangeNetwork(job) => self.queue_bypass_job(Job::Network(job)),
            Report::Log(level, category, message) => self.os.log(level, category, message),
            Report::Stuck => {
                if !matches!(self.lifecycle, Lifecycle::Exiting) {
                    self.exit(&FromEntry::AbortAfterDraining {});
                }
            }
        }
    }

    fn stop_replaced(&mut self, attempt: &AttemptId) {
        let held = self
            .runs
            .get_mut(attempt)
            .expect("reports come from a held run");
        let Some(replaced) = held.replacing.take() else {
            return;
        };
        let replaced_run = self
            .runs
            .get(&replaced)
            .expect("a replaced run stays held until its exit");
        self.os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!(
                "{}: replaced by {}",
                replaced_run.run.name(),
                self.runs[attempt].run.name()
            ),
        );
        replaced_run.run.stop(StopReason::Replaced);
    }

    fn run_exited(&mut self, attempt: &AttemptId, facts: &EndFacts) {
        let exited = self.runs.remove(attempt).expect("a held run exited");
        for held in self.runs.values_mut() {
            if held.replacing.as_ref() == Some(attempt) {
                held.replacing = None;
                held.run.replaced_run_exited();
            }
            if exited.slot == RecordSlot::First {
                held.slot = RecordSlot::First;
                held.run.first_slot_freed();
            }
        }
        self.os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!(
                "{} exited: code {}, initiator {:?}, furthest {:?}",
                exited.run.name(),
                facts.end_code,
                facts.stop_initiator,
                facts.furthest_milestone
            ),
        );
        let idle = self.runs.is_empty();
        match self.lifecycle {
            Lifecycle::ShuttingDown { .. } if idle => self.exit(&FromEntry::ExitAfterDraining {}),
            Lifecycle::Serving if idle && self.executable_replaced => self.exit_for_replacement(),
            Lifecycle::Serving
            | Lifecycle::ShuttingDown { .. }
            | Lifecycle::ExitingWhenIdle { .. }
            | Lifecycle::Exiting => {}
        }
    }
}
