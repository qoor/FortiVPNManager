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
use rustix::process::{Pid, Signal};

use crate::entry::{Event, Swift};
use crate::processes::ProcessIdentity;
use crate::run_directory::{self, RunRecord};

/// How long the processes get to disappear after SIGKILL.
const KILL_LIMIT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Stops the recorded processes that still run, asks Swift to repair the routes and waits for the
/// answer, then removes the files the earlier Helper left. Until "ready", Swift accepts no app
/// connection and passes on no signal, so the answer is the only message it sends.
pub(crate) fn clean_up(
    backend: &impl Backend,
    directory: &Path,
    swift: &Swift,
    events: &Receiver<Event>,
) {
    if let Err(failure) = run_directory::prepare(directory) {
        swift.log(
            LogLevel::Error,
            LogCategory::Helper,
            format!("cannot prepare the run directory: {failure}"),
        );
        return;
    }
    if let Some(record) = run_directory::read_record(directory) {
        // An unreadable pidfile still means a run did not finish, so the routes are checked.
        let record = record.unwrap_or_else(|_| {
            swift.log(
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
            swift.log(
                LogLevel::Notice,
                LogCategory::Helper,
                format!(
                    "stopping {} process(es) left by a previous helper: {:?}",
                    orphans.len(),
                    pids(&orphans)
                ),
            );
            terminate(&orphans, backend.stop_grace_period(), swift);
        }
        swift.send(&FromEntry::RepairRoutes {
            gateway: record.gateway,
        });
        let Event::Swift(ToEntry::RoutesRepaired { outcome }) =
            events.recv().expect("the entry holds a sender")
        else {
            panic!("Swift sent something other than the route repair's answer before ready");
        };
        swift.log(
            LogLevel::Notice,
            LogCategory::Helper,
            format!("cleaned up after an unfinished run (route repair: {outcome:?})"),
        );
    }
    for leftover in run_directory::leftovers(directory) {
        remove(&leftover, swift);
    }
}

/// Deletes a file; one that is already gone is fine.
pub(crate) fn remove(path: &Path, swift: &Swift) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => swift.log(
            LogLevel::Error,
            LogCategory::Helper,
            format!("cannot delete {}: {error}", path.display()),
        ),
    }
}

/// SIGTERM first, so the client can restore the routes, and SIGKILL after `grace_period`.
fn terminate(processes: &[ProcessIdentity], grace_period: Duration, swift: &Swift) {
    signal(processes, Signal::TERM);
    let survivors = wait_for_exit(processes, grace_period);
    if survivors.is_empty() {
        return;
    }
    swift.log(
        LogLevel::Notice,
        LogCategory::Helper,
        format!("sending SIGKILL to {:?}", pids(&survivors)),
    );
    signal(&survivors, Signal::KILL);
    let stuck = wait_for_exit(&survivors, KILL_LIMIT);
    if !stuck.is_empty() {
        swift.log(
            LogLevel::Error,
            LogCategory::Helper,
            format!("processes still running after SIGKILL: {:?}", pids(&stuck)),
        );
    }
}

/// Checks each identity right before signalling it, since its pid may have been reused meanwhile.
fn signal(processes: &[ProcessIdentity], signal: Signal) {
    for process in processes.iter().filter(|process| process.is_running()) {
        let pid = Pid::from_raw(process.pid).expect("a running process has a positive pid");
        // It may have exited since the check.
        let _ = rustix::process::kill_process(pid, signal);
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

fn pids(processes: &[ProcessIdentity]) -> Vec<i32> {
    processes.iter().map(|process| process.pid).collect()
}
