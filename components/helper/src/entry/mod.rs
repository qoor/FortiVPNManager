mod bypass_routes;

use std::collections::HashMap;
use std::env;
use std::mem;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::app_helper::{
    AttemptId, ClientStatus, ExitFacts, HelperError, HelperErrorCode, HelperEvent, HelperEventKind,
    HelperStatus, Milestone, OutputBatch, RunRequest, StopReason,
};
use fvm_contract::helper_internal::{
    AddedRoute, AppRequest, FromEntry, LogCategory, LogLevel, NetworkChange, NetworkOutcome,
    ToEntry,
};
use serde::Serialize;
use zeroize::{Zeroize, Zeroizing};

use crate::recovery::RecordSlot;
use crate::session::{NotReplaceable, Replaced, Report, Run, SessionManager, StartFailure};

use bypass_routes::{BypassRoutes, Job};

/// How long exiting after the OS's termination request waits for the run to report its exit.
/// Longer than the logout request's wait for the gateway, so a run whose session could not log out
/// itself still logs out and is reported first.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(15);
/// Lets the reply to an exitWhenIdle request leave before the Helper exits.
const EXIT_WHEN_IDLE_DELAY: Duration = Duration::from_millis(500);

/// Takes the OS side's messages to the entry.
pub struct Inbox(Sender<Event>);

impl Inbox {
    /// Passes on a message, in the order the OS side received them.
    pub fn deliver(&self, message: ToEntry) {
        self.0
            .send(Event::FromOs(message))
            .expect("the entry runs as long as the process");
    }
}

/// Starts the entry on its own thread with `sessions` for the VPN client. `send` takes the entry's
/// messages for the OS side, in order, on the entry's thread; the first is "ready", once the
/// session manager cleaned up after an earlier Helper.
pub fn start(sessions: SessionManager, send: impl Fn(&FromEntry) + Send + 'static) -> Inbox {
    let (sender, events) = mpsc::channel();
    let inbox = Inbox(sender.clone());
    thread::Builder::new()
        .name("fvm-entry".into())
        .spawn(move || {
            serve(sessions, OsSide(Box::new(send)), sender, events);
        })
        .expect("starting the entry thread");
    inbox
}

enum Event {
    FromOs(ToEntry),
    Run(AttemptId, Report),
}

/// Where the entry's messages for the OS side go, in order.
struct OsSide(Box<dyn Fn(&FromEntry) + Send>);

impl OsSide {
    pub fn send(&self, message: &FromEntry) {
        (self.0)(message);
    }

    /// Never with a secret: the line goes to the system log.
    pub fn log(&self, level: LogLevel, category: LogCategory, message: String) {
        self.send(&FromEntry::Log {
            level,
            category,
            message,
        });
    }
}

fn serve(sessions: SessionManager, os: OsSide, sender: Sender<Event>, events: Receiver<Event>) {
    let left = clean_up(&sessions, &os, &events);
    os.send(&FromEntry::Ready {});
    Entry {
        sessions,
        os,
        events,
        sender,
        apps: HashMap::new(),
        runs: HashMap::new(),
        executable_replaced: false,
        lifecycle: Lifecycle::Serving,
        bypass_routes: BypassRoutes::new(left),
    }
    .run()
}

/// Has the session manager clean up after an earlier Helper, with the OS side reverting the network
/// settings a run that did not finish changed and removing the exception routes the earlier Helper
/// added, before any app connection is accepted. Returns the exception routes that could not be
/// removed. Until "ready", the OS side accepts no app connection and passes on no stop request, so
/// the answers to these requests are the only messages it sends.
fn clean_up(sessions: &SessionManager, os: &OsSide, events: &Receiver<Event>) -> Vec<AddedRoute> {
    sessions.clean_up(
        |level, category, message| os.log(level, category, message),
        |changes| {
            let routes = changes.routes.len();
            os.send(&FromEntry::ChangeNetwork {
                change: NetworkChange::Revert { changes },
            });
            let Event::FromOs(ToEntry::NetworkChanged { outcome }) =
                events.recv().expect("the entry holds a sender")
            else {
                panic!(
                    "the OS side sent something other than the network revert's answer before ready"
                );
            };
            let (level, message) = match outcome {
                NetworkOutcome::Reverted { left } => (
                    LogLevel::Notice,
                    format!(
                        "reverted the network settings an unfinished run left ({} of {routes} route(s) still in place)",
                        left.len()
                    ),
                ),
                NetworkOutcome::Failed { step, error, .. } => (
                    LogLevel::Error,
                    format!(
                        "reverting the network settings an unfinished run left failed ({step:?}): {error}"
                    ),
                ),
                other => panic!("the OS side answered a network revert with {other:?}"),
            };
            os.log(level, LogCategory::Routes, message);
        },
        |recorded| {
            let change = bypass_routes::removal(&recorded);
            os.send(&FromEntry::ChangeBypassRoutes { change });
            let Event::FromOs(ToEntry::BypassRoutesChanged { outcome }) =
                events.recv().expect("the entry holds a sender")
            else {
                panic!(
                    "the OS side sent something other than the exception routes' answer before ready"
                );
            };
            let left = bypass_routes::left_after_removal(&recorded, outcome).unwrap_or_else(
                |(left, error)| {
                    let message = format!("removing exception routes failed: {}", error.message);
                    os.log(LogLevel::Error, LogCategory::Routes, message);
                    left
                },
            );
            os.log(
                LogLevel::Notice,
                LogCategory::Routes,
                format!(
                    "removed {} of the {} exception route(s) an earlier helper added",
                    recorded.len() - left.len(),
                    recorded.len()
                ),
            );
            left
        },
    )
}

struct Entry {
    sessions: SessionManager,
    os: OsSide,
    events: Receiver<Event>,
    /// For the run's reports.
    sender: Sender<Event>,
    /// Admitted app connections that are still open, with their uid.
    apps: HashMap<u64, u32>,
    runs: HashMap<AttemptId, HeldRun>,
    executable_replaced: bool,
    lifecycle: Lifecycle,
    bypass_routes: BypassRoutes,
}

struct HeldRun {
    owner_app: u64,
    owner_uid: u32,
    replacing: Option<AttemptId>,
    slot: RecordSlot,
    run: Run,
}

enum Lifecycle {
    Serving,
    /// Answered an exitWhenIdle request with true; tells the OS side to exit at `at`.
    ExitingWhenIdle {
        at: Instant,
    },
    ShuttingDown {
        deadline: Instant,
    },
    /// Told the OS side to exit.
    Exiting,
}

impl Lifecycle {
    fn deadline(&self) -> Option<Instant> {
        match *self {
            Self::ExitingWhenIdle { at } => Some(at),
            Self::ShuttingDown { deadline } => Some(deadline),
            Self::Serving | Self::Exiting => None,
        }
    }
}

impl Entry {
    fn run(mut self) -> ! {
        loop {
            let event = match self.lifecycle.deadline() {
                None => Some(self.events.recv().expect("the entry holds a sender")),
                Some(deadline) => match self
                    .events
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    Ok(event) => Some(event),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => unreachable!("the entry holds a sender"),
                },
            };
            if let Some(event) = event {
                self.handle(event);
            }
            if self
                .lifecycle
                .deadline()
                .is_some_and(|deadline| deadline <= Instant::now())
            {
                self.deadline_passed();
            }
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::FromOs(ToEntry::AppConnected { app, uid }) => self.admit(app, uid),
            Event::FromOs(ToEntry::AppDisconnected { app }) => self.app_disconnected(app),
            Event::FromOs(ToEntry::Request { id, app, request }) => self.request(id, app, request),
            Event::FromOs(ToEntry::ExecutableReplaced {}) => self.executable_replaced(),
            Event::FromOs(ToEntry::TerminationRequested {}) => self.termination_requested(),
            Event::FromOs(ToEntry::BypassRoutesChanged { outcome }) => {
                self.bypass_routes_changed(outcome);
            }
            Event::FromOs(ToEntry::NetworkChanged { outcome }) => self.network_changed(outcome),
            Event::Run(attempt, report) => self.run_report(attempt, report),
        }
    }

    fn admit(&mut self, app: u64, uid: u32) {
        let accept = matches!(self.lifecycle, Lifecycle::Serving);
        if accept {
            self.apps.insert(app, uid);
        }
        self.os.send(&FromEntry::Admitted { app, accept });
    }

    fn app_disconnected(&mut self, app: u64) {
        self.apps.remove(&app);
        self.app_gone_with_its_bypass_routes(app);
        for held in self.runs.values().filter(|held| held.owner_app == app) {
            self.os.log(
                LogLevel::Notice,
                LogCategory::Helper,
                format!("{}: owning connection invalidated", held.run.name()),
            );
            held.run.stop(StopReason::AppDisconnected);
        }
    }

    fn executable_replaced(&mut self) {
        self.executable_replaced = true;
        if matches!(self.lifecycle, Lifecycle::Serving) && self.runs.is_empty() {
            self.exit_for_replacement();
        }
    }

    /// A Helper whose executable was replaced no longer passes the app's signature check; launchd
    /// starts the new one on the next request.
    fn exit_for_replacement(&mut self) {
        let path = env::current_exe().expect("the Helper's executable path");
        self.os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!(
                "executable replaced at {}; exiting so launchd starts the new build",
                path.display()
            ),
        );
        self.exit(FromEntry::ExitAfterDraining {});
    }

    fn termination_requested(&mut self) {
        match self.lifecycle {
            Lifecycle::Serving | Lifecycle::ExitingWhenIdle { .. } if self.runs.is_empty() => {
                self.exit(FromEntry::ExitAfterDraining {});
            }
            Lifecycle::Serving | Lifecycle::ExitingWhenIdle { .. } => {
                for held in self.runs.values() {
                    held.run.stop(StopReason::HelperShutdown);
                }
                self.lifecycle = Lifecycle::ShuttingDown {
                    deadline: Instant::now() + SHUTDOWN_LIMIT,
                };
            }
            // The request may come again; the first one decided already.
            Lifecycle::ShuttingDown { .. } | Lifecycle::Exiting => {}
        }
    }

    fn deadline_passed(&mut self) {
        match self.lifecycle {
            Lifecycle::ExitingWhenIdle { .. } => self.exit(FromEntry::ExitAfterDraining {}),
            Lifecycle::ShuttingDown { .. } => {
                self.os.log(
                    LogLevel::Error,
                    LogCategory::Helper,
                    "the run did not finish after the termination request; exiting anyway"
                        .to_owned(),
                );
                self.exit(FromEntry::ExitNow {});
            }
            Lifecycle::Serving | Lifecycle::Exiting => {
                unreachable!("only a lifecycle with a deadline passes it")
            }
        }
    }

    fn exit(&mut self, message: FromEntry) {
        self.os.send(&message);
        self.lifecycle = Lifecycle::Exiting;
    }

    /// Answers every request exactly once, including those of an app that is gone.
    fn request(&mut self, id: u64, app: u64, request: AppRequest) {
        let result = match request {
            AppRequest::Connect { json } => self.connect(app, json),
            AppRequest::SubmitOtp { attempt, code } => self.submit_otp(app, &attempt, code),
            AppRequest::Disconnect { attempt, reason } => self.disconnect(app, &attempt, &reason),
            AppRequest::Status {} => {
                let json = Some(to_json(&self.status()));
                self.os.send(&FromEntry::Reply { id, json });
                return;
            }
            AppRequest::ExitWhenIdle {} => {
                let value = self.exit_when_idle();
                self.os.send(&FromEntry::ReplyFlag { id, value });
                return;
            }
            AppRequest::ReadBypassRoutes {} => return self.read_bypass_routes(app, id),
            AppRequest::WriteBypassRoutes { json } => {
                return self.write_bypass_routes(app, id, &json);
            }
        };
        let json = result.err().map(|error| to_json(&error));
        self.os.send(&FromEntry::Reply { id, json });
    }

    /// The uid of an app that may act on runs: requests can still arrive from a connection that is
    /// already gone, and no run must start for it.
    fn app_uid(&self, app: u64) -> Result<u32, HelperError> {
        self.apps.get(&app).copied().ok_or_else(|| {
            error(
                HelperErrorCode::NotAuthorized,
                "the app connection is closed",
            )
        })
    }

    fn connect(&mut self, app: u64, json: String) -> Result<(), HelperError> {
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
        let reports = self.sender.clone();
        let attempt = request.attempt_id.clone();
        let started = self.sessions.start(request, slot, replaced, move |report| {
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

    fn submit_otp(&self, app: u64, attempt: &str, code: String) -> Result<(), HelperError> {
        let mut code = Zeroizing::new(code);
        let uid = self.app_uid(app)?;
        let held = self.controllable_run(attempt, uid)?;
        held.run.submit_otp(mem::take(&mut *code))
    }

    fn disconnect(&self, app: u64, attempt: &str, reason: &str) -> Result<(), HelperError> {
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

    fn status(&self) -> HelperStatus {
        HelperStatus {
            client_running: self.runs.values().any(|held| held.run.session_running()),
            client: ClientStatus {
                version: Some(self.sessions.version().to_owned()),
            },
        }
    }

    fn exit_when_idle(&mut self) -> bool {
        if !self.runs.is_empty() {
            return false;
        }
        if matches!(self.lifecycle, Lifecycle::Serving) {
            self.os.log(
                LogLevel::Notice,
                LogCategory::Helper,
                "exiting on request so launchd starts the current binary".to_owned(),
            );
            self.lifecycle = Lifecycle::ExitingWhenIdle {
                at: Instant::now() + EXIT_WHEN_IDLE_DELAY,
            };
        }
        true
    }

    fn run_report(&mut self, attempt: AttemptId, report: Report) {
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

    fn run_exited(&mut self, attempt: &AttemptId, facts: &ExitFacts) {
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
                "{} exited: {:?}, initiator {:?}, furthest {:?}",
                exited.run.name(),
                facts.exit_status,
                facts.stop_initiator,
                facts.furthest_milestone
            ),
        );
        let idle = self.runs.is_empty();
        match self.lifecycle {
            Lifecycle::ShuttingDown { .. } if idle => self.exit(FromEntry::ExitAfterDraining {}),
            Lifecycle::Serving if idle && self.executable_replaced => self.exit_for_replacement(),
            Lifecycle::Serving
            | Lifecycle::ShuttingDown { .. }
            | Lifecycle::ExitingWhenIdle { .. }
            | Lifecycle::Exiting => {}
        }
    }
}

fn error(code: HelperErrorCode, message: impl Into<String>) -> HelperError {
    HelperError {
        code,
        message: message.into(),
    }
}

fn to_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("contract types serialize")
}
