//! Runs a system tool the way the Helper runs every tool: a fixed path and argv, no shell, no
//! window, only the variables the tool needs, input on stdin and a time limit.

use std::env;
use std::io::{self, Read, Write};
use std::os::windows::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// The process gets a console that has no window, so nothing shows on a desktop.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL: Duration = Duration::from_millis(50);
/// How long the output may still take once the tool ended. A program the tool started can keep the
/// output open; it is not waited for, and what it holds is given up.
const OUTPUT_GRACE: Duration = Duration::from_secs(1);

/// The variables of the Helper's own environment a tool gets: Windows' folders and search path.
/// PowerShell finds its modules and writes its caches through them.
const PASSED_VARIABLES: [&str; 9] = [
    "SystemRoot",
    "windir",
    "SystemDrive",
    "PATH",
    "PATHEXT",
    "TEMP",
    "TMP",
    "LOCALAPPDATA",
    "PSModulePath",
];

pub(crate) struct Output {
    /// `None` when the tool was killed at the time limit.
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `program` with `args`, writes `input` to its stdin and closes it, and collects its output.
/// A tool still running after `limit` is killed, and nothing here waits longer than `limit` and a
/// moment. The error is the failure to start or watch it.
pub(crate) fn run(
    program: &str,
    args: &[&str],
    input: Vec<u8>,
    limit: Duration,
) -> io::Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .envs(
            PASSED_VARIABLES
                .iter()
                .filter_map(|&name| Some((name, env::var_os(name)?))),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    // Read on their own threads, so a tool that fills one pipe cannot block on it while this waits.
    let (stdout_sender, stdout_read) = mpsc::channel();
    let (stderr_sender, stderr_read) = mpsc::channel();
    thread::spawn(move || {
        let _ = stdout_sender.send(read_all(stdout));
    });
    thread::spawn(move || {
        let _ = stderr_sender.send(read_all(stderr));
    });
    // Written on its own thread, so a tool that never reads its input is still stopped at the time
    // limit; killing it ends the write. A tool that exits without reading it closes the pipe, and
    // its exit status tells.
    thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            polled => {
                let _ = child.kill();
                let _ = child.wait();
                polled?;
                break None;
            }
        }
    };
    let ended = Instant::now();
    let collect = |output: mpsc::Receiver<Vec<u8>>| {
        let left = (ended + OUTPUT_GRACE).saturating_duration_since(Instant::now());
        output.recv_timeout(left).unwrap_or_default()
    };
    Ok(Output {
        status,
        stdout: collect(stdout_read),
        stderr: collect(stderr_read),
    })
}

fn read_all(mut pipe: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    // What arrived before a read error is still worth showing.
    let _ = pipe.read_to_end(&mut bytes);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_the_tool_started_does_not_hold_up_the_answer() {
        let started = Instant::now();
        // cmd ends at once; the ping it starts keeps cmd's output open for about 20 s.
        let output = run(
            r"C:\Windows\System32\cmd.exe",
            &["/d", "/c", "start /b ping -n 20 127.0.0.1 & echo done"],
            Vec::new(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(output.status.is_some_and(|status| status.success()));
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "{:?}",
            started.elapsed()
        );
    }
}
