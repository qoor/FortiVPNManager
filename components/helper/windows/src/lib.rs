//! Helper service (Windows): the Windows Helper, a service of the system account written in Rust
//! alone. It runs the shared Helper entry with a watchdog for openconnect, and does itself what
//! the Helper's Swift side does on macOS: it takes the app's connections over a named pipe,
//! answers the build check, repairs routes and changes the exception routes as the entry asks.
#![cfg(windows)]

pub mod os_api;
mod pipe;
mod routes;
mod service;

use std::sync::Arc;

use fvm_contract::helper_internal::{LogCategory, LogLevel};

pub use pipe::{Exit, Helper, Options};
pub use service::run;

/// This build, which the app compares with its own. `packaging/windows/build.ps1` gives the app the
/// build that the Helper was built with.
pub const BUILD: &str = match option_env!("FVM_BUILD") {
    Some(build) => build,
    None => "unknown",
};

/// Where the Helper's log lines go: the event log in the service. Never with a secret.
pub type Log = Arc<dyn Fn(LogLevel, LogCategory, &str) + Send + Sync>;
