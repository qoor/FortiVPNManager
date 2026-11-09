//! Homebrew's openfortivpn 1.24.1, which starts Apple's pppd. Its option set, prompts and output
//! wording are only known here.

mod client;
mod output;
mod request;
mod settings;

use std::path::{Path, PathBuf};
use std::time::Duration;

use fvm_contract::app_helper::RunRequest;
use fvm_contract::app_internal::problem::AccountValueProblem;
use fvm_contract::app_internal::section::AccountSaveError;
use fvm_contract::backend::{
    Backend, ClientExecutable, ClientProblem, LaunchSpec, StopPolicy, StopScope, UntestedVersion,
};

pub use output::Interpreter;
pub use settings::ImportedSettings;

pub struct OpenFortiVpn;

/// What the app's runtime uses: importing an existing config file, the certificate value format
/// and the account checks.
impl OpenFortiVpn {
    /// The config files openfortivpn reads by default, in the order to try; `home` is the user's
    /// home folder.
    pub fn default_settings_files(&self, home: &Path) -> Vec<PathBuf> {
        settings::default_files(home)
    }

    /// Reads an openfortivpn config file's contents. Never fails: what cannot be used is reported.
    pub fn read_settings(&self, contents: &[u8]) -> ImportedSettings {
        settings::read(contents)
    }

    /// Whether `value` is a gateway certificate value openfortivpn compares: a SHA-256 digest in
    /// 64 lowercase hex characters.
    pub fn is_certificate_value(&self, value: &str) -> bool {
        request::is_certificate_value(value)
    }

    /// Why openfortivpn would refuse the account's server address, username or realm, the first
    /// of them, by the rules the Helper applies when connecting. An empty server address or
    /// username passes, as an account not set up yet; an empty realm means none.
    pub fn check_account(
        &self,
        host: &str,
        username: &str,
        realm: &str,
    ) -> Result<(), AccountValueProblem> {
        request::check_account(host, username, realm)
    }

    /// Why openfortivpn would refuse a typed VPN password: a line break or NUL, or too many
    /// bytes. An empty one passes, as it keeps the stored password.
    pub fn check_vpn_password(&self, password: &str) -> Result<(), AccountSaveError> {
        request::check_vpn_password(password)
    }
}

impl Backend for OpenFortiVpn {
    type Interpreter = Interpreter;

    fn client_name(&self) -> &'static str {
        "openfortivpn"
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

    fn launch_spec(
        &self,
        client: &ClientExecutable,
        request: &RunRequest,
        config_path: &Path,
    ) -> Result<LaunchSpec, String> {
        request::launch_spec(client, request, config_path)
    }

    fn check_otp(&self, code: &str) -> Result<(), String> {
        request::check_otp(code)
    }

    fn interpreter(&self) -> Interpreter {
        Interpreter::default()
    }

    fn stop_policy(&self) -> StopPolicy {
        StopPolicy {
            // Its children, such as /sbin/route restoring the routes, must be left to finish.
            graceful: StopScope::Process,
            grace_period: Duration::from_secs(10),
            forced: StopScope::ProcessThenGroup,
        }
    }

    fn is_own_executable(&self, path: &Path) -> bool {
        client::is_own_executable(path)
    }
}
