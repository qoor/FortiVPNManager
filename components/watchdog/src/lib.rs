//! The Helper's VPN client, from finding it to what is left of it: checking the client executable,
//! starting a run with the launch and config file the backend adapter makes, watching it until it
//! ends (spawning, stdin input, output lines, stage time limits, stopping, the exit report),
//! recording its processes so that a later Helper can stop what this one leaves, and stopping what
//! an earlier one left. Everything that depends on the client, its name, options, prompts, output
//! wording and how it is stopped, comes from the `Backend` the Helper's OS start chose; nothing
//! else in the Helper uses it.
//!
//! On Unix the process this runs in must ignore SIGPIPE. The password and the OTP go to the client's stdin
//! pipe, and a write after the client exited must fail with EPIPE instead of killing the process.
//! macOS can switch SIGPIPE off for a single pipe only through an fcntl that no safe Rust API
//! offers. A Rust `main` ignores SIGPIPE by itself; the macOS Helper's Swift `main` does it
//! explicitly.

mod leftovers;
mod lines;
mod run;
mod run_directory;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as os;
#[cfg(windows)]
use windows as os;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use fvm_contract::app_helper::{HelperEventKind, Milestone, OutputLine, RunRequest};
use fvm_contract::backend::{
    Backend, ClientExecutable, ClientProblem, RouteRepairOutcome, UntestedVersion,
};
use fvm_contract::helper_internal::{LogCategory, LogLevel};

pub use run::Run;
pub use run_directory::Failure;

use run_directory::RunFiles;

/// The Helper's VPN client: the one `B` describes, with its runs' files in a directory only the
/// Helper may use.
pub struct Watchdog<B> {
    backend: Arc<B>,
    run_directory: PathBuf,
    timing: Timing,
    /// Numbers the runs' config files.
    runs_started: u64,
}

impl<B> Watchdog<B>
where
    B: Backend + Send + Sync + 'static,
{
    pub fn new(backend: B, run_directory: PathBuf, timing: Timing) -> Self {
        Self {
            backend: Arc::new(backend),
            run_directory,
            timing,
            runs_started: 0,
        }
    }

    /// The client's name as the Helper's English errors and log lines show it.
    pub fn client_name(&self) -> &'static str {
        self.backend.client_name()
    }

    /// The client executable the Helper would run now, with whether its version is outside the
    /// ones this app was tested with; or why there is none.
    pub fn find_client(
        &self,
    ) -> Result<(ClientExecutable, Option<UntestedVersion>), ClientProblem> {
        let client = self.backend.locate_client()?;
        let untested = self.backend.untested_version(&client.version);
        Ok((client, untested))
    }

    /// Starts a run for `request`, which the backend adapter checks again, since the Helper does
    /// not trust the app. A started run takes the request's password. Returns the run with the
    /// client it runs; `on_report` gets what the run tells, as `Run` describes.
    pub fn start(
        &mut self,
        request: &mut RunRequest,
        on_report: impl FnMut(Report) + Send + 'static,
    ) -> Result<(Run<B>, ClientExecutable), StartFailure> {
        let client = self.backend.locate_client().map_err(|problem| {
            StartFailure::ClientUnavailable(self.backend.describe_problem(&problem))
        })?;
        let number = self.runs_started + 1;
        let files = RunFiles::new(&self.run_directory, number);
        let launch = self
            .backend
            .launch_spec(&client, request, &files.config)
            .map_err(StartFailure::InvalidRequest)?;
        // The run directory also holds the pidfile, so it is prepared without a config file too.
        os::prepare_run_directory(&self.run_directory)
            .and_then(|()| {
                launch
                    .config_file
                    .as_ref()
                    .map_or(Ok(()), |contents| files.write_config(contents))
            })
            .map_err(StartFailure::RunFiles)?;
        self.runs_started = number;
        let run = Run::start(
            Arc::clone(&self.backend),
            &launch,
            request,
            files,
            self.timing,
            on_report,
        );
        Ok((run, client))
    }
}

/// Why `Watchdog::start` started no run.
#[derive(Debug)]
pub enum StartFailure {
    /// The client executable cannot be used; the adapter's English description for the app.
    ClientUnavailable(String),
    /// The adapter refused a value of the request; its English reason for the app.
    InvalidRequest(String),
    /// The run directory or the run's config file could not be written.
    RunFiles(Failure),
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
    /// A line for the Helper's own log, which the app never sees.
    Log(LogLevel, LogCategory, String),
}

/// Finishes the route repair a run asked for. The run waits for it, so dropping it unfinished
/// panics.
#[must_use = "the run reports its exit only after the route repair is finished"]
pub struct RouteRepair {
    /// The gateway the client connected to, if its IPv4 address is known.
    pub gateway: Option<Ipv4Addr>,
    answer: Option<Sender<run::Input>>,
}

impl RouteRepair {
    pub fn finish(mut self, outcome: RouteRepairOutcome) {
        self.answer
            .take()
            .expect("a route repair is finished once")
            .send(run::Input::RoutesRepaired(outcome))
            .expect("the run waits for the route repair");
    }
}

impl Drop for RouteRepair {
    fn drop(&mut self) {
        if self.answer.is_some() && !thread::panicking() {
            panic!("a route repair was dropped unfinished; the run would never report its exit");
        }
    }
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
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            stages: StageLimits::default(),
            output_batch: Duration::from_millis(100),
            line_idle_flush: Duration::from_millis(250),
            pipe_drain: Duration::from_secs(1),
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
