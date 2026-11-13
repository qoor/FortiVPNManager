//! The Helper entry's event loop. The OS side's messages and the current run's reports arrive on
//! one channel and are handled one at a time on one thread. So what reaches an app keeps the order
//! in which it happened, and the run thread, which only forwards, never waits for this one.

use std::collections::HashMap;
use std::env;
use std::mem;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use fvm_contract::app_helper::{
    AttemptId, ExitFacts, HelperError, HelperErrorCode, HelperEvent, HelperEventKind, HelperStatus,
    Milestone, RunRequest, StopReason,
};
use fvm_contract::backend::{Backend, ClientProblemKind};
use fvm_contract::helper_internal::{AppRequest, FromEntry, LogCategory, LogLevel, ToEntry};
use fvm_watchdog::{Report, RouteRepair, Run, StartFailure, Watchdog};
use serde::Serialize;
use zeroize::{Zeroize, Zeroizing};

/// How long exiting after the OS's termination request waits for the run to report its exit.
/// Longer than the client's grace period, so a client that ignores the stop request is killed and
/// reported first.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(15);
/// Lets the reply to an exitWhenIdle request leave before the Helper exits.
const EXIT_WHEN_IDLE_DELAY: Duration = Duration::from_millis(500);

pub(crate) enum Event {
    FromOs(ToEntry),
    Run(Report),
}

/// Where the entry's messages for the OS side go, in order.
pub(crate) struct OsSide(pub(crate) Box<dyn Fn(&FromEntry) + Send>);

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

pub(crate) fn serve<B>(
    watchdog: Watchdog<B>,
    os: OsSide,
    sender: Sender<Event>,
    events: Receiver<Event>,
) where
    B: Backend + Send + Sync + 'static,
{
    clean_up(&watchdog, &os, &events);
    os.send(&FromEntry::Ready {});
    Entry {
        watchdog,
        os,
        events,
        sender,
        apps: HashMap::new(),
        run: None,
        executable_replaced: false,
        lifecycle: Lifecycle::Serving,
        route_repair: None,
    }
    .run()
}

/// Has the watchdog clean up after a Helper that died during a run, and the OS side repair the
/// routes that run may have left broken, before any app connection is accepted. Until "ready", the
/// OS side accepts no app connection and passes on no stop request, so the route repair's answer is
/// the only message it sends.
fn clean_up<B>(watchdog: &Watchdog<B>, os: &OsSide, events: &Receiver<Event>)
where
    B: Backend + Send + Sync + 'static,
{
    watchdog.clean_up(
        |level, category, message| os.log(level, category, message),
        |gateway| {
            os.send(&FromEntry::RepairRoutes { gateway });
            let Event::FromOs(ToEntry::RoutesRepaired { outcome }) =
                events.recv().expect("the entry holds a sender")
            else {
                panic!(
                    "the OS side sent something other than the route repair's answer before ready"
                );
            };
            os.log(
                LogLevel::Notice,
                LogCategory::Helper,
                format!("cleaned up after an unfinished run (route repair: {outcome:?})"),
            );
        },
    );
}

struct Entry<B> {
    watchdog: Watchdog<B>,
    os: OsSide,
    events: Receiver<Event>,
    /// For the run's reports.
    sender: Sender<Event>,
    /// Admitted app connections that are still open, with their uid.
    apps: HashMap<u64, u32>,
    run: Option<CurrentRun<B>>,
    executable_replaced: bool,
    lifecycle: Lifecycle,
    /// The current run's route repair while the OS side carries it out.
    route_repair: Option<RouteRepair>,
}

/// The one run at a time and who owns it.
struct CurrentRun<B> {
    attempt: AttemptId,
    owner_app: u64,
    owner_uid: u32,
    run: Run<B>,
}

enum Lifecycle {
    Serving,
    /// Answered an exitWhenIdle request with true; tells the OS side to exit at `at`.
    ExitingWhenIdle {
        at: Instant,
    },
    /// The termination request came during a run; exiting once the run reports its exit, or at
    /// `deadline` anyway.
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

impl<B> Entry<B>
where
    B: Backend + Send + Sync + 'static,
{
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
            Event::FromOs(ToEntry::RoutesRepaired { outcome }) => self
                .route_repair
                .take()
                .expect("the OS side answers only the route repair it was asked for")
                .finish(outcome),
            Event::Run(report) => self.run_report(report),
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
        if let Some(current) = self.run.as_ref().filter(|run| run.owner_app == app) {
            self.os.log(
                LogLevel::Notice,
                LogCategory::Helper,
                format!("run {}: owning connection invalidated", current.attempt),
            );
            current.run.stop(StopReason::AppDisconnected);
        }
    }

    fn executable_replaced(&mut self) {
        self.executable_replaced = true;
        if matches!(self.lifecycle, Lifecycle::Serving) && self.run.is_none() {
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
            Lifecycle::Serving | Lifecycle::ExitingWhenIdle { .. } => match &self.run {
                Some(current) => {
                    current.run.stop(StopReason::HelperShutdown);
                    self.lifecycle = Lifecycle::ShuttingDown {
                        deadline: Instant::now() + SHUTDOWN_LIMIT,
                    };
                }
                None => self.exit(FromEntry::ExitAfterDraining {}),
            },
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
        if !matches!(self.lifecycle, Lifecycle::Serving) {
            return Err(error(
                HelperErrorCode::Busy,
                "the Helper is shutting down; try again shortly",
            ));
        }
        let client_name = self.watchdog.client_name();
        if let Some(current) = &self.run {
            return Err(if current.owner_uid == uid {
                error(
                    HelperErrorCode::Busy,
                    format!("{client_name} is already running"),
                )
            } else {
                error(
                    HelperErrorCode::NotAuthorized,
                    "a VPN run started by another user is active",
                )
            });
        }
        let reports = self.sender.clone();
        let started = self.watchdog.start(request, move |report| {
            reports
                .send(Event::Run(report))
                .expect("the event loop runs as long as the process");
        });
        let (run, client) = started.map_err(|failure| match failure {
            StartFailure::ClientUnavailable(message) => {
                error(HelperErrorCode::ClientUnavailable, message)
            }
            StartFailure::InvalidRequest(reason) => error(HelperErrorCode::InvalidRequest, reason),
            StartFailure::RunFiles(failure) => {
                self.os.log(
                    LogLevel::Error,
                    LogCategory::Helper,
                    format!("cannot write the {client_name} config: {failure}"),
                );
                error(
                    HelperErrorCode::InternalError,
                    format!("could not create the {client_name} config file: {failure}"),
                )
            }
        })?;
        self.os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!(
                "run {} started for uid {uid} with {client_name} {}",
                request.attempt_id, client.version
            ),
        );
        self.run = Some(CurrentRun {
            attempt: request.attempt_id.clone(),
            owner_app: app,
            owner_uid: uid,
            run,
        });
        Ok(())
    }

    fn submit_otp(&self, app: u64, attempt: &str, code: String) -> Result<(), HelperError> {
        let mut code = Zeroizing::new(code);
        let uid = self.app_uid(app)?;
        let current = self.controllable_run(attempt, uid)?;
        current.run.submit_otp(mem::take(&mut *code))
    }

    fn disconnect(&self, app: u64, attempt: &str, reason: &str) -> Result<(), HelperError> {
        let uid = self.app_uid(app)?;
        let current = self.controllable_run(attempt, uid)?;
        let reason = reason.parse().unwrap_or(StopReason::UserDisconnect);
        let raw = serde_json::to_value(reason).expect("a stop reason serializes");
        let raw = raw
            .as_str()
            .expect("a stop reason serializes as its raw value");
        self.os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!("run {}: stop requested ({raw})", current.attempt),
        );
        current.run.stop(reason);
        Ok(())
    }

    /// The current run if `attempt` names it and `uid` owns it.
    fn controllable_run(&self, attempt: &str, uid: u32) -> Result<&CurrentRun<B>, HelperError> {
        let current = self
            .run
            .as_ref()
            .filter(|current| {
                attempt
                    .parse::<AttemptId>()
                    .is_ok_and(|id| id == current.attempt)
            })
            .ok_or_else(|| {
                error(
                    HelperErrorCode::UnknownAttempt,
                    "not the active connection attempt",
                )
            })?;
        if current.owner_uid != uid {
            return Err(error(
                HelperErrorCode::NotAuthorized,
                "the VPN run belongs to another user",
            ));
        }
        Ok(current)
    }

    fn status(&self) -> HelperStatus {
        let client_running = self
            .run
            .as_ref()
            .is_some_and(|current| current.run.process_id().is_some());
        match self.watchdog.find_client() {
            Ok((client, untested_version)) => HelperStatus {
                client_running,
                client_path: Some(client.path.display().to_string()),
                client_version: Some(client.version),
                client_problem: None,
                untested_version,
            },
            Err(problem) => HelperStatus {
                client_running,
                // Nothing was found where the client is looked for, so there is no path to show.
                client_path: (problem.kind != ClientProblemKind::NotFound {})
                    .then(|| problem.path.display().to_string()),
                client_version: None,
                client_problem: Some(problem),
                untested_version: None,
            },
        }
    }

    fn exit_when_idle(&mut self) -> bool {
        if self.run.is_some() {
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

    fn run_report(&mut self, report: Report) {
        let current = self
            .run
            .as_ref()
            .expect("reports come from the current run");
        let app = current.owner_app;
        match report {
            Report::Output(lines) => {
                let json = to_json(&lines);
                self.os.send(&FromEntry::SendOutput { app, json });
            }
            Report::Event(kind) => {
                let event = HelperEvent {
                    attempt_id: current.attempt.clone(),
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
                    } => self.os.log(
                        LogLevel::Notice,
                        LogCategory::Helper,
                        format!("run {}: tunnel up", event.attempt_id),
                    ),
                    HelperEventKind::Milestone { .. } => {}
                    HelperEventKind::Exited { facts } => self.run_exited(&facts),
                }
            }
            Report::RepairRoutes(repair) => {
                let gateway = repair.gateway;
                self.route_repair = Some(repair);
                self.os.send(&FromEntry::RepairRoutes { gateway });
            }
            Report::Log(level, category, message) => self.os.log(level, category, message),
        }
    }

    fn run_exited(&mut self, facts: &ExitFacts) {
        let finished = self.run.take().expect("the current run exited");
        self.os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!(
                "run {} exited: {:?}, initiator {:?}, furthest {:?}",
                finished.attempt, facts.exit_status, facts.stop_initiator, facts.furthest_milestone
            ),
        );
        match self.lifecycle {
            Lifecycle::ShuttingDown { .. } => self.exit(FromEntry::ExitAfterDraining {}),
            Lifecycle::Serving if self.executable_replaced => self.exit_for_replacement(),
            Lifecycle::Serving | Lifecycle::ExitingWhenIdle { .. } | Lifecycle::Exiting => {}
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
