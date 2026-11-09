//! What a backend adapter provides for one VPN client. The Helper entry picks the adapter and the
//! watchdog runs the client with what it returns, so neither hard-codes the client's name, options
//! or output wording.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::app_helper::{Milestone, OutputSource, RunRequest};

pub trait Backend {
    type Interpreter: OutputInterpreter + Send;

    /// The client's name as the Helper's English errors and Log lines show it, such as
    /// "<name> is not running".
    fn client_name(&self) -> &'static str;

    /// Finds the client executable the Helper may run as root.
    fn locate_client(&self) -> Result<ClientExecutable, ClientProblem>;

    /// English description of `problem` that the current app shows as it is.
    fn describe_problem(&self, problem: &ClientProblem) -> String;

    /// Checks every value of `request` again, since the Helper does not trust the app, and builds
    /// the launch. The caller writes `LaunchSpec::config_file` to `config_path` with mode 0600
    /// before the launch and removes it after the run. The error is an English reason for the app.
    fn launch_spec(
        &self,
        client: &ClientExecutable,
        request: &RunRequest,
        config_path: &Path,
    ) -> Result<LaunchSpec, String>;

    /// Checks an OTP from the app before it goes to the client's stdin. The error is an English
    /// reason for the app.
    fn check_otp(&self, code: &str) -> Result<(), String>;

    /// A fresh interpreter for one run.
    fn interpreter(&self) -> Self::Interpreter;

    fn stop_policy(&self) -> StopPolicy;

    /// Whether an executable belongs to this client, so that a process of it left behind by an
    /// earlier Helper may be stopped.
    fn is_own_executable(&self, path: &Path) -> bool;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ClientExecutable {
    /// Fully resolved, without symlinks.
    pub path: PathBuf,
    pub version: Option<String>,
}

/// Why the client executable cannot be used, as a kind and the path it concerns; wording is left
/// to whoever shows it. The app's UI receives it in the Helper's section (`app_internal`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct ClientProblem {
    pub kind: ClientProblemKind,
    pub path: PathBuf,
}

/// Cases without data are empty structs so the JSON matches Swift's enum coding.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientProblemKind {
    /// Nothing usable at `path`, where the client is looked for. The Helper status leaves the
    /// client path empty for it.
    NotFound {},
    /// `path`, where the client resolves to, is outside the place it may be installed.
    OutsideAllowedLocation {},
    NotRegularFile {},
    /// Other users may modify it; `mode` holds the permission bits.
    WritableByOthers {
        mode: u32,
    },
    NotExecutable {},
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LaunchSpec {
    pub program: PathBuf,
    /// Arguments after the program name.
    pub args: Vec<String>,
    /// The whole environment; nothing else is inherited.
    pub env: Vec<(String, String)>,
    /// Never holds the password, which goes to stdin.
    pub config_file: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prompt {
    Password,
    Otp,
}

/// What one complete line of client output tells.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finding {
    /// Only `ConnectedToGateway`, `Authenticated` and `TunnelUp` come from output.
    Milestone(Milestone),
    TunnelIp(Ipv4Addr),
    /// The client rejected the gateway certificate; trusting this value accepts it next time.
    /// Each distinct value is reported once, in first-seen order.
    CertificateRejected(String),
    /// The gateway refused tunnel mode after authentication.
    TunnelModeRejected,
}

/// Reads one run's output.
pub trait OutputInterpreter {
    /// Called with the bytes of `source` (stdout or stderr) after its last line break each time
    /// more arrive, because a prompt has no line break. Returns the prompt those bytes end with.
    fn prompt(&mut self, source: OutputSource, partial_line: &[u8]) -> Option<Prompt>;

    /// Whether the client may ask for the password once the run has reached `milestone`; clients
    /// differ in when they ask. The watchdog answers only the first password prompt, and only
    /// where this holds.
    fn password_expected(&self, milestone: Milestone) -> bool;

    fn line(&mut self, source: OutputSource, text: &str) -> Option<Finding>;

    /// After the client exited: whether it changed routes and did not finish restoring them.
    fn routes_left_unrestored(&self) -> bool;
}

/// How to stop the client: a graceful stop (SIGTERM), then after the grace period a forced one
/// (SIGKILL) if it still runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StopPolicy {
    pub graceful: StopScope,
    pub grace_period: Duration,
    pub forced: StopScope,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StopScope {
    Process,
    /// The client process, then its whole process group.
    ProcessThenGroup,
}
