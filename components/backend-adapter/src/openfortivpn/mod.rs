//! Homebrew's openfortivpn 1.24.1, which starts Apple's pppd. Its option set, prompts and output
//! wording are only known here. Finding and running it is Unix only.

mod account;
#[cfg(unix)]
mod client;
mod output;
#[cfg(unix)]
mod request;
mod settings;

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use fvm_contract::app_helper::RunRequest;
use fvm_contract::app_internal::problem::AccountValueProblem;
use fvm_contract::app_internal::section::AccountSaveError;
#[cfg(unix)]
use fvm_contract::backend::{
    Backend, ClientExecutable, ClientProblem, LaunchSpec, UntestedVersion,
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

    /// `text` as a gateway certificate value openfortivpn compares, a SHA-256 digest in 64
    /// lowercase hex characters, or `None` when it is no digest.
    pub fn certificate_value(&self, text: &str) -> Option<String> {
        account::certificate_value(text)
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
        account::check_account(host, username, realm)
    }

    /// Why openfortivpn would refuse a typed VPN password: a line break or NUL, or too many
    /// bytes. An empty one passes, as it keeps the stored password.
    pub fn check_vpn_password(&self, password: &str) -> Result<(), AccountSaveError> {
        account::check_vpn_password(password)
    }
}

#[cfg(unix)]
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
        crate::values::check_otp(code)
    }

    fn interpreter(&self) -> Interpreter {
        Interpreter::default()
    }

    fn stop_grace_period(&self) -> Duration {
        Duration::from_secs(10)
    }

    fn is_own_executable(&self, path: &Path) -> bool {
        client::is_own_executable(path)
    }
}
