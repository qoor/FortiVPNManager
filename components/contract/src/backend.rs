//! What a backend adapter provides for one VPN client. The Helper's OS start hands the watchdog the
//! adapter of every client its OS can run, and the watchdog runs the client each run request names
//! with what its adapter returns, so neither hard-codes a client's name, options or output wording.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::app_helper::{Milestone, OutputSource, RunRequest};

pub trait Backend: Send + Sync {
    /// The client's name as the Helper's English errors and Log lines show it, such as
    /// "<name> is not running".
    fn client_name(&self) -> &'static str;

    /// Finds the client executable the Helper may run as root. Where finding it runs the client,
    /// such as to read its version, `wait` says how long that run may take.
    fn locate_client(&self, wait: ClientCheckWait) -> Result<ClientExecutable, ClientProblem>;

    /// English description of `problem` that the current app shows as it is.
    fn describe_problem(&self, problem: &ClientProblem) -> String;

    /// Whether `version`, as `locate_client` found it, is outside the versions this app was tested
    /// with. The Helper reports it in its status, so the app warns without knowing the rule.
    fn untested_version(&self, version: &str) -> Option<UntestedVersion>;

    /// Checks every value of `request` again, since the Helper does not trust the app, and builds
    /// the launch. If the launch has a config file, the caller writes it to `config_path` with mode
    /// 0600 before the launch and removes it after the run. The error is an English reason for the
    /// app.
    fn launch_spec(
        &self,
        client: &ClientExecutable,
        request: &RunRequest,
        config_path: &Path,
        values_path: &Path,
    ) -> Result<LaunchSpec, String>;

    fn network_settings(&self, values: &[u8]) -> Result<NetworkSettings, String>;

    /// Checks an OTP from the app before it goes to the client's stdin. The error is an English
    /// reason for the app.
    fn check_otp(&self, code: &str) -> Result<(), String>;

    /// A fresh interpreter for one run.
    fn interpreter(&self) -> Box<dyn OutputInterpreter + Send>;

    /// How long the client may take to end after the OS's stop request before it is killed.
    fn stop_grace_period(&self) -> Duration;

    /// Whether an executable belongs to this client, so that a process of it left behind by an
    /// earlier Helper may be stopped.
    fn is_own_executable(&self, path: &Path) -> bool;
}

/// How long `Backend::locate_client` waits for a run of the client.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClientCheckWait {
    /// Up to the adapter's time limit; a client still running then counts as not executable.
    Limited,
    /// Until the client exits.
    UntilExit,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ClientExecutable {
    /// Fully resolved, without symlinks.
    pub path: PathBuf,
    pub version: String,
}

/// The client's version is outside the series this app was tested with.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UntestedVersion {
    /// Such as `1.24`.
    pub tested_series: String,
}

/// Why the client executable cannot be used, as a kind and the path it concerns; wording is left
/// to whoever shows it. The Helper reports it in its status, and the app's UI receives it in the
/// Helper's section (`app_internal`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ClientProblem {
    pub kind: ClientProblemKind,
    pub path: PathBuf,
}

/// Cases without data are empty structs so the JSON matches Swift's enum coding.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
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
    /// Never holds the password, which goes to stdin. `None` for a client that takes everything
    /// as arguments.
    pub config_file: Option<String>,
    /// For a client that logs in and opens the tunnel in separate runs, what follows this login
    /// run; `None` for a client that does both in one run.
    pub session: Option<Box<SessionLaunch>>,
    pub script_runner: Option<PathBuf>,
}

/// The rest of an attempt whose login run printed a session (`Finding::Session`). Neither has a
/// config file or a session of its own.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SessionLaunch {
    /// Opens the tunnel with the session, which it reads from stdin.
    pub tunnel: LaunchSpec,
    /// Ends the session at the gateway, sent when the Helper ended the tunnel run by force or never
    /// started it, so the client could not log out itself. It reads the session from stdin; the
    /// first line of its stdout goes to the log.
    pub logout: LaunchSpec,
}

/// What the tunnel run and the logout request read on stdin, each holding the session cookie. Debug
/// output hides them, and they are zeroed when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionInputs {
    pub tunnel: Zeroizing<Vec<u8>>,
    pub logout: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for SessionInputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionInputs(<redacted>)")
    }
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
    /// The login run printed its session: the login succeeded. The line holds the session cookie,
    /// so it is not logged.
    Session(SessionInputs),
    /// A line that holds a secret but could not be read as one; it is not logged either.
    Hidden,
    ScriptLeftRunning,
    GatewayRouteRequested,
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

/// What became of the routes a client left unrestored, as the OS integration reports it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RouteRepairOutcome {
    Repaired,
    NotNeeded,
    Failed,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSettings {
    pub tunnel: TunnelDevice,
    pub gateway: IpAddr,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,
    pub ipv4: FamilySettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<FamilySettings>,
    pub split_exclude: Vec<IpNetwork>,
    pub dns_servers: Vec<IpAddr>,
    pub default_domains: Vec<String>,
    pub split_dns_domains: Vec<String>,
    pub wins_servers: Vec<Ipv4Addr>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct TunnelDevice {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct FamilySettings {
    pub address: IpNetwork,
    pub routes: TunnelRoutes,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TunnelRoutes {
    Full {},
    Split { include: Vec<IpNetwork> },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpNetwork {
    pub address: IpAddr,
    pub prefix_length: u8,
}
