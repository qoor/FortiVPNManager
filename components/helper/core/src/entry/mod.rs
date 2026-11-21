mod bypass_routes;
mod runs;
#[cfg(test)]
mod update_tests;
mod updates;

use std::collections::{HashMap, HashSet};
use std::env;
use std::panic;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::app_helper::{
    AttemptId, BackendStatus, HelperError, HelperErrorCode, HelperStatus, StopReason,
};
use fvm_contract::helper_internal::{
    AddedRoute, AppRequest, FromEntry, LogCategory, LogLevel, NetworkChange, NetworkOutcome,
    NetworkStep, OutsideChanges, ToEntry,
};
use serde::Serialize;

use crate::auto_update::{self, Installer};
use crate::recovery::RecordSlot;
use crate::session::{Report, Run, SessionManager};
use crate::watchdog::{self, Beat, Fault};

use bypass_routes::{BypassRoutes, Job};
use updates::serve_updates;

/// How long exiting after the OS's termination request waits for the run to report its exit.
/// Longer than a stopped session's wait before it is detached, so a run whose logout hangs is still
/// detached and reported first.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(15);
const EXIT_WHEN_IDLE_LIMIT: Duration = Duration::from_secs(1);

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
    let installer = Installer::new(
        PathBuf::from(auto_update::RECORDS_DIRECTORY),
        auto_update::PUBLIC_KEY,
    );
    let (sender, events) = mpsc::channel();
    let inbox = Inbox(sender.clone());
    thread::Builder::new()
        .name("fvm-entry".into())
        .spawn(move || {
            serve(sessions, installer, OsSide(Box::new(send)), sender, events);
        })
        .expect("starting the entry thread");
    inbox
}

pub fn report_faults(fault: impl Fn(&str) + Send + Sync + 'static) {
    let fault: Fault = Arc::new(fault);
    watchdog::report_to(Arc::clone(&fault));
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let thread = thread::current();
        let location = info.location().expect("std gives every panic a location");
        let mut line = format!(
            "thread '{}' panicked at {location}",
            thread.name().unwrap_or("<unnamed>")
        );
        if let Some(literal) = info.payload().downcast_ref::<&'static str>() {
            line.push_str(": ");
            line.push_str(literal);
        }
        fault(&line);
        default(info);
        #[cfg(windows)]
        end_for_restart();
    }));
}

#[cfg(windows)]
pub const RESTART_EXIT_CODE: u32 = 4;

#[cfg(unix)]
pub fn end_for_restart() -> ! {
    std::process::abort()
}

#[cfg(windows)]
pub fn end_for_restart() -> ! {
    fvm_os_api_windows::process::terminate_this_process(RESTART_EXIT_CODE)
}

enum Event {
    FromOs(ToEntry),
    Run(AttemptId, Report),
    Updated {
        id: u64,
        log: Option<(LogLevel, String)>,
        json: String,
    },
    UpdateLog(LogLevel, String),
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

fn serve(
    sessions: SessionManager,
    installer: Installer,
    os: OsSide,
    sender: Sender<Event>,
    events: Receiver<Event>,
) {
    let (left, unreverted) = clean_up(&sessions, &os, &events);
    let beat = sessions.watch("entry".to_owned());
    os.send(&FromEntry::Ready {});
    let (updates, requests) = mpsc::channel();
    let replies = sender.clone();
    thread::Builder::new()
        .name("fvm-update".into())
        .spawn(move || serve_updates(installer, &requests, &replies))
        .expect("starting the update thread");
    Entry {
        sessions,
        beat,
        os,
        events,
        sender,
        apps: HashMap::new(),
        runs: HashMap::new(),
        executable_replaced: false,
        lifecycle: Lifecycle::Serving,
        bypass_routes: BypassRoutes::new(left),
        unreverted,
        updates,
    }
    .run()
}

/// Has the session manager clean up after an earlier Helper, with the OS side reverting the network
/// settings a run that did not finish changed and those a revert left, and removing the exception
/// routes the earlier Helper added, before any app connection is accepted. Returns the exception
/// routes that could not be removed and the network settings still left. Until "ready", the OS side
/// accepts no app connection and passes on no stop request, so the answers to these requests are
/// the only messages it sends.
fn clean_up(
    sessions: &SessionManager,
    os: &OsSide,
    events: &Receiver<Event>,
) -> (Vec<AddedRoute>, Vec<OutsideChanges>) {
    let revert = |changes: OutsideChanges| {
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
        outcome
    };
    sessions.clean_up(
        |level, category, message| os.log(level, category, message),
        |changes| {
            let routes = changes.routes.len();
            let dns_service_id = changes.dns_service_id.clone();
            let outcome = revert(changes);
            let (level, message) = match &outcome {
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
            left_after_revert(&dns_service_id, &outcome)
        },
        |kept| {
            kept.into_iter()
                .filter_map(|changes| {
                    let outcome = revert(changes.clone());
                    reverted_again(os, &changes, &outcome)
                })
                .collect()
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
    beat: Beat,
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
    unreverted: Vec<OutsideChanges>,
    updates: Sender<(u64, AppRequest)>,
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
    ExitingWhenIdle {
        asking: HashSet<u64>,
        deadline: Instant,
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
            Self::ExitingWhenIdle { deadline, .. } | Self::ShuttingDown { deadline } => {
                Some(deadline)
            }
            Self::Serving | Self::Exiting => None,
        }
    }
}

impl Entry {
    fn run(mut self) -> ! {
        loop {
            self.beat.beat();
            let period = self.beat.period();
            let wait = self.lifecycle.deadline().map_or(period, |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(period)
            });
            let event = match self.events.recv_timeout(wait) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => unreachable!("the entry holds a sender"),
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
            Event::Updated { id, log, json } => {
                if let Some((level, line)) = log {
                    self.os.log(level, LogCategory::Helper, line);
                }
                self.os.send(&FromEntry::Reply {
                    id,
                    json: Some(json),
                });
            }
            Event::UpdateLog(level, line) => self.os.log(level, LogCategory::Helper, line),
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
        if let Lifecycle::ExitingWhenIdle { asking, .. } = &mut self.lifecycle {
            asking.remove(&app);
            self.exit_when_asking_apps_are_gone();
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
            Lifecycle::ExitingWhenIdle { .. } => {
                self.os.log(
                    LogLevel::Notice,
                    LogCategory::Helper,
                    "an app that asked the Helper to exit kept its connection open; exiting anyway"
                        .to_owned(),
                );
                self.exit(FromEntry::ExitAfterDraining {});
            }
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
                if self.runs.is_empty() {
                    self.reply_status(id, None);
                } else {
                    self.queue_bypass_job(Job::Status { id });
                }
                return;
            }
            AppRequest::ExitWhenIdle {} => {
                let value = self.exit_when_idle(app);
                self.os.send(&FromEntry::ReplyFlag { id, value });
                self.exit_when_asking_apps_are_gone();
                return;
            }
            AppRequest::ReadBypassRoutes {} => return self.read_bypass_routes(app, id),
            AppRequest::WriteBypassRoutes { json } => {
                return self.write_bypass_routes(app, id, &json);
            }
            update @ (AppRequest::PrepareUpdate { .. }
            | AppRequest::InstallUpdate { .. }
            | AppRequest::ReadPreparedUpdate {}
            | AppRequest::DiscardPreparedUpdate {}) => {
                self.updates
                    .send((id, update))
                    .expect("the update thread runs as long as the process");
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

    fn reply_status(&self, id: u64, routing_complete: Option<bool>) {
        let status = HelperStatus {
            session_running: self.runs.values().any(|held| held.run.session_running()),
            backend: BackendStatus {
                version: Some(self.sessions.version().to_owned()),
            },
            routing_complete,
        };
        let json = Some(to_json(&status));
        self.os.send(&FromEntry::Reply { id, json });
    }

    fn exit_when_idle(&mut self, app: u64) -> bool {
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
                asking: HashSet::new(),
                deadline: Instant::now() + EXIT_WHEN_IDLE_LIMIT,
            };
        }
        if let Lifecycle::ExitingWhenIdle { asking, .. } = &mut self.lifecycle
            && self.apps.contains_key(&app)
        {
            asking.insert(app);
        }
        true
    }

    fn exit_when_asking_apps_are_gone(&mut self) {
        if let Lifecycle::ExitingWhenIdle { asking, .. } = &self.lifecycle
            && asking.is_empty()
        {
            self.exit(FromEntry::ExitAfterDraining {});
        }
    }
}

fn left_after_revert(dns_service_id: &str, outcome: &NetworkOutcome) -> Option<OutsideChanges> {
    let (dns_left, routes) = match outcome {
        NetworkOutcome::Reverted { left } => (false, left),
        NetworkOutcome::Failed { step, left, .. } => (*step != NetworkStep::OutsideRoutes, left),
        other => panic!("the OS side answered a network revert with {other:?}"),
    };
    let left = OutsideChanges {
        dns_service_id: if dns_left {
            dns_service_id.to_owned()
        } else {
            String::new()
        },
        routes: routes.clone(),
    };
    (!left.dns_service_id.is_empty() || !left.routes.is_empty()).then_some(left)
}

fn reverted_again(
    os: &OsSide,
    changes: &OutsideChanges,
    outcome: &NetworkOutcome,
) -> Option<OutsideChanges> {
    let left = left_after_revert(&changes.dns_service_id, outcome);
    let routes = changes.routes.len();
    match (&left, outcome) {
        (None, _) => os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!("reverted again the network settings a revert left ({routes} route(s))"),
        ),
        (Some(left), NetworkOutcome::Failed { step, error, .. }) => os.log(
            LogLevel::Error,
            LogCategory::Helper,
            format!(
                "reverting again the network settings a revert left failed ({step:?}): {error}; {} of {routes} route(s) still in place",
                left.routes.len()
            ),
        ),
        (Some(left), _) => os.log(
            LogLevel::Error,
            LogCategory::Helper,
            format!(
                "reverting again the network settings a revert left: {} of {routes} route(s) still in place",
                left.routes.len()
            ),
        ),
    }
    left
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

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::process::Command;

    use super::*;

    fn after_file<'a>(stdout: &'a str, thread: &str) -> &'a str {
        let prefix = format!("thread '{thread}' panicked at {}:", file!());
        let line = stdout
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("{stdout}"));
        &line[prefix.len()..]
    }

    fn assert_row_and_column(site: &str) {
        assert!(
            matches!(site.split_once(':'), Some((row, column))
                if row.parse::<u32>().is_ok() && column.parse::<u32>().is_ok()),
            "{site}"
        );
    }

    fn panicking_child_output(test: &str) -> String {
        let child = Command::new(env::current_exe().unwrap())
            .args(["--exact", test, "--ignored"])
            .output()
            .unwrap();
        let stdout = String::from_utf8(child.stdout).unwrap();
        #[cfg(unix)]
        assert!(child.status.success(), "{} {stdout}", child.status);
        #[cfg(windows)]
        assert_eq!(
            child.status.code(),
            Some(RESTART_EXIT_CODE.cast_signed()),
            "{stdout}"
        );
        stdout
    }

    #[test]
    fn a_panic_is_reported_with_its_thread_and_location_and_only_a_literal_message() {
        let stdout = panicking_child_output("entry::tests::panic_with_a_literal");
        let literal = after_file(&stdout, "fvm-literal");
        let (site, message) = literal
            .split_once(": ")
            .unwrap_or_else(|| panic!("{literal}"));
        assert_row_and_column(site);
        assert_eq!(message, "on purpose");
        let stdout = panicking_child_output("entry::tests::panic_with_a_formatted_message");
        assert_row_and_column(after_file(&stdout, "fvm-formatted"));
    }

    fn panic_on_a_named_thread(name: &str, panicking: fn()) {
        report_faults(|line| {
            io::stdout()
                .write_all(format!("{line}\n").as_bytes())
                .unwrap();
        });
        let panicked = thread::Builder::new()
            .name(name.into())
            .spawn(panicking)
            .unwrap()
            .join();
        assert!(panicked.is_err());
    }

    #[test]
    #[ignore = "a panicking child of a_panic_is_reported_with_its_thread_and_location_and_only_a_literal_message"]
    fn panic_with_a_literal() {
        panic_on_a_named_thread("fvm-literal", || panic!("on purpose"));
    }

    #[test]
    #[ignore = "a panicking child of a_panic_is_reported_with_its_thread_and_location_and_only_a_literal_message"]
    fn panic_with_a_formatted_message() {
        panic_on_a_named_thread("fvm-formatted", || {
            panic!("on purpose with {}", String::from("secret"))
        });
    }
}
