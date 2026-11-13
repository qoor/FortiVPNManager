//! Starting, signalling and collecting the client on Unix; telling root processes apart through
//! macOS's libproc, so that an earlier Helper's are found and stopped with signals; and keeping the
//! run directory to root with Unix permissions.

use std::ffi::CString;
use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io::{self, PipeReader, PipeWriter, Read};
use std::iter;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

use fvm_contract::app_helper::ProcessExitStatus;
use fvm_contract::backend::LaunchSpec;
use libproc::libproc::bsd_info::BSDInfo;
use libproc::libproc::proc_pid::{pidinfo, pidpath};
use libproc::processes::{ProcFilter, pids_by_type};
use nix::libc;
use nix::spawn::{PosixSpawnAttr, PosixSpawnFileActions, PosixSpawnFlags, posix_spawn};
use nix::sys::signal::{SigSet, Signal as SpawnSignal};
use rustix::event::{PollFd, PollFlags};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, WaitOptions};
use rustix::time::ClockId;

use crate::run_directory::{Failure, ProcessIdentity};

pub(crate) struct Spawned {
    pub child: Child,
    pub stdin: PipeWriter,
    pub stdout: PipeReader,
    pub stderr: PipeReader,
    /// Closing `wake_write` ends `read_output` on both output pipes.
    pub wake_read: PipeReader,
    pub wake_write: PipeWriter,
}

/// Starts the client without a shell. It gets the three pipes as stdin, stdout and stderr and no
/// other descriptor, an empty signal mask and the default action for every signal, in a process
/// group of its own. The error is the errno value.
pub(crate) fn spawn(launch: &LaunchSpec) -> Result<Spawned, i32> {
    let pipes = || Ok::<_, io::Error>((io::pipe()?, io::pipe()?, io::pipe()?, io::pipe()?));
    let (
        (stdin_read, stdin),
        (stdout, stdout_write),
        (stderr, stderr_write),
        (wake_read, wake_write),
    ) = pipes().map_err(|error| {
        error
            .raw_os_error()
            .expect("creating a pipe fails with an errno")
    })?;

    let argv: Vec<CString> = iter::once(launch.program.as_os_str().as_bytes().to_vec())
        .chain(launch.args.iter().map(|arg| arg.clone().into_bytes()))
        .map(|bytes| {
            CString::new(bytes).expect("the backend adapter allows no NUL in launch values")
        })
        .collect();
    let env: Vec<CString> = launch
        .env
        .iter()
        .map(|(name, value)| {
            CString::new(format!("{name}={value}"))
                .expect("the backend adapter allows no NUL in launch values")
        })
        .collect();

    let start = || -> nix::Result<nix::unistd::Pid> {
        let mut actions = PosixSpawnFileActions::init()?;
        actions.add_dup2(stdin_read.as_raw_fd(), 0)?;
        actions.add_dup2(stdout_write.as_raw_fd(), 1)?;
        actions.add_dup2(stderr_write.as_raw_fd(), 2)?;

        let mut attributes = PosixSpawnAttr::init()?;
        // The spawning thread may block signals, and the Helper replaces SIGTERM's action;
        // openfortivpn ends at once on SIGTERM before its tunnel is up only with the defaults.
        attributes.set_sigmask(&SigSet::empty())?;
        let mut every_signal = SigSet::all();
        // Their action cannot be changed, so they are always at the default.
        every_signal.remove(SpawnSignal::SIGKILL);
        every_signal.remove(SpawnSignal::SIGSTOP);
        attributes.set_sigdefault(&every_signal)?;
        // Its own group, so that leftovers it started can be killed together.
        attributes.set_pgroup(nix::unistd::Pid::from_raw(0))?;
        attributes.set_flags(
            PosixSpawnFlags::POSIX_SPAWN_SETSIGMASK
                | PosixSpawnFlags::POSIX_SPAWN_SETSIGDEF
                | PosixSpawnFlags::POSIX_SPAWN_SETPGROUP
                // Apple's: closes every descriptor the file actions do not name, including those
                // the Swift side opened without close-on-exec.
                | PosixSpawnFlags::from_bits_retain(libc::POSIX_SPAWN_CLOEXEC_DEFAULT),
        )?;
        posix_spawn(launch.program.as_path(), &actions, &attributes, &argv, &env)
    };
    let pid = start().map_err(|errno| errno as i32)?;
    Ok(Spawned {
        child: Child(Pid::from_raw(pid.as_raw()).expect("posix_spawn returns a positive pid")),
        stdin,
        stdout,
        stderr,
        wake_read,
        wake_write,
    })
}

/// What the Log tab calls the stop request in the line about the kill that follows it.
pub(crate) const STOP_REQUEST: &str = "SIGTERM";

/// The client until it is reaped. Until then its pid and process group id cannot be reused, so
/// signals reach only the client and the processes it started.
pub(crate) struct Child(Pid);

impl Child {
    pub(crate) fn id(&self) -> u32 {
        self.0.as_raw_nonzero().get().unsigned_abs()
    }

    /// SIGTERM to the client alone: what it started, such as /sbin/route restoring the routes,
    /// must be left to finish. Returns what was done, for the Log tab, and that it was a request,
    /// which the client may answer by logging out.
    pub(crate) fn terminate(&self) -> (&'static str, bool) {
        // An unreaped child takes a signal even after it exited.
        rustix::process::kill_process(self.0, Signal::TERM)
            .expect("signalling the unreaped client");
        ("sent SIGTERM", false)
    }

    /// SIGKILL to the client, then to its whole process group.
    pub(crate) fn kill(&self) -> &'static str {
        rustix::process::kill_process(self.0, Signal::KILL)
            .expect("signalling the unreaped client");
        // Fails when nothing is left in the group but the client itself, which already got it.
        let _ = rustix::process::kill_process_group(self.0, Signal::KILL);
        "sent SIGKILL"
    }

    pub(crate) fn exit_watch(&self) -> ExitWatch {
        ExitWatch(self.0)
    }

    /// Kills what is left of its process group, then collects its status.
    pub(crate) fn reap(self) -> ProcessExitStatus {
        // A process the client started and left behind, such as /sbin/route, would keep the output
        // pipes open.
        let _ = rustix::process::kill_process_group(self.0, Signal::KILL);
        loop {
            match rustix::process::waitpid(Some(self.0), WaitOptions::empty()) {
                Ok(Some((_, status))) => {
                    return if let Some(code) = status.exit_status() {
                        ProcessExitStatus::Exited { code }
                    } else if let Some(signal) = status.terminating_signal() {
                        ProcessExitStatus::Signaled { signal }
                    } else {
                        ProcessExitStatus::Unknown {}
                    };
                }
                Ok(None) => unreachable!("waitpid without WNOHANG returns a status"),
                Err(Errno::INTR) => {}
                // Something else collected it.
                Err(_) => return ProcessExitStatus::Unknown {},
            }
        }
    }
}

pub(crate) struct ExitWatch(Pid);

impl ExitWatch {
    /// Blocks until the client exits, and leaves it unreaped for `Child::reap`.
    pub(crate) fn wait(self) {
        loop {
            match rustix::process::waitid(
                WaitId::Pid(self.0),
                WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
            ) {
                Err(Errno::INTR) => {}
                // Any other error means it cannot be waited for any more; `reap` reports that.
                _ => return,
            }
        }
    }
}

/// Passes each read of `pipe` to `deliver` until the pipe ends, `deliver` returns false, or the
/// wake pipe's writer is closed, whichever comes first. Returning closes `pipe`.
pub(crate) fn read_output(
    mut pipe: PipeReader,
    wake: &PipeReader,
    mut deliver: impl FnMut(Vec<u8>) -> bool,
) {
    let mut buffer = vec![0; 65_536];
    loop {
        let mut ready = [
            PollFd::new(&pipe, PollFlags::IN),
            PollFd::new(wake, PollFlags::IN),
        ];
        match rustix::event::poll(&mut ready, None) {
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(error) => panic!("waiting for the client's output: {error}"),
        }
        if !ready[1].revents().is_empty() {
            return;
        }
        if ready[0].revents().is_empty() {
            continue;
        }
        match pipe.read(&mut buffer) {
            Ok(0) => return,
            Ok(count) => {
                if !deliver(buffer[..count].to_vec()) {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            // Ends the stream as the end of file does.
            Err(_) => return,
        }
    }
}

/// Time that keeps counting while the Mac sleeps, unlike `Instant`.
pub(crate) fn time_including_sleep() -> Duration {
    Duration::try_from(rustix::time::clock_gettime(ClockId::Monotonic))
        .expect("the monotonic clock is never negative")
}

/// The signals the Log tab names in the exit line.
pub(crate) fn signal_name(signal: i32) -> Option<&'static str> {
    [
        (Signal::HUP, "SIGHUP"),
        (Signal::INT, "SIGINT"),
        (Signal::QUIT, "SIGQUIT"),
        (Signal::ABORT, "SIGABRT"),
        (Signal::KILL, "SIGKILL"),
        (Signal::SEGV, "SIGSEGV"),
        (Signal::PIPE, "SIGPIPE"),
        (Signal::TERM, "SIGTERM"),
    ]
    .into_iter()
    .find(|(known, _)| known.as_raw() == signal)
    .map(|(_, name)| name)
}

/// How the Helper's log names the forced stop of a process an earlier Helper left.
pub(crate) const FORCE_STOP: &str = "SIGKILL";

/// The live process `pid` if it runs as root, as the Helper starts its clients; `None` if it is
/// gone, a zombie, unreadable or another user's.
pub(crate) fn process(pid: u32) -> Option<ProcessIdentity> {
    // The pid may come from a pidfile; 0 would name the kernel.
    let raw = i32::try_from(pid).ok().filter(|&raw| raw > 0)?;
    let info = pidinfo::<BSDInfo>(raw, 0).ok()?;
    if info.pbi_uid != 0 {
        return None;
    }
    // A zombie has no executable path any more.
    let path = pidpath(raw).ok()?;
    Some(ProcessIdentity {
        pid,
        start_seconds: info.pbi_start_tvsec,
        start_microseconds: info.pbi_start_tvusec,
        path,
    })
}

/// The root processes that `parent` started and that still run.
pub(crate) fn children(parent: u32) -> Vec<ProcessIdentity> {
    // Listing fails when there are none.
    let children = pids_by_type(ProcFilter::ByParentProcess { ppid: parent }).unwrap_or_default();
    children.into_iter().filter_map(process).collect()
}

/// SIGTERM, so that a client can restore the routes. It may have exited since it was checked.
pub(crate) fn request_stop(process: &ProcessIdentity) {
    signal(process, Signal::TERM);
}

/// SIGKILL. It may have exited since it was checked.
pub(crate) fn force_stop(process: &ProcessIdentity) {
    signal(process, Signal::KILL);
}

fn signal(process: &ProcessIdentity, signal: Signal) {
    let pid = i32::try_from(process.pid)
        .ok()
        .and_then(Pid::from_raw)
        .expect("a running process has a positive pid_t");
    // It may have exited since the check.
    let _ = rustix::process::kill_process(pid, signal);
}

/// Creates the run directory if needed and makes sure it is a real directory owned by this
/// process's user with mode 0700, so nobody else can place or read files in it.
pub(crate) fn prepare_run_directory(directory: &Path) -> Result<(), Failure> {
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

/// A new file with mode 0600; an existing file or symlink at `path` is an error.
pub(crate) fn create_private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}
