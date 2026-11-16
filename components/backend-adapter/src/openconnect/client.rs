//! Which openconnect executable the Helper may run, and which leftover processes are ours. Only the
//! copy at the OS's place is run, which only administrators, or on macOS Homebrew's owner, can
//! change (`platform`).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use fvm_contract::backend::{
    ClientCheckWait, ClientExecutable, ClientProblem, ClientProblemKind, UntestedVersion,
};

use super::platform::{self, INSTALL_HINT};

/// The release series whose output wording the adapter was checked against.
const TESTED_SERIES: &str = "9.21";
/// A `Limited` check's limit: `--version` prints and exits at once; a client that does not is not
/// used.
const VERSION_LIMIT: Duration = Duration::from_secs(5);
const VERSION_POLL: Duration = Duration::from_millis(50);

static KNOWN_VERSION: Mutex<Option<(FileStamp, String)>> = Mutex::new(None);

#[derive(PartialEq)]
struct FileStamp {
    path: PathBuf,
    modified: SystemTime,
    size: u64,
}

pub(super) fn locate(wait: ClientCheckWait) -> Result<ClientExecutable, ClientProblem> {
    let path = platform::executable()?;
    let Some(version) = known_version(&path, wait) else {
        return Err(ClientProblem {
            kind: ClientProblemKind::NotExecutable {},
            path,
        });
    };
    Ok(ClientExecutable { path, version })
}

/// The English text the current app shows as it is.
pub(super) fn describe(problem: &ClientProblem) -> String {
    let path = problem.path.display();
    match problem.kind {
        ClientProblemKind::NotFound {} => {
            format!("openconnect not found at {path}; {INSTALL_HINT}")
        }
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
            format!("openconnect is not executable or did not report its version: {path}")
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

pub(super) fn is_own_executable(path: &Path) -> bool {
    platform::is_own_executable(path)
}

fn known_version(path: &Path, wait: ClientCheckWait) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    let stamp = FileStamp {
        path: path.to_owned(),
        modified: metadata.modified().ok()?,
        size: metadata.len(),
    };
    let mut known = KNOWN_VERSION.lock().expect("a version check never panics");
    if let Some((checked, version)) = known.as_ref()
        && *checked == stamp
    {
        return Some(version.clone());
    }
    let version = version(path, wait)?;
    *known = Some((stamp, version.clone()));
    Some(version)
}

/// From the first line of `--version`, such as "OpenConnect version v9.21".
fn version(path: &Path, wait: ClientCheckWait) -> Option<String> {
    let mut child = Command::new(path)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = match wait {
        ClientCheckWait::Limited => Some(Instant::now() + VERSION_LIMIT),
        ClientCheckWait::UntilExit => None,
    };
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
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

    #[cfg(unix)]
    #[test]
    fn the_version_is_asked_again_only_when_the_file_changes() {
        use std::os::unix::fs::PermissionsExt;
        use std::process;

        let directory = std::env::temp_dir().join(format!("fvm-oc-version-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let runs = directory.join("runs");
        let client = directory.join("openconnect");
        let write = |version: &str| {
            fs::write(
                &client,
                format!(
                    "#!/bin/sh\necho run >> '{}'\necho 'OpenConnect version v{version}'\n",
                    runs.display()
                ),
            )
            .unwrap();
            fs::set_permissions(&client, fs::Permissions::from_mode(0o755)).unwrap();
        };
        let run_count = || {
            fs::read_to_string(&runs)
                .unwrap_or_default()
                .lines()
                .count()
        };
        write("9.21");
        for _ in 0..3 {
            assert_eq!(
                known_version(&client, ClientCheckWait::Limited).as_deref(),
                Some("9.21")
            );
        }
        assert_eq!(run_count(), 1);
        write("9.210");
        assert_eq!(
            known_version(&client, ClientCheckWait::Limited).as_deref(),
            Some("9.210")
        );
        assert_eq!(run_count(), 2);
        fs::remove_dir_all(&directory).unwrap();
    }

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
