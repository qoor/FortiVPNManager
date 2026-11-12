//! The Helper's root-only run directory: the config file of the current run, deleted after it, and
//! the pidfile, which tells a restarted Helper what an earlier one left running.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::platform::{Platform, ProcessIdentity};

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

/// A failed file operation: "mkdir <path>: Permission denied (os error 13)".
pub struct Failure {
    operation: String,
    error: io::Error,
}

impl Failure {
    pub fn new(operation: String, error: io::Error) -> Self {
        Self { operation, error }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.operation, self.error)
    }
}

/// Where run number `run` keeps its config file.
pub(crate) fn config_path(directory: &Path, run: u64) -> PathBuf {
    directory.join(format!("{CONFIG_PREFIX}{run}"))
}

/// Writes a new file only the Helper can read; an existing file or symlink at `path` is an error.
pub(crate) fn write_config(
    platform: &impl Platform,
    path: &Path,
    contents: &str,
) -> Result<(), Failure> {
    let mut file = platform
        .create_private_file(path)
        .map_err(|error| Failure::new(format!("open {}", path.display()), error))?;
    file.write_all(contents.as_bytes()).map_err(|error| {
        let _ = fs::remove_file(path);
        Failure::new(format!("write {}", path.display()), error)
    })
}

pub(crate) fn record_path(directory: &Path) -> PathBuf {
    directory.join(RECORD)
}

/// Replaces the pidfile at once, so a Helper that dies meanwhile leaves the old or the new one.
pub(crate) fn write_record(
    platform: &impl Platform,
    directory: &Path,
    record: &RunRecord,
) -> Result<(), Failure> {
    let draft = directory.join(RECORD_DRAFT);
    let json = serde_json::to_vec(record).expect("a run record serializes");
    // A draft left by a Helper that died while writing is replaced, never followed if a symlink.
    match fs::remove_file(&draft) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(Failure::new(format!("unlink {}", draft.display()), error)),
    }
    let mut file = platform
        .create_private_file(&draft)
        .map_err(|error| Failure::new(format!("open {}", draft.display()), error))?;
    let written = file
        .write_all(&json)
        .map_err(|error| Failure::new(format!("write {}", draft.display()), error))
        .and_then(|()| {
            fs::rename(&draft, record_path(directory))
                .map_err(|error| Failure::new(format!("rename {}", draft.display()), error))
        });
    if written.is_err() {
        let _ = fs::remove_file(&draft);
    }
    written
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
