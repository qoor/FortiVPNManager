//! Cleaning up after an earlier Helper: if it died during a run, the client and the processes it
//! started may still run, and a client that died without restoring the routes left them broken; and
//! the exception routes it added may still be in the routing table.

use std::net::Ipv4Addr;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::backend::Backend;
use fvm_contract::helper_internal::{AddedRoute, LogCategory, LogLevel};

use crate::process::os;
use crate::recovery::run_directory::{self, ProcessIdentity, RunRecord};

/// How long the processes get to disappear after the forced stop.
const KILL_LIMIT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Cleans up after an earlier Helper in `directory`. If it died during a run of `backend`'s client,
/// stops the client processes its pidfile names that still run, calls `repair_routes` with the
/// run's gateway, if it is known, and removes the files it left once that returns. The routes are
/// repaired whenever a pidfile is left, even an unreadable one, since a run did not finish. Then
/// `remove_bypass_routes` gets the exception routes it recorded and answers those still in the
/// routing table, which stay recorded and are returned as this Helper's. `log` takes lines for the
/// Helper's log.
pub(crate) fn clean_up<B: Backend>(
    backend: &B,
    directory: &Path,
    log: impl Fn(LogLevel, LogCategory, String),
    repair_routes: impl FnOnce(Option<Ipv4Addr>),
    remove_bypass_routes: impl FnOnce(Vec<AddedRoute>) -> Vec<AddedRoute>,
) -> Vec<AddedRoute> {
    if let Err(failure) = os::prepare_run_directory(directory) {
        log(
            LogLevel::Error,
            LogCategory::Helper,
            format!("cannot prepare the run directory: {failure}"),
        );
        return Vec::new();
    }
    if let Some(record) = run_directory::read_record(directory) {
        // An unreadable pidfile still means a run did not finish, so the routes are checked.
        let record = record.unwrap_or_else(|_| {
            log(
                LogLevel::Error,
                LogCategory::Helper,
                format!(
                    "ignoring unreadable pidfile {}",
                    run_directory::record_path(directory).display()
                ),
            );
            RunRecord::default()
        });
        let orphans: Vec<ProcessIdentity> = record
            .processes
            .into_iter()
            .filter(|process| {
                backend.is_own_executable(Path::new(&process.path)) && process.is_running()
            })
            .collect();
        if !orphans.is_empty() {
            log(
                LogLevel::Notice,
                LogCategory::Helper,
                format!(
                    "stopping {} process(es) left by a previous helper: {:?}",
                    orphans.len(),
                    pids(&orphans)
                ),
            );
            terminate(&orphans, backend.stop_grace_period(), &log);
        }
        repair_routes(record.gateway);
    }
    for leftover in run_directory::leftovers(directory) {
        if let Err(line) = run_directory::remove(&leftover) {
            log(LogLevel::Error, LogCategory::Helper, line);
        }
    }
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

/// The OS's stop request first, so the client can restore the routes, and its forced stop after
/// `grace_period`.
fn terminate(
    processes: &[ProcessIdentity],
    grace_period: Duration,
    log: &impl Fn(LogLevel, LogCategory, String),
) {
    stop(processes, os::request_stop);
    let survivors = wait_for_exit(processes, grace_period);
    if survivors.is_empty() {
        return;
    }
    log(
        LogLevel::Notice,
        LogCategory::Helper,
        format!("sending {} to {:?}", os::FORCE_STOP, pids(&survivors)),
    );
    stop(&survivors, os::force_stop);
    let stuck = wait_for_exit(&survivors, KILL_LIMIT);
    if !stuck.is_empty() {
        log(
            LogLevel::Error,
            LogCategory::Helper,
            format!(
                "processes still running after {}: {:?}",
                os::FORCE_STOP,
                pids(&stuck)
            ),
        );
    }
}

/// Checks each identity right before stopping it, since its pid may have been reused meanwhile.
fn stop(processes: &[ProcessIdentity], how: fn(&ProcessIdentity)) {
    for process in processes.iter().filter(|process| process.is_running()) {
        how(process);
    }
}

/// The processes still running once all are gone or `limit` passed.
fn wait_for_exit(processes: &[ProcessIdentity], limit: Duration) -> Vec<ProcessIdentity> {
    let deadline = Instant::now() + limit;
    let mut remaining: Vec<ProcessIdentity> = processes
        .iter()
        .filter(|process| process.is_running())
        .cloned()
        .collect();
    while !remaining.is_empty() && Instant::now() < deadline {
        thread::sleep(POLL_INTERVAL);
        remaining.retain(ProcessIdentity::is_running);
    }
    remaining
}

fn pids(processes: &[ProcessIdentity]) -> Vec<u32> {
    processes.iter().map(|process| process.pid).collect()
}
