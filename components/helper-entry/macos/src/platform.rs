//! What the Helper entry needs from macOS: root processes told apart through libproc, stopped with
//! signals, and the run directory kept to root with Unix permissions.

use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use fvm_helper_entry::{Failure, Platform, ProcessIdentity};
use libproc::libproc::bsd_info::BSDInfo;
use libproc::libproc::proc_pid::{pidinfo, pidpath};
use libproc::processes::{ProcFilter, pids_by_type};
use rustix::io::Errno;
use rustix::process::{Pid, Signal};

pub(crate) struct MacOs;

impl Platform for MacOs {
    const FORCE_STOP: &'static str = "SIGKILL";

    /// Only root processes count: the Helper starts its clients as root.
    fn process(&self, pid: i32) -> Option<ProcessIdentity> {
        // The pid may come from a pidfile; 0 would name the kernel.
        if pid <= 0 {
            return None;
        }
        let info = pidinfo::<BSDInfo>(pid, 0).ok()?;
        if info.pbi_uid != 0 {
            return None;
        }
        // A zombie has no executable path any more.
        let path = pidpath(pid).ok()?;
        Some(ProcessIdentity {
            pid,
            start_seconds: info.pbi_start_tvsec,
            start_microseconds: info.pbi_start_tvusec,
            path,
        })
    }

    fn children(&self, parent: u32) -> Vec<ProcessIdentity> {
        // Listing fails when there are none.
        let children =
            pids_by_type(ProcFilter::ByParentProcess { ppid: parent }).unwrap_or_default();
        children
            .into_iter()
            .filter_map(|pid| self.process(i32::try_from(pid).ok()?))
            .collect()
    }

    /// SIGTERM.
    fn request_stop(&self, process: &ProcessIdentity) {
        signal(process, Signal::TERM);
    }

    /// SIGKILL.
    fn force_stop(&self, process: &ProcessIdentity) {
        signal(process, Signal::KILL);
    }

    /// Owned by this process's user with mode 0700.
    fn prepare_run_directory(&self, directory: &Path) -> Result<(), Failure> {
        let shown = directory.display();
        match DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(Failure::new(format!("mkdir {shown}"), error)),
        }
        let metadata = fs::symlink_metadata(directory)
            .map_err(|error| Failure::new(format!("lstat {shown}"), error))?;
        if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(Failure::new(
                format!("{shown} is not a directory owned by the helper"),
                io::Error::from_raw_os_error(Errno::PERM.raw_os_error()),
            ));
        }
        if metadata.mode() & 0o777 != 0o700 {
            fs::set_permissions(directory, Permissions::from_mode(0o700))
                .map_err(|error| Failure::new(format!("chmod {shown}"), error))?;
        }
        Ok(())
    }

    /// Mode 0600.
    fn create_private_file(&self, path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
}

fn signal(process: &ProcessIdentity, signal: Signal) {
    let pid = Pid::from_raw(process.pid).expect("a running process has a positive pid");
    // It may have exited since the check.
    let _ = rustix::process::kill_process(pid, signal);
}
