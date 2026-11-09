//! Homebrew's openfortivpn 1.24.1, which starts Apple's pppd. Its option set, prompts and output
//! wording are only known here.

mod client;
mod output;
mod request;

use std::path::Path;
use std::time::Duration;

use fvm_contract::app_helper::RunRequest;
use fvm_contract::backend::{
    Backend, ClientExecutable, ClientProblem, LaunchSpec, StopPolicy, StopScope, UntestedVersion,
};

pub use output::Interpreter;

pub struct OpenFortiVpn;

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
