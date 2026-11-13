//! Helper service (Windows): the Windows Helper, a service of the system account written in Rust
//! alone. It runs the shared Helper entry with a watchdog for openconnect, and does itself what
//! Swift's OS integration Helper does on macOS: it takes the app's connections over a named pipe,
//! answers the build check, repairs routes and reads and writes the bypass routes.
#![cfg(windows)]

mod command;
mod pipe;
mod routes;
mod service;

use std::sync::Arc;

use fvm_contract::helper_internal::{LogCategory, LogLevel};

pub use pipe::{Exit, Helper, Options};
pub use service::run;

/// This build, which the app compares with its own. The install script sets `FVM_BUILD` for both.
pub const BUILD: &str = match option_env!("FVM_BUILD") {
    Some(build) => build,
    None => "unknown",
};

/// Where the Helper's log lines go: the event log in the service. Never with a secret.
pub type Log = Arc<dyn Fn(LogLevel, LogCategory, &str) + Send + Sync>;
