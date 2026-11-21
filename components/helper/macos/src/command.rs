use std::fmt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

const LIMIT: Duration = Duration::from_secs(10);

pub(crate) struct Output {
    pub status: Status,
    pub output: String,
    pub error_output: String,
}

impl Output {
    pub fn succeeded(&self) -> bool {
        self.status == Status::Exited(0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Status {
    Exited(i32),
    Signaled(i32),
    /// `error_output` then says why.
    SpawnFailed(i32),
}

impl fmt::Display for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited(code) => write!(formatter, "exited(code: {code})"),
            Self::Signaled(signal) => write!(formatter, "signaled(signal: {signal})"),
            Self::SpawnFailed(errno) => write!(formatter, "spawnFailed(errno: {errno})"),
        }
    }
}

/// Runs `executable` with a fixed argv and `PATH` as its only environment variable, no shell, and
/// collects its output. A tool still running after 10 seconds is killed.
pub(crate) fn run(executable: &str, arguments: &[String]) -> Output {
    let spawned = Command::new(executable)
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            let code = error.raw_os_error().unwrap_or(Errno::ENOENT as i32);
            return Output {
                status: Status::SpawnFailed(code),
                output: String::new(),
                error_output: format!("{} (errno {code})", Errno::from_raw(code).desc()),
            };
        }
    };
    let pid = Pid::from_raw(child.id() as i32);
    let reaped = Arc::new(AtomicBool::new(false));
    let (done, finished) = mpsc::channel::<()>();
    let killer = {
        let reaped = Arc::clone(&reaped);
        thread::spawn(move || {
            if finished.recv_timeout(LIMIT) == Err(mpsc::RecvTimeoutError::Timeout)
                && !reaped.load(Ordering::SeqCst)
            {
                let _ = kill(pid, Signal::SIGKILL);
            }
        })
    };
    let mut output = Vec::new();
    let mut error_output = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut output);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut error_output);
    }
    let status = child
        .wait()
        .expect("waiting for a child this process spawned");
    reaped.store(true, Ordering::SeqCst);
    let _ = done.send(());
    let _ = killer.join();
    let status = match (
        status.code(),
        std::os::unix::process::ExitStatusExt::signal(&status),
    ) {
        (Some(code), _) => Status::Exited(code),
        (None, Some(signal)) => Status::Signaled(signal),
        (None, None) => unreachable!("a reaped child exited or was signaled"),
    };
    Output {
        status,
        output: String::from_utf8_lossy(&output).into_owned(),
        error_output: String::from_utf8_lossy(&error_output).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(arguments: &[&str]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect()
    }

    #[test]
    fn a_tool_gets_its_arguments_and_only_path() {
        let result = run("/usr/bin/env", &[]);
        assert!(result.succeeded());
        assert_eq!(result.output, "PATH=/usr/bin:/bin:/usr/sbin:/sbin\n");
        let result = run("/bin/echo", &strings(&["a b", "$HOME"]));
        assert_eq!(result.output, "a b $HOME\n");
    }

    #[test]
    fn the_exit_code_and_both_outputs_are_kept() {
        let result = run(
            "/bin/sh",
            &strings(&["-c", "echo out; echo err >&2; exit 3"]),
        );
        assert_eq!(result.status, Status::Exited(3));
        assert_eq!(result.output, "out\n");
        assert_eq!(result.error_output, "err\n");
        assert!(!result.succeeded());
    }

    #[test]
    fn a_missing_tool_is_a_failed_spawn() {
        let result = run("/nonexistent/tool", &[]);
        assert_eq!(result.status, Status::SpawnFailed(2));
        assert_eq!(result.error_output, "No such file or directory (errno 2)");
        assert_eq!(result.status.to_string(), "spawnFailed(errno: 2)");
    }

    #[test]
    fn a_tool_that_runs_too_long_is_killed() {
        let started = std::time::Instant::now();
        let result = run("/bin/sleep", &strings(&["30"]));
        assert_eq!(result.status, Status::Signaled(9));
        assert!(started.elapsed() < Duration::from_secs(15));
    }
}
