use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use fvm_contract::helper_internal::{AddedRoute, OutsideChanges};
use serde::{Deserialize, Serialize};

use crate::process::os;

const RECORD: &str = "run.json";
/// Written in full, then renamed over the pidfile.
const RECORD_DRAFT: &str = "run.json.tmp";
const SECOND_RECORD: &str = "run.json.2";
const SECOND_RECORD_DRAFT: &str = "run.json.2.tmp";
const CONFIG_PREFIX: &str = "config.";
const VALUES_DIRECTORY: &str = "values";
const VALUES_PREFIX: &str = "values.";
/// Apart from the pidfile, whose name earlier Helpers clean up as their own.
const BYPASS_RECORD: &str = "exception-routes.json";
const BYPASS_RECORD_DRAFT: &str = "exception-routes.json.tmp";

/// What a restarted Helper cleans up if this one dies during a run. A Helper of another build may
/// read it, so its keys stay as they are.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct RunRecord {
    /// The client, and once the tunnel is up the processes it started that the backend owns.
    pub processes: Vec<ProcessIdentity>,
    /// For removing the client's host route to the gateway.
    #[serde(rename = "gatewayIP", default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<Ipv4Addr>,
    #[serde(
        rename = "outsideChanges",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub outside_changes: Option<OutsideChanges>,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordSlot {
    First,
    Second,
}

impl RecordSlot {
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::First => (RECORD, RECORD_DRAFT),
            Self::Second => (SECOND_RECORD, SECOND_RECORD_DRAFT),
        }
    }
}

pub(crate) struct RunFiles {
    pub directory: PathBuf,
    pub config: PathBuf,
    pub values_directory: PathBuf,
    pub values: PathBuf,
    values_draft: PathBuf,
    slot: RecordSlot,
}

impl RunFiles {
    /// The files of run number `run`.
    pub fn new(directory: &Path, run: u64, slot: RecordSlot) -> Self {
        let values_directory = directory.join(VALUES_DIRECTORY);
        Self {
            directory: directory.to_owned(),
            config: directory.join(format!("{CONFIG_PREFIX}{run}")),
            values: values_directory.join(format!("{VALUES_PREFIX}{run}")),
            values_draft: values_directory.join(format!("{VALUES_PREFIX}{run}.tmp")),
            values_directory,
            slot,
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
        let json = serde_json::to_vec(record).expect("a run record serializes");
        let (name, draft) = self.slot.names();
        replace(&self.directory, name, draft, &json)
    }

    pub fn take_first_slot(&mut self) -> Option<PathBuf> {
        self.slot = RecordSlot::First;
        let second = self.directory.join(SECOND_RECORD);
        second.exists().then_some(second)
    }

    /// Deletes them once the run is over. The errors are lines for the Helper's log.
    pub fn remove(&self) -> Vec<String> {
        [
            self.config.clone(),
            self.values.clone(),
            self.values_draft.clone(),
            self.directory.join(self.slot.names().0),
        ]
        .iter()
        .filter_map(|path| remove(path).err())
        .collect()
    }
}

/// Writes `bytes` as `name` in `directory` at once, through `draft`, so a Helper that dies meanwhile
/// leaves the old or the new file.
fn replace(directory: &Path, name: &str, draft: &str, bytes: &[u8]) -> Result<(), Failure> {
    let draft = directory.join(draft);
    // A draft left by a Helper that died while writing is replaced, never followed if a symlink.
    match fs::remove_file(&draft) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(Failure::new(format!("unlink {}", draft.display()), error)),
    }
    let mut file = os::create_private_file(&draft)
        .map_err(|error| Failure::new(format!("open {}", draft.display()), error))?;
    let written = file
        .write_all(bytes)
        .map_err(|error| Failure::new(format!("write {}", draft.display()), error))
        .and_then(|()| {
            fs::rename(&draft, directory.join(name))
                .map_err(|error| Failure::new(format!("rename {}", draft.display()), error))
        });
    if written.is_err() {
        let _ = fs::remove_file(&draft);
    }
    written
}

/// The exception routes the Helper added, or is about to add, and has not removed. A Helper of
/// another build may read it, so its keys stay as they are.
#[derive(Default, Serialize, Deserialize)]
struct BypassRecord {
    routes: Vec<AddedRoute>,
}

/// Records `routes` as the exception routes the Helper added; none removes the record.
pub(crate) fn write_bypass_record(directory: &Path, routes: &[AddedRoute]) -> Result<(), Failure> {
    if routes.is_empty() {
        let path = directory.join(BYPASS_RECORD);
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Failure::new(format!("unlink {}", path.display()), error)),
        };
    }
    let record = BypassRecord {
        routes: routes.to_vec(),
    };
    let json = serde_json::to_vec(&record).expect("an exception route record serializes");
    replace(directory, BYPASS_RECORD, BYPASS_RECORD_DRAFT, &json)
}

/// The exception routes an earlier Helper recorded; `None` without a record, an error if it cannot
/// be read or decoded.
pub(crate) fn read_bypass_record(
    directory: &Path,
) -> Option<Result<Vec<AddedRoute>, serde_json::Error>> {
    let bytes = fs::read(directory.join(BYPASS_RECORD)).ok()?;
    Some(serde_json::from_slice::<BypassRecord>(&bytes).map(|record| record.routes))
}

pub(crate) fn bypass_record_path(directory: &Path) -> PathBuf {
    directory.join(BYPASS_RECORD)
}

pub(crate) fn read_records(
    directory: &Path,
) -> Vec<(PathBuf, Result<RunRecord, serde_json::Error>)> {
    [RECORD, SECOND_RECORD]
        .into_iter()
        .filter_map(|name| {
            let path = directory.join(name);
            let bytes = fs::read(&path).ok()?;
            Some((path, serde_json::from_slice(&bytes)))
        })
        .collect()
}

/// Config files, value files and pidfiles, finished or not, and unfinished exception route records
/// that an earlier Helper left.
pub(crate) fn leftovers(directory: &Path) -> Vec<PathBuf> {
    let run_files = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let name = entry.ok()?.file_name();
            let name = name.to_str()?;
            (name.starts_with(CONFIG_PREFIX)
                || name.starts_with(RECORD)
                || name == BYPASS_RECORD_DRAFT)
                .then(|| directory.join(name))
        });
    let value_files = fs::read_dir(directory.join(VALUES_DIRECTORY))
        .into_iter()
        .flatten()
        .filter_map(|entry| Some(entry.ok()?.path()));
    run_files.chain(value_files).collect()
}

/// Deletes a file; one that is already gone is fine. The error is the line for the Helper's log.
pub(crate) fn remove(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot delete {}: {error}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use std::{env, process};

    use super::*;

    #[test]
    fn each_run_writes_and_removes_only_its_own_pidfile() {
        let directory = env::temp_dir().join(format!("fvm-run-records-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let first = RunFiles::new(&directory, 1, RecordSlot::First);
        let second = RunFiles::new(&directory, 2, RecordSlot::Second);
        let record = |gateway| RunRecord {
            processes: Vec::new(),
            gateway: Some(gateway),
            outside_changes: None,
        };
        let first_gateway = Ipv4Addr::new(192, 0, 2, 1);
        let second_gateway = Ipv4Addr::new(192, 0, 2, 2);
        first.write_record(&record(first_gateway)).unwrap();
        second.write_record(&record(second_gateway)).unwrap();
        let recorded = || {
            read_records(&directory)
                .into_iter()
                .map(|(path, record)| (path, record.unwrap().gateway))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            recorded(),
            [
                (directory.join("run.json"), Some(first_gateway)),
                (directory.join("run.json.2"), Some(second_gateway)),
            ]
        );

        assert!(second.remove().is_empty());
        assert_eq!(
            recorded(),
            [(directory.join("run.json"), Some(first_gateway))]
        );
        second.write_record(&record(second_gateway)).unwrap();
        assert!(first.remove().is_empty());
        assert_eq!(
            recorded(),
            [(directory.join("run.json.2"), Some(second_gateway))]
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn the_second_run_writes_the_first_slot_once_the_first_ends() {
        let directory = env::temp_dir().join(format!("fvm-run-record-move-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let first = RunFiles::new(&directory, 1, RecordSlot::First);
        let mut second = RunFiles::new(&directory, 2, RecordSlot::Second);
        let record = |gateway| RunRecord {
            processes: Vec::new(),
            gateway: Some(gateway),
            outside_changes: None,
        };
        let recorded = || {
            read_records(&directory)
                .into_iter()
                .map(|(path, record)| (path, record.unwrap().gateway))
                .collect::<Vec<_>>()
        };
        let first_gateway = Ipv4Addr::new(192, 0, 2, 1);
        let second_gateway = Ipv4Addr::new(192, 0, 2, 2);
        first.write_record(&record(first_gateway)).unwrap();
        second.write_record(&record(second_gateway)).unwrap();
        assert!(first.remove().is_empty());
        let second_record = second.take_first_slot().unwrap();
        assert_eq!(second_record, directory.join("run.json.2"));
        second.write_record(&record(second_gateway)).unwrap();
        assert_eq!(
            recorded(),
            [
                (directory.join("run.json"), Some(second_gateway)),
                (directory.join("run.json.2"), Some(second_gateway)),
            ]
        );
        remove(&second_record).unwrap();
        assert_eq!(
            recorded(),
            [(directory.join("run.json"), Some(second_gateway))]
        );
        let later_gateway = Ipv4Addr::new(192, 0, 2, 3);
        second.write_record(&record(later_gateway)).unwrap();
        assert_eq!(
            recorded(),
            [(directory.join("run.json"), Some(later_gateway))]
        );
        assert!(second.remove().is_empty());
        assert!(recorded().is_empty());

        let mut unrecorded = RunFiles::new(&directory, 3, RecordSlot::Second);
        assert_eq!(unrecorded.take_first_slot(), None);
        unrecorded.write_record(&record(second_gateway)).unwrap();
        assert_eq!(
            recorded(),
            [(directory.join("run.json"), Some(second_gateway))]
        );
        assert!(unrecorded.remove().is_empty());
        fs::remove_dir_all(&directory).unwrap();
    }
}
