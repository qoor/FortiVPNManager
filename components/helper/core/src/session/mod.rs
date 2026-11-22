//! The Helper's VPN sessions: checking a run request, starting a run whose session the backend
//! runs on a thread of its own, and following it until it ends (the password and the OTP, its log
//! lines, the stage time limits, stopping it, the exit report). The run record and the
//! session secrets a run leaves for a later Helper, and cleaning up after an earlier one, are
//! `recovery`'s. Everything that depends on the VPN client comes from the `Backend` and the
//! `SecretStore` the Helper's OS start hands over.
//!
//! On Unix the process this runs in must ignore SIGPIPE. The logout request's cookie goes to curl's
//! stdin pipe, and a write after curl exited must fail with EPIPE instead of killing the process.
//! A Rust `main` ignores SIGPIPE by itself.

pub mod backend;
mod run;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use fvm_contract::app_helper::{HelperEventKind, Milestone, OutputLine, RunRequest};
use fvm_contract::helper_internal::{
    AddedRoute, LogCategory, LogLevel, NetworkChange, NetworkOutcome, OutsideChanges,
};

pub use run::{NotReplaceable, Replaced, Run};

use crate::recovery::run_directory::{self, RunFiles};
use crate::recovery::{self, Failure, RecordSlot};
use crate::secrets::SecretStore;
use crate::watchdog::{Beat, Watchdog};
use backend::Backend;

/// The VPN client the Helper runs, the store for its session secrets, and its runs' files in a
/// directory only the Helper may use.
pub struct SessionManager {
    backend: Arc<dyn Backend>,
    secrets: Arc<dyn SecretStore>,
    run_directory: PathBuf,
    timing: Timing,
    watchdog: Watchdog,
}

impl SessionManager {
    pub fn new(
        backend: Arc<dyn Backend>,
        secrets: Arc<dyn SecretStore>,
        run_directory: PathBuf,
        timing: Timing,
    ) -> Self {
        backend.init();
        Self {
            backend,
            secrets,
            run_directory,
            timing,
            watchdog: Watchdog::start(timing.beat, timing.stalled_after, timing.resume_gap),
        }
    }

    #[must_use]
    pub fn version(&self) -> &str {
        self.backend.version()
    }

    pub(crate) fn watch(&self, name: String) -> Beat {
        self.watchdog.watch(name)
    }

    /// Starts a run for `request`, which the backend checks again, since the Helper does not trust
    /// the app. A started run takes the request's password. `on_report` gets what the run tells,
    /// as `Run` describes.
    pub fn start(
        &self,
        request: &mut RunRequest,
        slot: RecordSlot,
        replaced: Option<Replaced>,
        start_after: Option<Receiver<()>>,
        on_report: impl FnMut(Report) + Send + 'static,
    ) -> Result<Run, StartFailure> {
        self.backend
            .check(request)
            .map_err(StartFailure::InvalidRequest)?;
        run_directory::prepare(&self.run_directory).map_err(StartFailure::RunDirectory)?;
        Ok(Run::start(
            Arc::clone(&self.backend),
            Arc::clone(&self.secrets),
            request,
            RunFiles::new(&self.run_directory, slot),
            replaced,
            start_after,
            &self.watchdog,
            self.timing,
            on_report,
        ))
    }

    /// Runs `recovery::clean_up` with this secret store and run directory, which `entry` cannot
    /// reach.
    pub fn clean_up(
        &self,
        log: impl Fn(LogLevel, LogCategory, String),
        revert_network: impl FnMut(OutsideChanges) -> Option<OutsideChanges>,
        revert_again: impl FnOnce(Vec<OutsideChanges>) -> Vec<OutsideChanges>,
        remove_bypass_routes: impl FnOnce(Vec<AddedRoute>) -> Vec<AddedRoute>,
    ) -> (Vec<AddedRoute>, Vec<OutsideChanges>) {
        recovery::clean_up(
            self.secrets.as_ref(),
            &self.run_directory,
            log,
            revert_network,
            revert_again,
            remove_bypass_routes,
        )
    }

    /// Records in the run directory the exception routes the Helper added, so that a later Helper
    /// removes them if this one cannot; none removes the record.
    pub fn record_bypass_routes(&self, routes: &[AddedRoute]) -> Result<(), Failure> {
        recovery::write_bypass_record(&self.run_directory, routes)
    }

    pub fn record_unreverted(&self, changes: &[OutsideChanges]) -> Result<(), Failure> {
        recovery::write_unreverted_record(&self.run_directory, changes)
    }
}

/// Why `SessionManager::start` started no run.
#[derive(Debug)]
pub enum StartFailure {
    /// The backend refused a value of the request; its English reason for the app.
    InvalidRequest(String),
    RunDirectory(Failure),
}

/// What a run tells its caller, in order. Lines are reported before the event they led to.
pub enum Report {
    /// Log lines for the app, at least one.
    Output(Vec<OutputLine>),
    /// A milestone, or `Exited`, the run's last report.
    Event(HelperEventKind),
    ChangeNetwork(NetworkJob),
    /// A line for the Helper's own log, which the app never sees.
    Log(LogLevel, LogCategory, String),
    Stuck,
}

/// The time values of a run and of the watchdog. The defaults are the Helper's.
#[derive(Clone, Copy)]
pub struct Timing {
    pub stages: StageLimits,
    /// Lines go out in batches at most this far apart, and before every event.
    pub output_batch: Duration,
    pub network_retry: Duration,
    pub detach_after: Duration,
    pub stuck_after: Duration,
    pub beat: Duration,
    pub stalled_after: Duration,
    pub resume_gap: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            stages: StageLimits::default(),
            output_batch: Duration::from_millis(100),
            network_retry: Duration::from_secs(1),
            detach_after: Duration::from_secs(10),
            stuck_after: Duration::from_secs(30),
            beat: Duration::from_secs(1),
            stalled_after: Duration::from_secs(30),
            resume_gap: Duration::from_secs(2),
        }
    }
}

#[must_use = "the run waits for the answer to its network change"]
pub struct NetworkJob {
    pub change: NetworkChange,
    returned: Arc<AtomicBool>,
    answer: Option<Box<dyn FnOnce(NetworkOutcome) + Send>>,
}

impl NetworkJob {
    fn new(
        change: NetworkChange,
        returned: Arc<AtomicBool>,
        answer: impl FnOnce(NetworkOutcome) + Send + 'static,
    ) -> Self {
        Self {
            change,
            returned,
            answer: Some(Box::new(answer)),
        }
    }

    #[must_use]
    pub fn session_running(&self) -> bool {
        !self.returned.load(Ordering::Relaxed)
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
        assert!(
            self.answer.is_none() || thread::panicking(),
            "a network change was dropped unanswered; the run would wait for it forever"
        );
    }
}

/// How long each stage before the tunnel is up may take. A stage starts at the milestone that
/// begins it; one that takes longer stops the session.
#[derive(Clone, Copy)]
pub struct StageLimits {
    /// Started until connected to the gateway.
    pub start_to_gateway: Duration,
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
            start_to_gateway: Duration::from_secs(30),
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
            Milestone::Started => Some(("startToGateway", self.start_to_gateway)),
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
