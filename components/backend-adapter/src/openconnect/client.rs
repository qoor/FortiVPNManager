//! Which openconnect executable the Helper may run, and which leftover processes are ours. Only the
//! copy at the OS's fixed location is run, which only administrators can change (`platform`).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::backend::{ClientExecutable, ClientProblem, ClientProblemKind, UntestedVersion};

use super::platform::CLIENT;

/// The release series whose output wording the adapter was checked against.
const TESTED_SERIES: &str = "9.21";
/// `--version` prints and exits at once; a client that does not is not used.
const VERSION_LIMIT: Duration = Duration::from_secs(5);
const VERSION_POLL: Duration = Duration::from_millis(50);

pub(super) fn locate() -> Result<ClientExecutable, ClientProblem> {
    let Some(client) = CLIENT else {
        return Err(ClientProblem {
            kind: ClientProblemKind::NotFound {},
            path: PathBuf::from("openconnect"),
        });
    };
    let path = PathBuf::from(client);
    let problem = |kind| ClientProblem {
        kind,
        path: path.clone(),
    };
    // Not followed: a link swapped in would point the Helper at something else.
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| problem(ClientProblemKind::NotFound {}))?;
    if !metadata.is_file() {
        return Err(problem(ClientProblemKind::NotRegularFile {}));
    }
    let version = version(&path).ok_or_else(|| problem(ClientProblemKind::NotExecutable {}))?;
    Ok(ClientExecutable { path, version })
}

/// The English text the current app shows as it is.
pub(super) fn describe(problem: &ClientProblem) -> String {
    let path = problem.path.display();
    match problem.kind {
        ClientProblemKind::NotFound {} => format!(
            "openconnect not found at {path}; install OpenConnect {TESTED_SERIES} for Windows in its default folder"
        ),
        ClientProblemKind::OutsideAllowedLocation {} => {
            format!("openconnect is outside its allowed location: {path}")
        }
        ClientProblemKind::NotRegularFile {} => {
            format!("openconnect is not a regular file: {path}")
        }
        ClientProblemKind::WritableByOthers { mode } => format!(
            "openconnect is writable by other users (mode {mode:o}); refusing to run it: {path}"
        ),
        ClientProblemKind::NotExecutable {} => {
            format!("openconnect did not report its version: {path}")
        }
    }
}

pub(super) fn untested_version(version: &str) -> Option<UntestedVersion> {
    let tested = version
        .strip_prefix(TESTED_SERIES)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '-']));
    (!tested).then(|| UntestedVersion {
        tested_series: TESTED_SERIES.into(),
    })
}

/// Windows paths ignore case.
pub(super) fn is_own_executable(path: &Path) -> bool {
    CLIENT
        .zip(path.to_str())
        .is_some_and(|(client, path)| path.eq_ignore_ascii_case(client))
}

/// From the first line of `--version`, such as "OpenConnect version v9.21".
fn version(path: &Path) -> Option<String> {
    let mut child = Command::new(path)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + VERSION_LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(VERSION_POLL);
    };
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    if !status.success() {
        return None;
    }
    parse_version(&output)
}

fn parse_version(output: &str) -> Option<String> {
    let version = output
        .lines()
        .next()?
        .strip_prefix("OpenConnect version ")?
        .trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    (!version.is_empty()).then(|| version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comes_from_the_first_line() {
        let output = "OpenConnect version v9.21\r\nUsing GnuTLS 3.8.11. Features present: DTLS\r\n";
        assert_eq!(parse_version(output), Some("9.21".to_owned()));
        assert_eq!(
            parse_version("OpenConnect version 9.21-12-gabcdef\n"),
            Some("9.21-12-gabcdef".to_owned())
        );
        for unreadable in ["", "openconnect 9.21\n", "OpenConnect version \n"] {
            assert_eq!(parse_version(unreadable), None, "{unreadable:?}");
        }
    }
}
