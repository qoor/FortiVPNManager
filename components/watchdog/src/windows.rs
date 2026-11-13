//! Starting, stopping and collecting the client on Windows, and the run directory. Windows has no
//! signals, so a client is always ended by force, together with what it started, such as
//! openconnect's route script.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, PipeReader, PipeWriter, Read};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{self, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::app_helper::ProcessExitStatus;
use fvm_contract::backend::LaunchSpec;

use crate::run_directory::{Failure, ProcessIdentity};

/// The process gets a console that has no window, so nothing shows on a desktop.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// How often the exit watch looks at the client: std can wait for a child only through `&mut`.
const EXIT_POLL: Duration = Duration::from_millis(50);
/// Windows' own taskkill, which only administrators can change. It ends a process together with the
/// processes it started.
const TASKKILL: &str = r"C:\Windows\System32\taskkill.exe";
/// taskkill ends a tree at once; this only bounds a taskkill that hangs.
const TASKKILL_LIMIT: Duration = Duration::from_secs(10);

/// What the Log tab calls the stop request in the line about the kill that follows it.
pub(crate) const STOP_REQUEST: &str = "the forced stop";

pub(crate) struct Spawned {
    pub child: Child,
    pub stdin: PipeWriter,
    pub stdout: PipeReader,
    pub stderr: PipeReader,
    /// Unused on Windows: a blocked pipe read cannot be woken without unsafe code, so a reader ends
    /// when the last writer, such as the route script, closes the pipe.
    pub wake_read: PipeReader,
    pub wake_write: PipeWriter,
}

/// Starts the client without a shell, with the three pipes as stdin, stdout and stderr and only
/// the launch's environment, without a window. The error is the Windows error code.
pub(crate) fn spawn(launch: &LaunchSpec) -> Result<Spawned, i32> {
    let code = |error: io::Error| {
        error
            .raw_os_error()
            .expect("starting a process fails with a Windows error code")
    };
    let (stdin_read, stdin) = io::pipe().map_err(code)?;
    let (stdout, stdout_write) = io::pipe().map_err(code)?;
    let (stderr, stderr_write) = io::pipe().map_err(code)?;
    let (wake_read, wake_write) = io::pipe().map_err(code)?;
    // The command holds the client's ends and closes them when dropped, so that the output ends
    // when the client and what it started are gone.
    let child = Command::new(&launch.program)
        .args(&launch.args)
        .env_clear()
        .envs(launch.env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::from(stdin_read))
        .stdout(Stdio::from(stdout_write))
        .stderr(Stdio::from(stderr_write))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(code)?;
    let id = child.id();
    Ok(Spawned {
        child: Child {
            process: Arc::new(Mutex::new(child)),
            id,
        },
        stdin,
        stdout,
        stderr,
        wake_read,
        wake_write,
    })
}

/// The client until it is reaped. Its process handle stays open, so its pid is not reused.
pub(crate) struct Child {
    process: Arc<Mutex<process::Child>>,
    id: u32,
}

impl Child {
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    /// Ends the client and what it started at once: Windows has no stop request a client could
    /// answer by logging out. Returns what was done, for the Log tab, and that it was forced.
    pub(crate) fn terminate(&self) -> (&'static str, bool) {
        (self.kill(), true)
    }

    /// Ends the client and the processes it started, such as a route script that would otherwise
    /// go on changing routes after the client is gone. When taskkill cannot do it, the client alone
    /// is ended.
    pub(crate) fn kill(&self) -> &'static str {
        if kill_tree(self.id) {
            return "killed the process tree";
        }
        // Fails only when it already exited.
        let _ = self
            .process
            .lock()
            .expect("the client's handle is never held across a panic")
            .kill();
        "killed the client; taskkill failed"
    }

    pub(crate) fn exit_watch(&self) -> ExitWatch {
        ExitWatch(Arc::clone(&self.process))
    }

    pub(crate) fn reap(self) -> ProcessExitStatus {
        let status = self
            .process
            .lock()
            .expect("the client's handle is never held across a panic")
            .wait();
        match status.ok().and_then(|status| status.code()) {
            Some(code) => ProcessExitStatus::Exited { code },
            None => ProcessExitStatus::Unknown {},
        }
    }
}

pub(crate) struct ExitWatch(Arc<Mutex<process::Child>>);

impl ExitWatch {
    /// Returns once the client exited; `Child::reap` collects the status.
    pub(crate) fn wait(self) {
        loop {
            let exited = self
                .0
                .lock()
                .expect("the client's handle is never held across a panic")
                .try_wait();
            // An error means it cannot be watched any more; `reap` reports that.
            if !matches!(exited, Ok(None)) {
                return;
            }
            thread::sleep(EXIT_POLL);
        }
    }
}

/// Passes each read of `pipe` to `deliver` until the pipe ends or `deliver` returns false.
/// Returning closes `pipe`.
pub(crate) fn read_output(
    mut pipe: PipeReader,
    _wake: &PipeReader,
    mut deliver: impl FnMut(Vec<u8>) -> bool,
) {
    let mut buffer = vec![0; 65_536];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => return,
            Ok(count) => {
                if !deliver(buffer[..count].to_vec()) {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            // A broken pipe ends the stream as the end of file does.
            Err(_) => return,
        }
    }
}

/// The time since the Helper started; up times decide only whether a session was stable. `Instant`
/// reads QueryPerformanceCounter.
pub(crate) fn time_including_sleep() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

/// Windows ends processes with exit codes only.
pub(crate) fn signal_name(_signal: i32) -> Option<&'static str> {
    None
}

/// How the Helper's log names the forced stop of a process an earlier Helper left.
pub(crate) const FORCE_STOP: &str = "taskkill";

/// Telling Windows processes apart comes with the Windows Helper. Until then no process is
/// recognised, so a run records none and nothing an earlier Helper left is stopped.
pub(crate) fn process(_pid: u32) -> Option<ProcessIdentity> {
    None
}

/// See `process`.
pub(crate) fn children(_parent: u32) -> Vec<ProcessIdentity> {
    Vec::new()
}

/// Windows has no stop request a client could answer, so this ends the process and what it
/// started at once, as the forced stop does.
pub(crate) fn request_stop(process: &ProcessIdentity) {
    force_stop(process);
}

pub(crate) fn force_stop(process: &ProcessIdentity) {
    kill_tree(process.pid);
}

/// The directory inherits its access rights from where it is created, which only the system and
/// administrators can reach; so only the last level is created, and it must be a real directory.
pub(crate) fn prepare_run_directory(directory: &Path) -> Result<(), Failure> {
    let shown = directory.display();
    match fs::create_dir(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(Failure::new(format!("create {shown}"), error)),
    }
    // A link has no directory type of its own, so it fails here too.
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| Failure::new(format!("inspect {shown}"), error))?;
    if !metadata.is_dir() {
        return Err(Failure::new(
            format!("{shown} is not a directory"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ));
    }
    Ok(())
}

/// A new file with the run directory's access rights; an existing file or link at `path` is an
/// error.
pub(crate) fn create_private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Whether taskkill ended process `pid` and every process it started. Its messages are in the
/// system's language, so only its exit status is read. It fails without the system folder's
/// variables, so it keeps those of the Helper's environment.
fn kill_tree(pid: u32) -> bool {
    let Ok(mut taskkill) = Command::new(TASKKILL)
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .env_clear()
        .envs(
            ["SystemRoot", "windir"]
                .iter()
                .filter_map(|&name| Some((name, env::var_os(name)?))),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + TASKKILL_LIMIT;
    loop {
        match taskkill.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(EXIT_POLL),
            _ => {
                let _ = taskkill.kill();
                let _ = taskkill.wait();
                return false;
            }
        }
    }
}
