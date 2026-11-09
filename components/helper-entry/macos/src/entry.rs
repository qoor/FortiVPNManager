//! The Helper entry's event loop. Swift's messages, the current run's reports and gateway lookups
//! arrive on one channel and are handled one at a time on one thread. So what reaches an app keeps
//! the order in which it happened, and the run thread, which only forwards, never waits for this one.

use std::collections::HashMap;
use std::env;
use std::io::BufReader;
use std::mem;
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::app_helper::{
    AttemptId, ExitFacts, HelperError, HelperErrorCode, HelperEvent, HelperEventKind, HelperStatus,
    Milestone, ProcessExitStatus, RunRequest, StopInitiator, StopReason,
};
use fvm_contract::backend::{Backend, ClientProblemKind};
use fvm_contract::helper_internal::{AppRequest, FromEntry, LogCategory, LogLevel, ToEntry};
use fvm_contract::stream;
use fvm_watchdog::{Report, RouteRepair, Run, Timing};
use serde::Serialize;
use zeroize::{Zeroize, Zeroizing};

use crate::leftovers;
use crate::processes::ProcessIdentity;
use crate::run_directory::{self, RunRecord};

/// How long exiting after SIGTERM waits for the run to report its exit. Longer than the client's
/// grace period, so a client that ignores SIGTERM is killed and reported first.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(15);
/// Lets the reply to an exitWhenIdle request leave before the Helper exits.
const EXIT_WHEN_IDLE_DELAY: Duration = Duration::from_millis(500);

pub(crate) enum Event {
    Swift(ToEntry),
    Run(Report),
    GatewayResolved { run: u64, gateway: Option<Ipv4Addr> },
}

/// The writing side of the stream to Swift.
pub(crate) struct Swift(UnixStream);

impl Swift {
    pub fn send(&self, message: &FromEntry) {
        stream::send(&mut &self.0, message).expect("writing a message to Swift");
    }

    /// Never with a secret: the line goes to the unified log.
    pub fn log(&self, level: LogLevel, category: LogCategory, message: String) {
        self.send(&FromEntry::Log {
            level,
            category,
            message,
        });
    }
}

pub(crate) fn serve<B>(backend: B, run_directory: PathBuf, socket: UnixStream)
where
    B: Backend,
    B::Interpreter: 'static,
{
    let (sender, events) = mpsc::channel();
    let reader = socket.try_clone().expect("cloning the socket to Swift");
    let forwarded = sender.clone();
    thread::Builder::new()
        .name("fvm-helper-entry-read".into())
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            loop {
                let message = stream::receive(&mut reader).expect("reading a message from Swift");
                forwarded
                    .send(Event::Swift(message))
                    .expect("the event loop runs as long as the process");
            }
        })
        .expect("starting the Helper entry's reader thread");

    let swift = Swift(socket);
    leftovers::clean_up(&backend, &run_directory, &swift, &events);
    swift.send(&FromEntry::Ready {});
    Entry {
        backend,
        run_directory,
        swift,
        events,
        sender,
        apps: HashMap::new(),
        run: None,
        runs_started: 0,
        executable_replaced: false,
        lifecycle: Lifecycle::Serving,
        route_repair: None,
    }
    .run()
}

struct Entry<B> {
    backend: B,
    run_directory: PathBuf,
    swift: Swift,
    events: Receiver<Event>,
    /// For the run's reports and the gateway lookups.
    sender: Sender<Event>,
    /// Admitted app connections that are still open, with their uid.
    apps: HashMap<u64, u32>,
    run: Option<CurrentRun>,
    runs_started: u64,
    executable_replaced: bool,
    lifecycle: Lifecycle,
    /// The current run's route repair while Swift carries it out.
    route_repair: Option<RouteRepair>,
}

/// The one run at a time and who owns it.
struct CurrentRun {
    /// Tells this run's gateway lookup apart from an earlier run's.
    number: u64,
    attempt: AttemptId,
    owner_app: u64,
    owner_uid: u32,
    run: Run,
    config_path: PathBuf,
    record: RunRecord,
}

enum Lifecycle {
    Serving,
    /// Answered an exitWhenIdle request with true; tells Swift to exit at `at`.
    ExitingWhenIdle {
        at: Instant,
    },
    /// SIGTERM came during a run; exiting once the run reports its exit, or at `deadline` anyway.
    ShuttingDown {
        deadline: Instant,
    },
    /// Told Swift to exit.
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
    B: Backend,
    B::Interpreter: 'static,
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
            Event::Swift(ToEntry::AppConnected { app, uid }) => self.admit(app, uid),
            Event::Swift(ToEntry::AppDisconnected { app }) => self.app_disconnected(app),
            Event::Swift(ToEntry::Request { id, app, request }) => self.request(id, app, request),
            Event::Swift(ToEntry::ExecutableReplaced {}) => self.executable_replaced(),
            Event::Swift(ToEntry::TerminationRequested {}) => self.termination_requested(),
            Event::Swift(ToEntry::RoutesRepaired { outcome }) => self
                .route_repair
                .take()
                .expect("Swift answers only the route repair it was asked for")
                .finish(outcome),
            Event::Run(report) => self.run_report(report),
            Event::GatewayResolved { run, gateway } => self.gateway_resolved(run, gateway),
        }
    }

    fn admit(&mut self, app: u64, uid: u32) {
        let accept = matches!(self.lifecycle, Lifecycle::Serving);
        if accept {
            self.apps.insert(app, uid);
        }
        self.swift.send(&FromEntry::Admitted { app, accept });
    }

    fn app_disconnected(&mut self, app: u64) {
        self.apps.remove(&app);
        if let Some(current) = self.run.as_ref().filter(|run| run.owner_app == app) {
            self.swift.log(
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
        self.swift.log(
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
            // SIGTERM may come again; the first one decided already.
            Lifecycle::ShuttingDown { .. } | Lifecycle::Exiting => {}
        }
    }

    fn deadline_passed(&mut self) {
        match self.lifecycle {
            Lifecycle::ExitingWhenIdle { .. } => self.exit(FromEntry::ExitAfterDraining {}),
            Lifecycle::ShuttingDown { .. } => {
                self.swift.log(
                    LogLevel::Error,
                    LogCategory::Helper,
                    "the run did not finish after SIGTERM; exiting anyway".to_owned(),
                );
                self.exit(FromEntry::ExitNow {});
            }
            Lifecycle::Serving | Lifecycle::Exiting => {
                unreachable!("only a lifecycle with a deadline passes it")
            }
        }
    }

    fn exit(&mut self, message: FromEntry) {
        self.swift.send(&message);
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
                self.swift.send(&FromEntry::Reply { id, json });
                return;
            }
            AppRequest::ExitWhenIdle {} => {
                let value = self.exit_when_idle();
                self.swift.send(&FromEntry::ReplyFlag { id, value });
                return;
            }
        };
        let json = result.err().map(|error| to_json(&error));
        self.swift.send(&FromEntry::Reply { id, json });
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
        let client_name = self.backend.client_name();
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
        let client = self.backend.locate_client().map_err(|problem| {
            error(
                HelperErrorCode::ClientUnavailable,
                self.backend.describe_problem(&problem),
            )
        })?;
        let number = self.runs_started + 1;
        let config_path = run_directory::config_path(&self.run_directory, number);
        let launch = self
            .backend
            .launch_spec(&client, request, &config_path)
            .map_err(|reason| error(HelperErrorCode::InvalidRequest, reason))?;
        if let Err(failure) = run_directory::prepare(&self.run_directory)
            .and_then(|()| run_directory::write_config(&config_path, &launch.config_file))
        {
            self.swift.log(
                LogLevel::Error,
                LogCategory::Helper,
                format!("cannot write the {client_name} config: {failure}"),
            );
            return Err(error(
                HelperErrorCode::InternalError,
                format!("could not create the {client_name} config file: {failure}"),
            ));
        }

        self.runs_started = number;
        let reports = self.sender.clone();
        let run = Run::start(
            &self.backend,
            &launch,
            mem::take(&mut request.password),
            Timing::default(),
            move |report| {
                reports
                    .send(Event::Run(report))
                    .expect("the event loop runs as long as the process");
            },
        );
        self.swift.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!(
                "run {} started for uid {uid} with {client_name} {}",
                request.attempt_id,
                client.version.as_deref().unwrap_or("unknown")
            ),
        );
        self.resolve_gateway(number, request.host.clone());
        self.run = Some(CurrentRun {
            number,
            attempt: request.attempt_id.clone(),
            owner_app: app,
            owner_uid: uid,
            run,
            config_path,
            record: RunRecord::default(),
        });
        Ok(())
    }

    fn submit_otp(&self, app: u64, attempt: &str, code: String) -> Result<(), HelperError> {
        let mut code = Zeroizing::new(code);
        let uid = self.app_uid(app)?;
        let current = self.controllable_run(attempt, uid)?;
        self.backend
            .check_otp(&code)
            .map_err(|reason| error(HelperErrorCode::InvalidRequest, reason))?;
        current.run.submit_otp(mem::take(&mut *code))
    }

    /// An empty `attempt` means whatever runs.
    fn disconnect(&self, app: u64, attempt: &str, reason: &str) -> Result<(), HelperError> {
        let uid = self.app_uid(app)?;
        if attempt.is_empty() && self.run.is_none() {
            return Ok(());
        }
        let current = self.controllable_run(attempt, uid)?;
        let reason = reason.parse().unwrap_or(StopReason::UserDisconnect);
        let raw = serde_json::to_value(reason).expect("a stop reason serializes");
        let raw = raw
            .as_str()
            .expect("a stop reason serializes as its raw value");
        self.swift.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!("run {}: stop requested ({raw})", current.attempt),
        );
        current.run.stop(reason);
        Ok(())
    }

    /// The current run if `attempt` names it or is empty, and `uid` owns it.
    fn controllable_run(&self, attempt: &str, uid: u32) -> Result<&CurrentRun, HelperError> {
        let current = self
            .run
            .as_ref()
            .filter(|current| {
                attempt.is_empty()
                    || attempt
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
        let (client_path, client_version, client_problem) = match self.backend.locate_client() {
            Ok(client) => (
                Some(client.path.display().to_string()),
                client.version,
                None,
            ),
            Err(problem) => (
                // Nothing was found where the client is looked for, so there is no path to show.
                (problem.kind != ClientProblemKind::NotFound {})
                    .then(|| problem.path.display().to_string()),
                None,
                Some(self.backend.describe_problem(&problem)),
            ),
        };
        HelperStatus {
            client_running: self
                .run
                .as_ref()
                .is_some_and(|current| current.run.process_id().is_some()),
            client_path,
            client_version,
            client_problem,
        }
    }

    fn exit_when_idle(&mut self) -> bool {
        if self.run.is_some() {
            return false;
        }
        if matches!(self.lifecycle, Lifecycle::Serving) {
            self.swift.log(
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

    /// The gateway's IPv4 address, for the pidfile and the route repair. Looked up while the run
    /// starts, since DNS may not answer once a broken tunnel took the routes; off this thread, since
    /// a slow answer must not hold up the app's requests. std cannot ask for IPv4 alone, so only the
    /// first IPv4 answer counts, as in the Swift Helper's IPv4-only lookup.
    fn resolve_gateway(&self, run: u64, host: String) {
        let events = self.sender.clone();
        thread::Builder::new()
            .name("fvm-helper-entry-gateway".into())
            .spawn(move || {
                let gateway =
                    (host.as_str(), 0)
                        .to_socket_addrs()
                        .ok()
                        .and_then(|mut addresses| {
                            addresses.find_map(|address| match address {
                                SocketAddr::V4(address) => Some(*address.ip()),
                                SocketAddr::V6(_) => None,
                            })
                        });
                events
                    .send(Event::GatewayResolved { run, gateway })
                    .expect("the event loop runs as long as the process");
            })
            .expect("starting the gateway lookup thread");
    }

    fn gateway_resolved(&mut self, run: u64, gateway: Option<Ipv4Addr>) {
        let Some(gateway) = gateway else {
            self.swift.log(
                LogLevel::Info,
                LogCategory::Routes,
                "gateway has no IPv4 address; its host route will not be repaired".to_owned(),
            );
            return;
        };
        let Some(current) = self.run.as_mut().filter(|current| current.number == run) else {
            return;
        };
        current.record.gateway = Some(gateway);
        if !current.record.processes.is_empty() {
            self.save_record();
        }
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
                self.swift.send(&FromEntry::SendOutput { app, json });
            }
            Report::Event(kind) => {
                let event = HelperEvent {
                    attempt_id: current.attempt.clone(),
                    kind,
                };
                self.swift.send(&FromEntry::SendEvent {
                    app,
                    json: to_json(&event),
                });
                match event.kind {
                    HelperEventKind::Milestone {
                        milestone: Milestone::Spawned,
                        ..
                    } => self.client_spawned(),
                    HelperEventKind::Milestone {
                        milestone: Milestone::TunnelUp,
                        ..
                    } => self.tunnel_up(),
                    HelperEventKind::Milestone { .. } => {}
                    HelperEventKind::Exited { facts } => self.run_exited(&facts),
                }
            }
            Report::RepairRoutes(repair) => {
                let gateway = current.record.gateway;
                self.route_repair = Some(repair);
                self.swift.send(&FromEntry::RepairRoutes { gateway });
            }
        }
    }

    /// Records the client so that a Helper started after this one died can stop it.
    fn client_spawned(&mut self) {
        let client_name = self.backend.client_name();
        let current = self.run.as_mut().expect("a run spawned its client");
        let Some(pid) = current.run.process_id() else {
            return;
        };
        let pid = i32::try_from(pid).expect("a pid fits in pid_t");
        let Some(client) = ProcessIdentity::of_root_process(pid) else {
            return;
        };
        current.record.processes = vec![client];
        let message = format!(
            "run {}: {client_name} running as pid {pid}",
            current.attempt
        );
        self.save_record();
        self.swift
            .log(LogLevel::Notice, LogCategory::Helper, message);
    }

    /// Adds what the client started for the tunnel, such as pppd, to the record.
    fn tunnel_up(&mut self) {
        let current = self.run.as_mut().expect("a run brought the tunnel up");
        let message = format!("run {}: tunnel up", current.attempt);
        if let Some(pid) = current.run.process_id() {
            for child in ProcessIdentity::root_children(pid) {
                if self.backend.is_own_executable(Path::new(&child.path))
                    && !current.record.processes.contains(&child)
                {
                    current.record.processes.push(child);
                }
            }
            self.save_record();
        }
        self.swift
            .log(LogLevel::Notice, LogCategory::Helper, message);
    }

    fn save_record(&self) {
        let current = self
            .run
            .as_ref()
            .expect("a record belongs to the current run");
        if let Err(failure) = run_directory::write_record(&self.run_directory, &current.record) {
            self.swift.log(
                LogLevel::Error,
                LogCategory::Helper,
                format!("cannot write the pidfile: {failure}"),
            );
        }
    }

    fn run_exited(&mut self, facts: &ExitFacts) {
        let finished = self.run.take().expect("the current run exited");
        leftovers::remove(&finished.config_path, &self.swift);
        leftovers::remove(
            &run_directory::record_path(&self.run_directory),
            &self.swift,
        );
        self.swift.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!(
                "run {} exited: {}, initiator {}, furthest {}",
                finished.attempt,
                swift_exit_status(facts.exit_status),
                swift_initiator(facts.stop_initiator),
                swift_furthest(facts.furthest_milestone)
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

// The run's end in the unified log, worded as the Swift Helper's description of its types so that
// searches through older logs keep matching.

fn swift_exit_status(status: ProcessExitStatus) -> String {
    match status {
        ProcessExitStatus::Exited { code } => format!("exited(code: {code})"),
        ProcessExitStatus::Signaled { signal } => format!("signaled(signal: {signal})"),
        ProcessExitStatus::SpawnFailed { errno } => format!("spawnFailed(errno: {errno})"),
        ProcessExitStatus::Unknown {} => "unknown".to_owned(),
    }
}

fn swift_initiator(initiator: StopInitiator) -> String {
    match initiator {
        StopInitiator::Client {} => "none".to_owned(),
        StopInitiator::Helper { reason } => {
            let raw = serde_json::to_value(reason).expect("a stop reason serializes");
            let raw = raw
                .as_str()
                .expect("a stop reason serializes as its raw value");
            format!("helper(FortiShared.StopReason.{raw})")
        }
    }
}

fn swift_furthest(milestone: Option<Milestone>) -> String {
    let Some(milestone) = milestone else {
        return "nil".to_owned();
    };
    let case = match milestone {
        Milestone::Spawned => "spawned",
        Milestone::ConnectedToGateway => "connectedToGateway",
        Milestone::OtpPrompted => "otpPrompted",
        Milestone::OtpSubmitted => "otpSubmitted",
        Milestone::Authenticated => "authenticated",
        Milestone::TunnelUp => "tunnelUp",
    };
    format!("Optional(FortiShared.Milestone.{case})")
}
