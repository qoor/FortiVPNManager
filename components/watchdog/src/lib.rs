//! Runs one VPN client process and watches it until it ends: spawning, stdin input, output lines,
//! stage time limits, stopping, and the exit report. Everything that depends on the client, its
//! name, prompts, output wording and how it is stopped, comes from the caller's `Backend`.
//!
//! The process this runs in must ignore SIGPIPE. The password and the OTP go to the client's stdin
//! pipe, and a write after the client exited must fail with EPIPE instead of killing the process.
//! macOS can switch SIGPIPE off for a single pipe only through an fcntl that no safe Rust API
//! offers. A Rust `main` ignores SIGPIPE by itself; the macOS Helper's Swift `main` does it
//! explicitly.

mod lines;
mod run;
#[cfg(unix)]
mod unix;

#[cfg(unix)]
use unix as os;

use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use fvm_contract::app_helper::{HelperEventKind, Milestone, OutputLine};
use fvm_contract::backend::RouteRepairOutcome;

pub use run::Run;

/// What a run tells its caller, in order. Lines are reported before the event they led to.
pub enum Report {
    /// Log lines, at least one.
    Output(Vec<OutputLine>),
    /// A milestone, or `Exited`, the run's last report.
    Event(HelperEventKind),
    /// The client exited without restoring the routes it changed. The run reports its exit once
    /// the repair is finished.
    RepairRoutes(RouteRepair),
}

/// Finishes the route repair a run asked for. The run waits for it, so dropping it unfinished
/// panics.
#[must_use = "the run reports its exit only after the route repair is finished"]
pub struct RouteRepair(Option<Sender<run::Input>>);

impl RouteRepair {
    pub fn finish(mut self, outcome: RouteRepairOutcome) {
        self.0
            .take()
            .expect("a route repair is finished once")
            .send(run::Input::RoutesRepaired(outcome))
            .expect("the run waits for the route repair");
    }
}

impl Drop for RouteRepair {
    fn drop(&mut self) {
        if self.0.is_some() && !thread::panicking() {
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
