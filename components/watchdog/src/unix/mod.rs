//! Starting, signalling and collecting the client on Unix.

use std::ffi::CString;
use std::io::{self, PipeReader, PipeWriter};
use std::iter;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;

use fvm_contract::app_helper::ProcessExitStatus;
use fvm_contract::backend::{LaunchSpec, StopScope};
use nix::libc;
use nix::spawn::{PosixSpawnAttr, PosixSpawnFileActions, PosixSpawnFlags, posix_spawn};
use nix::sys::signal::{SigSet, Signal as SpawnSignal};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, WaitOptions};

pub(crate) struct Spawned {
    pub child: Child,
    pub stdin: PipeWriter,
    pub stdout: PipeReader,
    pub stderr: PipeReader,
}

/// Starts the client without a shell. It gets the three pipes as stdin, stdout and stderr and no
/// other descriptor, an empty signal mask and the default action for every signal, in a process
/// group of its own. The error is the errno value.
pub(crate) fn spawn(launch: &LaunchSpec) -> Result<Spawned, i32> {
    let (stdin_read, stdin) = io::pipe().map_err(os_error)?;
    let (stdout, stdout_write) = io::pipe().map_err(os_error)?;
    let (stderr, stderr_write) = io::pipe().map_err(os_error)?;

    let mut actions = PosixSpawnFileActions::init().map_err(|error| error as i32)?;
    for (pipe, target) in [
        (stdin_read.as_raw_fd(), 0),
        (stdout_write.as_raw_fd(), 1),
        (stderr_write.as_raw_fd(), 2),
    ] {
        actions
            .add_dup2(pipe, target)
            .map_err(|error| error as i32)?;
    }

    let mut attributes = PosixSpawnAttr::init().map_err(|error| error as i32)?;
    // The spawning thread may block signals, and the Helper replaces SIGTERM's action; openfortivpn
    // ends at once on SIGTERM before its tunnel is up only with the defaults.
    attributes
        .set_sigmask(&SigSet::empty())
        .map_err(|error| error as i32)?;
    let mut every_signal = SigSet::all();
    // Their action cannot be changed, so they are always at the default.
    every_signal.remove(SpawnSignal::SIGKILL);
    every_signal.remove(SpawnSignal::SIGSTOP);
    attributes
        .set_sigdefault(&every_signal)
        .map_err(|error| error as i32)?;
    // Its own group, so that leftovers it started can be killed together.
    attributes
        .set_pgroup(nix::unistd::Pid::from_raw(0))
        .map_err(|error| error as i32)?;
    attributes
        .set_flags(
            PosixSpawnFlags::POSIX_SPAWN_SETSIGMASK
                | PosixSpawnFlags::POSIX_SPAWN_SETSIGDEF
                | PosixSpawnFlags::POSIX_SPAWN_SETPGROUP
                // Apple's: closes every descriptor the file actions do not name, including those the
                // Swift side opened without close-on-exec.
                | PosixSpawnFlags::from_bits_retain(libc::POSIX_SPAWN_CLOEXEC_DEFAULT),
        )
        .map_err(|error| error as i32)?;

    let argv: Vec<CString> = iter::once(launch.program.as_os_str().as_bytes().to_vec())
        .chain(launch.args.iter().map(|arg| arg.clone().into_bytes()))
        .map(c_string)
        .collect();
    let env: Vec<CString> = launch
        .env
        .iter()
        .map(|(name, value)| c_string(format!("{name}={value}").into_bytes()))
        .collect();
    let pid = posix_spawn(launch.program.as_path(), &actions, &attributes, &argv, &env)
        .map_err(|error| error as i32)?;
    Ok(Spawned {
        child: Child(Pid::from_raw(pid.as_raw()).expect("posix_spawn returns a positive pid")),
        stdin,
        stdout,
        stderr,
    })
}

fn c_string(bytes: Vec<u8>) -> CString {
    CString::new(bytes).expect("the backend adapter allows no NUL in launch values")
}

fn os_error(error: io::Error) -> i32 {
    error
        .raw_os_error()
        .expect("creating a pipe fails with an errno")
}

/// The client until it is reaped. Until then its pid and process group id cannot be reused, so
/// signals reach only the client and the processes it started.
pub(crate) struct Child(Pid);

impl Child {
    pub(crate) fn id(&self) -> u32 {
        self.0.as_raw_nonzero().get().unsigned_abs()
    }

    /// SIGTERM.
    pub(crate) fn terminate(&self, scope: StopScope) {
        self.signal(Signal::TERM, scope);
    }

    /// SIGKILL.
    pub(crate) fn kill(&self, scope: StopScope) {
        self.signal(Signal::KILL, scope);
    }

    fn signal(&self, signal: Signal, scope: StopScope) {
        // An unreaped child takes a signal even after it exited.
        rustix::process::kill_process(self.0, signal).expect("signalling the unreaped client");
        if scope == StopScope::ProcessThenGroup {
            // Fails when nothing is left in the group but the client itself, which already got it.
            let _ = rustix::process::kill_process_group(self.0, signal);
        }
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

/// strerror's text and the number, as the Log tab shows OS errors: "No such file or directory
/// (errno 2)".
pub(crate) fn describe_errno(errno: i32) -> String {
    let shown = io::Error::from_raw_os_error(errno).to_string();
    let text = shown
        .strip_suffix(&format!(" (os error {errno})"))
        .expect("std shows an OS error as strerror's text and the code");
    format!("{text} (errno {errno})")
}
