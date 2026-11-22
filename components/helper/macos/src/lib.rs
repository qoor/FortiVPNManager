//! The macOS Helper: a root `LaunchDaemon`, demand-launched through its mach service. It runs the
//! shared Helper entry with a session manager for openconnect, keeping the session secrets in the
//! System keychain, takes the app's connections over NSXPC, answers the build check, and changes
//! the network settings and the exception routes as the entry asks.

mod command;
mod network;
mod relay;
mod route_table;
mod routes;
mod secrets;
mod self_update;
mod xpc;

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::{process, thread};

use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_contract::helper_internal::LogCategory;
use fvm_helper_core::entry;
use fvm_helper_core::session::{SessionManager, Timing};
use fvm_os_api_macos::code_signing;
use fvm_os_api_macos::log::{Level, Log};
use signal_hook::consts::SIGTERM;
use signal_hook::iterator::Signals;

use relay::Relay;
use secrets::Keychain;
use self_update::Watcher;

/// This build, which the app compares with its own.
const BUILD: &str = match option_env!("FVM_BUILD") {
    Some(build) => build,
    None => "unknown",
};

const APP_ID: &str = "com.qoor.FortiVPNManager";
const HELPER_ID: &str = "com.qoor.FortiVPNManager.Helper";
const LOG_SUBSYSTEM: &str = "com.qoor.FortiVPNManager";
/// Root-only: it holds the run record.
const RUN_DIRECTORY: &str = "/var/run/com.qoor.FortiVPNManager.Helper";

static HELPER_LOG: LazyLock<Log> = LazyLock::new(|| Log::new(LOG_SUBSYSTEM, "helper"));
static ROUTES_LOG: LazyLock<Log> = LazyLock::new(|| Log::new(LOG_SUBSYSTEM, "routes"));

/// A line for the unified log. Never with a password, an OTP code or a config file's contents.
fn log(category: LogCategory, level: Level, message: &str) {
    let log = match category {
        LogCategory::Helper => &HELPER_LOG,
        LogCategory::Routes => &ROUTES_LOG,
    };
    log.write(level, message);
}

/// Runs the Helper until the system ends it.
///
/// # Panics
///
/// Panics if the SIGTERM handler or its thread cannot be set up.
pub fn run() -> ! {
    let Some(team_id) = code_signing::team_id() else {
        log(
            LogCategory::Helper,
            Level::Fault,
            "helper is not signed with a team identity; refusing to start",
        );
        process::exit(1);
    };
    // Installed before the entry starts: a SIGTERM during its cleanup is passed on once it is ready.
    let mut terminations = Signals::new([SIGTERM]).expect("installing the SIGTERM handler");
    // Created before the entry starts, so a reinstall during its cleanup counts as a replacement.
    let watcher = Arc::new(Watcher::new());
    entry::report_faults(|line| log(LogCategory::Helper, Level::Fault, line));
    let sessions = SessionManager::new(
        Arc::new(OpenConnect::default()),
        Arc::new(Keychain),
        PathBuf::from(RUN_DIRECTORY),
        Timing::default(),
    );
    let relay = Relay::start(sessions);
    let replaced = Arc::clone(&relay);
    Arc::clone(&watcher).start(move || replaced.executable_replaced());
    let terminated = Arc::clone(&relay);
    thread::Builder::new()
        .name("fvm-helper-sigterm".into())
        .spawn(move || {
            for _ in terminations.forever() {
                terminated.termination_requested();
            }
        })
        .expect("starting the SIGTERM thread");
    let _listener = xpc::listen(relay, watcher, &team_id);
    log(
        LogCategory::Helper,
        Level::Notice,
        &format!("helper {BUILD} listening (team {team_id})"),
    );
    loop {
        thread::park();
    }
}
