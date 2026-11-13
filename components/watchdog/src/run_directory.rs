//! The Helper's root-only run directory: the config file of the current run, deleted after it, and
//! the pidfile, which tells a restarted Helper what an earlier one left running.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::os;

const RECORD: &str = "run.json";
/// Written in full, then renamed over the pidfile.
const RECORD_DRAFT: &str = "run.json.tmp";
const CONFIG_PREFIX: &str = "config.";

/// What a restarted Helper cleans up if this one dies during a run. A Helper of another build may
/// read it, so its keys stay as they are.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct RunRecord {
    /// The client, and once the tunnel is up the processes it started that the backend owns.
    pub processes: Vec<ProcessIdentity>,
    /// For removing the client's host route to the gateway.
    #[serde(rename = "gatewayIP", default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<Ipv4Addr>,
}

/// A process told apart by pid, start time and executable path: a pid can be reused, so only a
/// process that matches all three is the one an earlier Helper recorded. As the pidfile records it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessIdentity {
    pub pid: u32,
    /// When it started, since 1970. An OS that tells only whole seconds leaves the microseconds 0.
    pub start_seconds: u64,
    pub start_microseconds: u64,
    pub path: String,
}

impl ProcessIdentity {
    /// Whether the recorded process still runs: same pid, start time and path, with the Helper's
    /// rights.
    pub fn is_running(&self) -> bool {
        os::process(self.pid).as_ref() == Some(self)
    }
}

/// A failed file operation: "mkdir <path>: Permission denied (os error 13)".
#[derive(Debug)]
pub struct Failure {
    operation: String,
    error: io::Error,
}

impl Failure {
    pub(crate) fn new(operation: String, error: io::Error) -> Self {
        Self { operation, error }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.operation, self.error)
    }
}

/// Where one run keeps its files: its config file, if its launch has one, and the pidfile.
pub(crate) struct RunFiles {
    pub directory: PathBuf,
    pub config: PathBuf,
}

impl RunFiles {
    /// The files of run number `run`.
    pub fn new(directory: &Path, run: u64) -> Self {
        Self {
            directory: directory.to_owned(),
            config: directory.join(format!("{CONFIG_PREFIX}{run}")),
        }
    }

    /// Writes the config file, readable only by the Helper; an existing file or symlink at its
    /// path is an error.
    pub fn write_config(&self, contents: &str) -> Result<(), Failure> {
        let path = &self.config;
        let mut file = os::create_private_file(path)
            .map_err(|error| Failure::new(format!("open {}", path.display()), error))?;
        file.write_all(contents.as_bytes()).map_err(|error| {
            let _ = fs::remove_file(path);
            Failure::new(format!("write {}", path.display()), error)
        })
    }

    /// Replaces the pidfile at once, so a Helper that dies meanwhile leaves the old or the new one.
    pub fn write_record(&self, record: &RunRecord) -> Result<(), Failure> {
        let draft = self.directory.join(RECORD_DRAFT);
        let json = serde_json::to_vec(record).expect("a run record serializes");
        // A draft left by a Helper that died while writing is replaced, never followed if a symlink.
        match fs::remove_file(&draft) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(Failure::new(format!("unlink {}", draft.display()), error)),
        }
        let mut file = os::create_private_file(&draft)
            .map_err(|error| Failure::new(format!("open {}", draft.display()), error))?;
        let written = file
            .write_all(&json)
            .map_err(|error| Failure::new(format!("write {}", draft.display()), error))
            .and_then(|()| {
                fs::rename(&draft, record_path(&self.directory))
                    .map_err(|error| Failure::new(format!("rename {}", draft.display()), error))
            });
        if written.is_err() {
            let _ = fs::remove_file(&draft);
        }
        written
    }

    /// Deletes both once the run is over. The errors are lines for the Helper's log.
    pub fn remove(&self) -> Vec<String> {
        [self.config.clone(), record_path(&self.directory)]
            .iter()
            .filter_map(|path| remove(path).err())
            .collect()
    }
}

pub(crate) fn record_path(directory: &Path) -> PathBuf {
    directory.join(RECORD)
}

/// `None` without a pidfile; an error if it cannot be decoded.
pub(crate) fn read_record(directory: &Path) -> Option<Result<RunRecord, serde_json::Error>> {
    let bytes = fs::read(record_path(directory)).ok()?;
    Some(serde_json::from_slice(&bytes))
}

/// Config files and pidfiles, finished or not, that an earlier Helper left.
pub(crate) fn leftovers(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let name = entry.ok()?.file_name();
            let name = name.to_str()?;
            (name.starts_with(CONFIG_PREFIX) || name.starts_with(RECORD))
                .then(|| directory.join(name))
        })
        .collect()
}

/// Deletes a file; one that is already gone is fine. The error is the line for the Helper's log.
pub(crate) fn remove(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot delete {}: {error}", path.display())),
    }
}
