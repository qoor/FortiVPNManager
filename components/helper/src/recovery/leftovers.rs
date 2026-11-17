//! Cleaning up after an earlier Helper: if it died during a run, the client and the processes it
//! started may still run, and a client that died without restoring the routes left them broken; and
//! the exception routes it added may still be in the routing table.

use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fvm_contract::app_helper::VpnClient;
use fvm_contract::backend::Backend;
use fvm_contract::helper_internal::{AddedRoute, LogCategory, LogLevel, OutsideChanges};

use crate::process::os;
use crate::recovery::run_directory::{self, ProcessIdentity, RunRecord};

/// How long the processes get to disappear after the forced stop.
const KILL_LIMIT: Duration = Duration::from_secs(2);

/// Cleans up after an earlier Helper in `directory`. If it died during a run, stops the processes
/// its pidfile names that still run and belong to one of `clients`, calls `repair_routes` with the
/// run's gateway, if it is known, and removes the files it left once that returns. The routes are
/// repaired whenever a pidfile is left, even an unreadable one, since a run did not finish. Then
/// `remove_bypass_routes` gets the exception routes it recorded and answers those still in the
/// routing table, which stay recorded and are returned as this Helper's. `log` takes lines for the
/// Helper's log.
pub(crate) fn clean_up(
    clients: &[(VpnClient, Arc<dyn Backend>)],
    directory: &Path,
    log: impl Fn(LogLevel, LogCategory, String),
    revert_network: impl FnOnce(OutsideChanges),
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
                let path = Path::new(&process.path);
                clients
                    .iter()
                    .any(|(_, backend)| backend.is_own_executable(path))
                    && process.is_running()
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
            let grace_period = clients
                .iter()
                .map(|(_, backend)| backend.stop_grace_period())
                .max()
                .unwrap_or_default();
            terminate(&orphans, grace_period, &log);
        }
        if let Some(changes) = record.outside_changes {
            revert_network(changes);
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
    processes
        .iter()
        .filter(|process| {
            os::watch_exit(process).is_some_and(|exit| {
                !exit.wait(Some(deadline.saturating_duration_since(Instant::now())))
            })
        })
        .cloned()
        .collect()
}

fn pids(processes: &[ProcessIdentity]) -> Vec<u32> {
    processes.iter().map(|process| process.pid).collect()
}

#[cfg(all(test, windows))]
mod tests {
    use std::process::{Command, Stdio};
    use std::{slice, thread};

    use super::*;

    #[test]
    fn leftovers_are_waited_for_until_they_exit_or_the_limit_passes() {
        let mut child = Command::new(r"C:\Windows\System32\cmd.exe")
            .args(["/d", "/c", "ping -n 30 127.0.0.1 >nul"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let leftover = os::process(child.id()).unwrap();
        let started = Instant::now();
        assert_eq!(
            wait_for_exit(slice::from_ref(&leftover), Duration::from_millis(300)),
            slice::from_ref(&leftover)
        );
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(300 - 16) && waited < Duration::from_secs(2),
            "{waited:?}"
        );
        let stopped = leftover.clone();
        let killer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            os::force_stop(&stopped);
            child.wait().unwrap();
        });
        let started = Instant::now();
        assert!(wait_for_exit(slice::from_ref(&leftover), Duration::from_secs(10)).is_empty());
        let waited = started.elapsed();
        assert!(waited < Duration::from_secs(5), "{waited:?}");
        killer.join().unwrap();
    }
}
