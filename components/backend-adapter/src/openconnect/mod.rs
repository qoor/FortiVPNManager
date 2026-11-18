//! openconnect 9.21 with its fortinet protocol; on Windows with the Wintun driver its installer
//! brings. Its options, prompts and output wording are only known here, and are the same on every
//! OS; what differs per OS is in `platform`. It takes everything as arguments, asks for the
//! password only after it verified the gateway, and runs its own route script. An attempt logs in
//! and opens the tunnel in separate runs, so the Helper holds the session cookie and can log out
//! when it has to end the tunnel by force.

mod client;
mod network;
mod output;
mod platform;
mod request;

use std::path::Path;
use std::time::Duration;

use fvm_contract::app_helper::RunRequest;
use fvm_contract::backend::{
    Backend, ClientCheckWait, ClientExecutable, ClientProblem, LaunchSpec, NetworkSettings,
    OutputInterpreter, UntestedVersion,
};
use fvm_contract::validation;

pub use output::Interpreter;

pub struct OpenConnect;

impl Backend for OpenConnect {
    fn client_name(&self) -> &'static str {
        "openconnect"
    }

    fn locate_client(&self, wait: ClientCheckWait) -> Result<ClientExecutable, ClientProblem> {
        client::locate(wait)
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
        values_path: &Path,
    ) -> Result<LaunchSpec, String> {
        request::launch_spec(client, request, values_path)
    }

    fn network_settings(&self, values: &[u8]) -> Result<NetworkSettings, String> {
        network::settings(values)
    }

    fn check_otp(&self, code: &str) -> Result<(), String> {
        validation::check_otp(code)
    }

    fn interpreter(&self) -> Box<dyn OutputInterpreter + Send> {
        Box::new(Interpreter::default())
    }

    /// Where the OS has a stop request (SIGTERM), openconnect logs out before it ends. Windows has
    /// none, so there it is killed at once.
    fn stop_grace_period(&self) -> Duration {
        Duration::from_secs(10)
    }

    fn is_own_executable(&self, path: &Path) -> bool {
        client::is_own_executable(path)
    }
}
