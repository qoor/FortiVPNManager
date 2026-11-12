//! openconnect 9.21 with its fortinet protocol; on Windows with the Wintun driver its installer
//! brings. Its options, prompts and output wording are only known here, and are the same on every
//! OS; what differs per OS is in `platform`, where only Windows is supported so far. Unlike
//! openfortivpn it takes everything as arguments, asks for the password only after it verified the
//! gateway, and runs its own route script. An attempt logs in and opens the tunnel in separate runs,
//! so the Helper holds the session cookie and can log out when it has to end the tunnel by force.

mod client;
mod output;
mod platform;
mod request;

use std::path::Path;
use std::time::Duration;

use fvm_contract::app_helper::RunRequest;
use fvm_contract::app_internal::problem::AccountValueProblem;
use fvm_contract::app_internal::section::AccountSaveError;
use fvm_contract::backend::{
    Backend, ClientExecutable, ClientProblem, LaunchSpec, UntestedVersion,
};

pub use output::Interpreter;

pub struct OpenConnect;

/// What the app's runtime uses: the certificate value format and the account checks.
impl OpenConnect {
    /// `text` as a gateway certificate value openconnect compares, `pin-sha256:` and the base64
    /// of a SHA-256 hash, or `None` when it is no such value.
    pub fn certificate_value(&self, text: &str) -> Option<String> {
        request::certificate_value(text)
    }

    /// Why openconnect would refuse the account's server address, username or realm, the first of
    /// them, by the rules the Helper applies when connecting. An empty server address or username
    /// passes, as an account not set up yet; an empty realm means none.
    pub fn check_account(
        &self,
        host: &str,
        username: &str,
        realm: &str,
    ) -> Result<(), AccountValueProblem> {
        request::check_account(host, username, realm)
    }

    /// Why openconnect would refuse a typed VPN password: a character outside printable ASCII, or
    /// too many bytes. An empty one passes, as it keeps the stored password.
    pub fn check_vpn_password(&self, password: &str) -> Result<(), AccountSaveError> {
        request::check_vpn_password(password)
    }
}

impl Backend for OpenConnect {
    type Interpreter = Interpreter;

    fn client_name(&self) -> &'static str {
        "openconnect"
    }

    fn locate_client(&self) -> Result<ClientExecutable, ClientProblem> {
        client::locate()
    }

    fn describe_problem(&self, problem: &ClientProblem) -> String {
        client::describe(problem)
    }

    fn untested_version(&self, version: &str) -> Option<UntestedVersion> {
        client::untested_version(version)
    }

    /// openconnect has no config file, so `config_path` is not used.
    fn launch_spec(
        &self,
        client: &ClientExecutable,
        request: &RunRequest,
        _config_path: &Path,
    ) -> Result<LaunchSpec, String> {
        request::launch_spec(client, request)
    }

    fn check_otp(&self, code: &str) -> Result<(), String> {
        crate::values::check_otp(code)
    }

    fn interpreter(&self) -> Interpreter {
        Interpreter::default()
    }

    /// Where the OS has a stop request (SIGTERM), openconnect logs out and its route script
    /// restores the routes before it ends. Windows has none, so there it is killed at once.
    fn stop_grace_period(&self) -> Duration {
        Duration::from_secs(10)
    }

    fn is_own_executable(&self, path: &Path) -> bool {
        client::is_own_executable(path)
    }
}
