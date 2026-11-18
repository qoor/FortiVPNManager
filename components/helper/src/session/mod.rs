//! The Helper's VPN client: checking the client executable, starting a run with the launch and
//! config file the backend adapter makes, and watching it until it ends (spawning, stdin input,
//! output lines, stage time limits, stopping, the logout, the exit report). The run record its
//! processes go into, and stopping what an earlier Helper left, are `recovery`'s. Everything that
//! depends on the client, its name, options, prompts, output wording and how it is stopped, comes
//! from the client's `Backend`, which the Helper's OS start hands over; only this module and
//! `recovery`, which gets it from here, use it.
//!
//! On Unix the process this runs in must ignore SIGPIPE. The password and the OTP go to the client's stdin
//! pipe, and a write after the client exited must fail with EPIPE instead of killing the process.
//! macOS can switch SIGPIPE off for a single pipe only through an fcntl that no safe Rust API
//! offers. A Rust `main` ignores SIGPIPE by itself; the macOS Helper's Swift `main` does it
//! explicitly.

mod lines;
mod run;
#[cfg(unix)]
pub(crate) mod unix;
#[cfg(windows)]
pub(crate) mod windows;

#[cfg(unix)]
pub(crate) use unix as os;
#[cfg(windows)]
pub(crate) use windows as os;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use fvm_contract::app_helper::{HelperEventKind, Milestone, OutputLine, RunRequest};
use fvm_contract::backend::{
    Backend, ClientCheckWait, ClientExecutable, ClientProblem, UntestedVersion,
};
use fvm_contract::helper_internal::{
    AddedRoute, LogCategory, LogLevel, NetworkChange, NetworkOutcome, OutsideChanges,
};

pub use run::{NotReplaceable, Replaced, Run};

use crate::recovery::run_directory::RunFiles;
use crate::recovery::{self, Failure, RecordSlot, RouteRepair};

/// The VPN client the Helper runs, with the adapter that describes it, and its runs' files in a
/// directory only the Helper may use.
pub struct SessionManager {
    backend: Arc<dyn Backend>,
    run_directory: PathBuf,
    timing: Timing,
    /// Numbers the runs' config files.
    runs_started: u64,
}

pub type FoundClient = Result<(ClientExecutable, Option<UntestedVersion>), ClientProblem>;

impl SessionManager {
    pub fn new(backend: Arc<dyn Backend>, run_directory: PathBuf, timing: Timing) -> Self {
        Self {
            backend,
            run_directory,
            timing,
            runs_started: 0,
        }
    }

    /// The client executable the Helper would run now. Where finding it runs the client, such as
    /// to read its version, `wait` says how long that run may take.
    pub fn find_client(&self, wait: ClientCheckWait) -> FoundClient {
        self.backend.locate_client(wait).map(|executable| {
            let untested = self.backend.untested_version(&executable.version);
            (executable, untested)
        })
    }

    /// Starts a run for `request`, which the backend adapter checks again, since the Helper does
    /// not trust the app. A started run takes the request's password. Returns the run with the
    /// client it runs; `on_report` gets what the run tells, as `Run` describes.
    pub fn start(
        &mut self,
        request: &mut RunRequest,
        slot: RecordSlot,
        replaced: Option<Replaced>,
        on_report: impl FnMut(Report) + Send + 'static,
    ) -> Result<(Run, ClientExecutable), StartFailure> {
        let backend = Arc::clone(&self.backend);
        let client = backend
            .locate_client(ClientCheckWait::Limited)
            .map_err(|problem| {
                StartFailure::ClientUnavailable(backend.describe_problem(&problem))
            })?;
        let number = self.runs_started + 1;
        let files = RunFiles::new(&self.run_directory, number, slot);
        let launch = backend
            .launch_spec(&client, request, &files.config, &files.values)
            .map_err(StartFailure::InvalidRequest)?;
        let writes_values = launch.script_runner.is_some()
            || launch
                .session
                .as_ref()
                .is_some_and(|session| session.tunnel.script_runner.is_some());
        // The run directory also holds the pidfile, so it is prepared without a config file too.
        os::prepare_run_directory(&self.run_directory)
            .and_then(|()| {
                if writes_values {
                    os::prepare_run_directory(&files.values_directory)
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                launch
                    .config_file
                    .as_ref()
                    .map_or(Ok(()), |contents| files.write_config(contents))
            })
            .map_err(|failure| StartFailure::RunFiles {
                client_name: backend.client_name(),
                failure,
            })?;
        self.runs_started = number;
        let run = Run::start(
            backend,
            &client,
            &launch,
            request,
            files,
            replaced,
            self.timing,
            on_report,
        );
        Ok((run, client))
    }

    /// Runs `recovery::clean_up` with this client and run directory, which `entry` cannot reach.
    pub fn clean_up(
        &self,
        log: impl Fn(LogLevel, LogCategory, String),
        revert_network: impl FnMut(OutsideChanges),
        repair_routes: impl FnMut(Option<Ipv4Addr>),
        remove_bypass_routes: impl FnOnce(Vec<AddedRoute>) -> Vec<AddedRoute>,
    ) -> Vec<AddedRoute> {
        recovery::clean_up(
            self.backend.as_ref(),
            &self.run_directory,
            log,
            revert_network,
            repair_routes,
            remove_bypass_routes,
        )
    }

    /// Records in the run directory the exception routes the Helper added, so that a later Helper
    /// removes them if this one cannot; none removes the record.
    pub fn record_bypass_routes(&self, routes: &[AddedRoute]) -> Result<(), Failure> {
        recovery::write_bypass_record(&self.run_directory, routes)
    }
}

/// Why `SessionManager::start` started no run.
#[derive(Debug)]
pub enum StartFailure {
    /// The client executable cannot be used; the adapter's English description for the app.
    ClientUnavailable(String),
    /// The adapter refused a value of the request; its English reason for the app.
    InvalidRequest(String),
    /// The run directory or the run's config file could not be written.
    RunFiles {
        client_name: &'static str,
        failure: Failure,
    },
}

/// What a run tells its caller, in order. Lines are reported before the event they led to.
pub enum Report {
    /// Log lines for the app, at least one.
    Output(Vec<OutputLine>),
    /// A milestone, or `Exited`, the run's last report.
    Event(HelperEventKind),
    /// The client exited without restoring the routes it changed. The run reports its exit once
    /// the repair is finished.
    RepairRoutes(RouteRepair),
    ChangeNetwork(NetworkJob),
    /// A line for the Helper's own log, which the app never sees.
    Log(LogLevel, LogCategory, String),
}

/// The time values of a run. The defaults are the Helper's.
#[derive(Clone, Copy)]
pub struct Timing {
    pub stages: StageLimits,
    /// Lines go out in batches at most this far apart, and before every event.
    pub output_batch: Duration,
    /// An unfinished line, such as a prompt, is shown after this much silence on its stream.
    pub line_idle_flush: Duration,
    /// How long to wait for the end of the output after the client exited: a process it started,
    /// such as pppd, may still hold a pipe.
    pub pipe_drain: Duration,
    pub network_values: Duration,
    pub network_retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            stages: StageLimits::default(),
            output_batch: Duration::from_millis(100),
            line_idle_flush: Duration::from_millis(250),
            pipe_drain: Duration::from_secs(1),
            network_values: Duration::from_secs(10),
            network_retry: Duration::from_secs(1),
        }
    }
}

#[must_use = "the run waits for the answer to its network change"]
pub struct NetworkJob {
    pub change: NetworkChange,
    answer: Option<Box<dyn FnOnce(NetworkOutcome) + Send>>,
}

impl NetworkJob {
    fn new(change: NetworkChange, answer: impl FnOnce(NetworkOutcome) + Send + 'static) -> Self {
        Self {
            change,
            answer: Some(Box::new(answer)),
        }
    }

    pub fn finish(mut self, outcome: NetworkOutcome) {
        let answer = self
            .answer
            .take()
            .expect("a network change is answered once");
        answer(outcome);
    }
}

impl Drop for NetworkJob {
    fn drop(&mut self) {
        if self.answer.is_some() && !thread::panicking() {
            panic!("a network change was dropped unanswered; the run would wait for it forever");
        }
    }
}

/// How long each stage before the tunnel is up may take. A stage starts at the milestone that
/// begins it; one that takes longer stops the client.
#[derive(Clone, Copy)]
pub struct StageLimits {
    /// Spawned until connected to the gateway.
    pub spawn_to_gateway: Duration,
    /// Connected to the gateway until the OTP prompt or authenticated.
    pub gateway_to_prompt_or_auth: Duration,
    /// OTP prompt until the OTP is submitted. The app's own OTP limit is shorter; this one is a
    /// backstop.
    pub otp_prompt_to_submit: Duration,
    /// OTP submitted until authenticated.
    pub otp_submit_to_auth: Duration,
    /// Authenticated until the tunnel is up.
    pub auth_to_tunnel: Duration,
}

impl Default for StageLimits {
    fn default() -> Self {
        Self {
            spawn_to_gateway: Duration::from_secs(30),
            gateway_to_prompt_or_auth: Duration::from_secs(30),
            otp_prompt_to_submit: Duration::from_secs(65),
            otp_submit_to_auth: Duration::from_secs(20),
            auth_to_tunnel: Duration::from_secs(40),
        }
    }
}

impl StageLimits {
    /// The stage that runs once `milestone` is the furthest one, with the name the Log tab shows
    /// for it; `None` once the tunnel is up.
    fn after(&self, milestone: Milestone) -> Option<(&'static str, Duration)> {
        match milestone {
            Milestone::Spawned => Some(("spawnToGateway", self.spawn_to_gateway)),
            Milestone::ConnectedToGateway => {
                Some(("gatewayToPromptOrAuth", self.gateway_to_prompt_or_auth))
            }
            Milestone::OtpPrompted => Some(("otpPromptToSubmit", self.otp_prompt_to_submit)),
            Milestone::OtpSubmitted => Some(("otpSubmitToAuth", self.otp_submit_to_auth)),
            Milestone::Authenticated => Some(("authToTunnel", self.auth_to_tunnel)),
            Milestone::TunnelUp => None,
        }
    }
}
