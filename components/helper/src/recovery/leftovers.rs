//! Cleaning up after an earlier Helper: if it died during a run, the network settings it changed
//! outside the tunnel are still in place and its sessions are still open at the gateway; and the
//! exception routes it added may still be in the routing table.

use std::path::Path;

use fvm_contract::helper_internal::{AddedRoute, LogCategory, LogLevel, OutsideChanges};

use crate::recovery::logout::{self, SessionSecret};
use crate::recovery::run_directory;
use crate::secrets::SecretStore;

pub(crate) fn clean_up(
    secrets: &dyn SecretStore,
    directory: &Path,
    log: impl Fn(LogLevel, LogCategory, String),
    mut revert_network: impl FnMut(OutsideChanges),
    remove_bypass_routes: impl FnOnce(Vec<AddedRoute>) -> Vec<AddedRoute>,
) -> Vec<AddedRoute> {
    if let Err(failure) = run_directory::prepare(directory) {
        log(
            LogLevel::Error,
            LogCategory::Helper,
            format!("cannot prepare the run directory: {failure}"),
        );
        log_out_sessions(secrets, &log);
        return Vec::new();
    }
    for (path, record) in run_directory::read_records(directory) {
        match record {
            Ok(record) => {
                if let Some(changes) = record.outside_changes {
                    revert_network(changes);
                }
            }
            Err(_) => log(
                LogLevel::Error,
                LogCategory::Helper,
                format!("ignoring unreadable run record {}", path.display()),
            ),
        }
        if let Err(line) = run_directory::remove(&path) {
            log(LogLevel::Error, LogCategory::Helper, line);
        }
    }
    log_out_sessions(secrets, &log);
    let recorded = match run_directory::read_bypass_record(directory) {
        None => return Vec::new(),
        Some(Ok(routes)) => routes,
        Some(Err(_)) => {
            log(
                LogLevel::Error,
                LogCategory::Helper,
                format!(
                    "ignoring unreadable exception route record {}",
                    run_directory::bypass_record_path(directory).display()
                ),
            );
            Vec::new()
        }
    };
    let left = if recorded.is_empty() {
        Vec::new()
    } else {
        remove_bypass_routes(recorded)
    };
    if let Err(failure) = run_directory::write_bypass_record(directory, &left) {
        log(
            LogLevel::Error,
            LogCategory::Helper,
            format!("cannot record the exception routes: {failure}"),
        );
    }
    left
}

fn log_out_sessions(secrets: &dyn SecretStore, log: &impl Fn(LogLevel, LogCategory, String)) {
    let stored = match secrets.load_all() {
        Ok(stored) => stored,
        Err(error) => {
            log(
                LogLevel::Error,
                LogCategory::Helper,
                format!("cannot list the sessions an earlier helper left: {error}"),
            );
            return;
        }
    };
    for (name, secret) in stored {
        match secret.and_then(|bytes| SessionSecret::decode(&bytes)) {
            Ok(secret) => log(
                LogLevel::Notice,
                LogCategory::Helper,
                format!(
                    "session {name} an earlier helper left: {}",
                    logout::log_out(&secret)
                ),
            ),
            Err(error) => log(
                LogLevel::Error,
                LogCategory::Helper,
                format!("cannot read the session {name} an earlier helper left: {error}"),
            ),
        }
        if let Err(error) = secrets.delete(&name) {
            log(
                LogLevel::Error,
                LogCategory::Helper,
                format!("cannot delete the session {name}: {error}"),
            );
        }
    }
}
