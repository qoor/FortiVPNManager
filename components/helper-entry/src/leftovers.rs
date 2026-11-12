//! Cleaning up after a Helper that died during a run, before any app connection is accepted: the
//! client and the processes it started may still run, and a client that died without restoring the
//! routes leaves the Mac without a default route.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::backend::Backend;
use fvm_contract::helper_internal::{FromEntry, LogCategory, LogLevel, ToEntry};

use crate::entry::{Event, OsSide};
use crate::platform::{Platform, ProcessIdentity};
use crate::run_directory::{self, RunRecord};

/// How long the processes get to disappear after the forced stop.
const KILL_LIMIT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Stops the recorded processes that still run, asks the OS side to repair the routes and waits for
/// the answer, then removes the files the earlier Helper left. Until "ready", the OS side accepts no
/// app connection and passes on no stop request, so the answer is the only message it sends.
pub(crate) fn clean_up<P: Platform>(
    backend: &impl Backend,
    platform: &P,
    directory: &Path,
    os: &OsSide,
    events: &Receiver<Event>,
) {
    if let Err(failure) = platform.prepare_run_directory(directory) {
        os.log(
            LogLevel::Error,
            LogCategory::Helper,
            format!("cannot prepare the run directory: {failure}"),
        );
        return;
    }
    if let Some(record) = run_directory::read_record(directory) {
        // An unreadable pidfile still means a run did not finish, so the routes are checked.
        let record = record.unwrap_or_else(|_| {
            os.log(
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
                backend.is_own_executable(Path::new(&process.path)) && process.is_running(platform)
            })
            .collect();
        if !orphans.is_empty() {
            os.log(
                LogLevel::Notice,
                LogCategory::Helper,
                format!(
                    "stopping {} process(es) left by a previous helper: {:?}",
                    orphans.len(),
                    pids(&orphans)
                ),
            );
            terminate(platform, &orphans, backend.stop_grace_period(), os);
        }
        os.send(&FromEntry::RepairRoutes {
            gateway: record.gateway,
        });
        let Event::FromOs(ToEntry::RoutesRepaired { outcome }) =
            events.recv().expect("the entry holds a sender")
        else {
            panic!("the OS side sent something other than the route repair's answer before ready");
        };
        os.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!("cleaned up after an unfinished run (route repair: {outcome:?})"),
        );
    }
    for leftover in run_directory::leftovers(directory) {
        remove(&leftover, os);
    }
}

/// Deletes a file; one that is already gone is fine.
pub(crate) fn remove(path: &Path, os: &OsSide) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => os.log(
            LogLevel::Error,
            LogCategory::Helper,
            format!("cannot delete {}: {error}", path.display()),
        ),
    }
}

/// The OS's stop request first, so the client can restore the routes, and its forced stop after
/// `grace_period`.
fn terminate<P: Platform>(
    platform: &P,
    processes: &[ProcessIdentity],
    grace_period: Duration,
    os: &OsSide,
) {
    stop(platform, processes, P::request_stop);
    let survivors = wait_for_exit(platform, processes, grace_period);
    if survivors.is_empty() {
        return;
    }
    os.log(
        LogLevel::Notice,
        LogCategory::Helper,
        format!("sending {} to {:?}", P::FORCE_STOP, pids(&survivors)),
    );
    stop(platform, &survivors, P::force_stop);
    let stuck = wait_for_exit(platform, &survivors, KILL_LIMIT);
    if !stuck.is_empty() {
        os.log(
            LogLevel::Error,
            LogCategory::Helper,
            format!(
                "processes still running after {}: {:?}",
                P::FORCE_STOP,
                pids(&stuck)
            ),
        );
    }
}

/// Checks each identity right before stopping it, since its pid may have been reused meanwhile.
fn stop<P: Platform>(platform: &P, processes: &[ProcessIdentity], how: fn(&P, &ProcessIdentity)) {
    for process in processes
        .iter()
        .filter(|process| process.is_running(platform))
    {
        how(platform, process);
    }
}

/// The processes still running once all are gone or `limit` passed.
fn wait_for_exit(
    platform: &impl Platform,
    processes: &[ProcessIdentity],
    limit: Duration,
) -> Vec<ProcessIdentity> {
    let deadline = Instant::now() + limit;
    let mut remaining: Vec<ProcessIdentity> = processes
        .iter()
        .filter(|process| process.is_running(platform))
        .cloned()
        .collect();
    while !remaining.is_empty() && Instant::now() < deadline {
        thread::sleep(POLL_INTERVAL);
        remaining.retain(|process| process.is_running(platform));
    }
    remaining
}

fn pids(processes: &[ProcessIdentity]) -> Vec<i32> {
    processes.iter().map(|process| process.pid).collect()
}
