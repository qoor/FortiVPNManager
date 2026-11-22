use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use fvm_contract::helper_internal::{AddedRoute, OutsideChanges};
use serde::{Deserialize, Serialize};

const RECORD: &str = "run.json";
/// Written in full, then renamed over the run record.
const RECORD_DRAFT: &str = "run.json.tmp";
const SECOND_RECORD: &str = "run.json.2";
const SECOND_RECORD_DRAFT: &str = "run.json.2.tmp";
/// Apart from the run record, whose name earlier Helpers clean up as their own.
const BYPASS_RECORD: &str = "exception-routes.json";
const BYPASS_RECORD_DRAFT: &str = "exception-routes.json.tmp";
const UNREVERTED_RECORD: &str = "unreverted-changes.json";
const UNREVERTED_RECORD_DRAFT: &str = "unreverted-changes.json.tmp";

/// What a restarted Helper cleans up if this one dies during a run.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct RunRecord {
    #[serde(
        rename = "outsideChanges",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub outside_changes: Option<OutsideChanges>,
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
    directory: PathBuf,
    slot: RecordSlot,
}

impl RunFiles {
    pub fn new(directory: &Path, slot: RecordSlot) -> Self {
        Self {
            directory: directory.to_owned(),
            slot,
        }
    }

    /// Replaces the run record at once, so a Helper that dies meanwhile leaves the old or the new
    /// one.
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

    /// Deletes the run record once the run is over. The error is the line for the Helper's log.
    pub fn remove(&self) -> Result<(), String> {
        remove(&self.directory.join(self.slot.names().0))
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
    let mut file = create_private_file(&draft)
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

#[derive(Serialize, Deserialize)]
struct UnrevertedRecord {
    changes: Vec<OutsideChanges>,
}

pub(crate) fn write_unreverted_record(
    directory: &Path,
    changes: &[OutsideChanges],
) -> Result<(), Failure> {
    if changes.is_empty() {
        let path = directory.join(UNREVERTED_RECORD);
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Failure::new(format!("unlink {}", path.display()), error)),
        };
    }
    let record = UnrevertedRecord {
        changes: changes.to_vec(),
    };
    let json = serde_json::to_vec(&record).expect("an unreverted change record serializes");
    replace(directory, UNREVERTED_RECORD, UNREVERTED_RECORD_DRAFT, &json)
}

pub(crate) fn read_unreverted_record(
    directory: &Path,
) -> Option<Result<Vec<OutsideChanges>, serde_json::Error>> {
    let bytes = fs::read(directory.join(UNREVERTED_RECORD)).ok()?;
    Some(serde_json::from_slice::<UnrevertedRecord>(&bytes).map(|record| record.changes))
}

pub(crate) fn unreverted_record_path(directory: &Path) -> PathBuf {
    directory.join(UNREVERTED_RECORD)
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

/// Deletes a file; one that is already gone is fine. The error is the line for the Helper's log.
pub(crate) fn remove(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot delete {}: {error}", path.display())),
    }
}

/// Creates the run directory if needed and makes sure it is a real directory owned by this
/// process's user with mode 0700, so nobody else can place or read files in it.
#[cfg(unix)]
pub(crate) fn prepare(directory: &Path) -> Result<(), Failure> {
    let shown = directory.display();
    match fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(Failure::new(format!("mkdir {shown}"), error)),
    }
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| Failure::new(format!("lstat {shown}"), error))?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(Failure::new(
            format!("{shown} is not a directory owned by the helper"),
            io::Error::from_raw_os_error(rustix::io::Errno::PERM.raw_os_error()),
        ));
    }
    if metadata.mode() & 0o777 != 0o700 {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| Failure::new(format!("chmod {shown}"), error))?;
    }
    Ok(())
}

/// The directory inherits its access rights from where it is created, which only the system and
/// administrators can reach; so only the last level is created, and it must be a real directory.
#[cfg(windows)]
pub(crate) fn prepare(directory: &Path) -> Result<(), Failure> {
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

/// A new file with mode 0600; an existing file or symlink at `path` is an error.
#[cfg(unix)]
pub fn create_private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// A new file with the run directory's access rights; an existing file or link at `path` is an
/// error.
#[cfg(windows)]
pub fn create_private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(test)]
mod tests {
    use std::{env, process};

    use super::*;

    fn record(dns_service_id: &str) -> RunRecord {
        RunRecord {
            outside_changes: Some(OutsideChanges {
                dns_service_id: dns_service_id.to_owned(),
                routes: Vec::new(),
            }),
        }
    }

    fn recorded(directory: &Path) -> Vec<(PathBuf, String)> {
        read_records(directory)
            .into_iter()
            .map(|(path, record)| {
                let changes = record.unwrap().outside_changes.unwrap();
                (path, changes.dns_service_id)
            })
            .collect()
    }

    fn pair(directory: &Path, name: &str, dns_service_id: &str) -> (PathBuf, String) {
        (directory.join(name), dns_service_id.to_owned())
    }

    #[test]
    fn each_run_writes_and_removes_only_its_own_record() {
        let directory = env::temp_dir().join(format!("fvm-run-records-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let first = RunFiles::new(&directory, RecordSlot::First);
        let second = RunFiles::new(&directory, RecordSlot::Second);
        first.write_record(&record("first")).unwrap();
        second.write_record(&record("second")).unwrap();
        assert_eq!(
            recorded(&directory),
            [
                pair(&directory, "run.json", "first"),
                pair(&directory, "run.json.2", "second"),
            ]
        );

        second.remove().unwrap();
        assert_eq!(
            recorded(&directory),
            [pair(&directory, "run.json", "first")]
        );
        second.write_record(&record("second")).unwrap();
        first.remove().unwrap();
        assert_eq!(
            recorded(&directory),
            [pair(&directory, "run.json.2", "second")]
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn the_second_run_writes_the_first_slot_once_the_first_ends() {
        let directory = env::temp_dir().join(format!("fvm-run-record-move-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let first = RunFiles::new(&directory, RecordSlot::First);
        let mut second = RunFiles::new(&directory, RecordSlot::Second);
        first.write_record(&record("first")).unwrap();
        second.write_record(&record("second")).unwrap();
        first.remove().unwrap();
        let second_record = second.take_first_slot().unwrap();
        assert_eq!(second_record, directory.join("run.json.2"));
        second.write_record(&record("second")).unwrap();
        assert_eq!(
            recorded(&directory),
            [
                pair(&directory, "run.json", "second"),
                pair(&directory, "run.json.2", "second"),
            ]
        );
        remove(&second_record).unwrap();
        assert_eq!(
            recorded(&directory),
            [pair(&directory, "run.json", "second")]
        );
        second.write_record(&record("later")).unwrap();
        assert_eq!(
            recorded(&directory),
            [pair(&directory, "run.json", "later")]
        );
        second.remove().unwrap();
        assert_eq!(recorded(&directory), []);

        let mut unrecorded = RunFiles::new(&directory, RecordSlot::Second);
        assert_eq!(unrecorded.take_first_slot(), None);
        unrecorded.write_record(&record("second")).unwrap();
        assert_eq!(
            recorded(&directory),
            [pair(&directory, "run.json", "second")]
        );
        unrecorded.remove().unwrap();
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_record_of_an_earlier_build_is_read_for_its_outside_changes() {
        let directory = env::temp_dir().join(format!("fvm-run-record-old-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("run.json"),
            r#"{"processes":[],"gatewayIP":"203.0.113.10","outsideChanges":{"dnsServiceID":"svc","routes":[]}}"#,
        )
        .unwrap();
        assert_eq!(recorded(&directory), [pair(&directory, "run.json", "svc")]);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn the_run_directory_must_be_a_real_directory() {
        let base = env::temp_dir().join(format!("fvm-run-directory-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let directory = base.join("run");
        for _ in 0..2 {
            if let Err(failure) = prepare(&directory) {
                panic!("{failure}");
            }
        }
        let file = directory.join("record");
        create_private_file(&file).unwrap();
        assert!(create_private_file(&file).is_err());
        let not_directory = base.join("file");
        fs::write(&not_directory, b"").unwrap();
        assert!(prepare(&not_directory).is_err());
        fs::remove_dir_all(&base).unwrap();
    }
}
