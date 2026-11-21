#[cfg(test)]
pub(crate) mod tests;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as os;
#[cfg(windows)]
use windows as os;

pub use os::RECORDS_DIRECTORY;

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str;
use std::sync::Arc;

use fvm_contract::app_helper::{
    CHECKSUMS_FILE, CHECKSUMS_SIGNATURE_FILE, UPDATE_RESULT_SUFFIX, UPDATE_STARTED_SUFFIX,
    UpdatePrepareRequest, UpdateRefusal, Version, installer_version,
};
use fvm_contract::helper_internal::LogLevel;
use ring::digest::{Context, SHA256};
use ring::signature::{ED25519, UnparsedPublicKey};

use crate::recovery::run_directory;

pub const PUBLIC_KEY: [u8; 32] = [
    0x3d, 0x5f, 0xad, 0x4b, 0xd3, 0x6c, 0xe5, 0xe1, 0x76, 0xe0, 0x66, 0x67, 0x34, 0x10, 0x91, 0x8a,
    0xd3, 0x39, 0xfc, 0x2d, 0x6b, 0x58, 0x84, 0x4c, 0x3e, 0xb0, 0x49, 0x3d, 0xbb, 0xb3, 0xb6, 0xcb,
];
const VERSION: &str = include_str!("../../../../../VERSION");
const PREPARED: &str = "prepared";
const DRAFT_SUFFIX: &str = ".tmp";
const RUN_SUFFIX: &str = ".run";

pub type Log = Arc<dyn Fn(LogLevel, String) + Send + Sync>;

pub struct Installer {
    records: PathBuf,
    staging: PathBuf,
    public_key: [u8; 32],
    current: Version,
    runner: Option<os::Runner>,
}

pub struct Prepared {
    pub installer: String,
    pub version: Version,
}

pub struct Started {
    pub installer: String,
    pub runner: u32,
    pub installer_process: Option<u32>,
}

#[derive(Debug)]
pub enum NotDone {
    Refused(UpdateRefusal, String),
    Failed(String),
}

#[derive(Clone)]
struct Staged {
    installer: PathBuf,
    checksums: PathBuf,
    signature: PathBuf,
}

impl Staged {
    fn new(staging: &Path, installer: &str) -> Self {
        Self {
            installer: staging.join(installer),
            checksums: staging.join(CHECKSUMS_FILE),
            signature: staging.join(CHECKSUMS_SIGNATURE_FILE),
        }
    }
}

impl Installer {
    pub fn new(records: PathBuf, public_key: [u8; 32]) -> Self {
        Self {
            staging: records.join(PREPARED),
            records,
            public_key,
            current: Version::parse(VERSION.trim()).expect("VERSION holds the app's version"),
            runner: None,
        }
    }

    pub fn prepare(&mut self, request: &UpdatePrepareRequest) -> Result<Prepared, NotDone> {
        let sources = [
            &request.installer_path,
            &request.checksums_path,
            &request.signature_path,
        ]
        .map(Path::new);
        if self.installing() {
            return Err(NotDone::Refused(
                UpdateRefusal::AlreadyInstalling,
                "an update is already being installed".to_owned(),
            ));
        }
        let Some(installer) = sources[0].file_name().and_then(OsStr::to_str) else {
            return Err(NotDone::Refused(
                UpdateRefusal::FilesUnreadable,
                format!("{} names no file", sources[0].display()),
            ));
        };
        os::create_records_directory(&self.records).map_err(|error| {
            NotDone::Failed(format!(
                "could not create {}: {error}",
                self.records.display()
            ))
        })?;
        run_directory::prepare(&self.staging)
            .map_err(|failure| NotDone::Failed(failure.to_string()))?;
        self.remove_prepared().map_err(NotDone::Failed)?;
        let staged = Staged::new(&self.staging, installer);
        let draft = self.staging.join(format!("{installer}{DRAFT_SUFFIX}"));
        let verified = self
            .verify(sources, &staged, &draft, installer)
            .and_then(|version| {
                fs::rename(&draft, &staged.installer).map_err(|error| {
                    NotDone::Failed(format!(
                        "could not rename {} to {installer}: {error}",
                        draft.display()
                    ))
                })?;
                Ok(version)
            });
        if verified.is_err() {
            let _ = self.remove_prepared();
        }
        verified.map(|version| Prepared {
            installer: installer.to_owned(),
            version,
        })
    }

    pub fn recover(&mut self, log: &Log) {
        if let Some(runner) = os::recover(self, log) {
            self.runner = Some(runner);
        }
    }

    pub fn install(&mut self, install_id: u64, log: &Log) -> Result<Started, NotDone> {
        if self.installing() {
            return Err(NotDone::Refused(
                UpdateRefusal::AlreadyInstalling,
                "an update is already being installed".to_owned(),
            ));
        }
        let Some(prepared) = self.prepared()? else {
            return Err(NotDone::Refused(
                UpdateRefusal::NothingPrepared,
                "no update is prepared".to_owned(),
            ));
        };
        let staged = Staged::new(&self.staging, &prepared.installer);
        let started = self.record(install_id)?;
        let runner =
            os::launch(self, &staged, install_id, prepared.version, log).map_err(|error| {
                let _ = fs::remove_file(&started);
                NotDone::Failed(format!("could not start the installer: {error}"))
            })?;
        let started = Started {
            installer: prepared.installer,
            runner: runner.id(),
            installer_process: runner.installer_process(),
        };
        self.runner = Some(runner);
        Ok(started)
    }

    pub fn prepared(&self) -> Result<Option<Prepared>, NotDone> {
        let unreadable = |error: io::Error| {
            NotDone::Failed(format!(
                "could not read {}: {error}",
                self.staging.display()
            ))
        };
        let entries = match fs::read_dir(&self.staging) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            entries => entries.map_err(unreadable)?,
        };
        let mut prepared = None;
        for entry in entries {
            let name = entry.map_err(unreadable)?.file_name();
            if let Some(name) = name.to_str()
                && let Some(version) = installer_version(name, os::INSTALLER_EXTENSION)
            {
                prepared = Some(Prepared {
                    installer: name.to_owned(),
                    version,
                });
                break;
            }
        }
        match prepared {
            Some(outdated) if outdated.version <= self.current => {
                self.remove_prepared().map_err(NotDone::Failed)?;
                Err(NotDone::Refused(
                    UpdateRefusal::VersionNotNewer,
                    format!(
                        "removed the prepared update {}: not newer than {}",
                        outdated.version, self.current
                    ),
                ))
            }
            prepared => Ok(prepared),
        }
    }

    pub fn discard(&mut self) -> Result<(), NotDone> {
        if self.installing() {
            return Err(NotDone::Refused(
                UpdateRefusal::AlreadyInstalling,
                "an update is being installed".to_owned(),
            ));
        }
        self.remove_prepared().map_err(NotDone::Failed)
    }

    fn remove_prepared(&self) -> Result<(), String> {
        let unreadable =
            |error: io::Error| format!("could not read {}: {error}", self.staging.display());
        let entries = match fs::read_dir(&self.staging) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            entries => entries.map_err(unreadable)?,
        };
        for entry in entries {
            run_directory::remove(&entry.map_err(unreadable)?.path())?;
        }
        Ok(())
    }

    fn installing(&mut self) -> bool {
        let running = self.runner.as_mut().is_some_and(os::Runner::running);
        if !running {
            self.runner = None;
        }
        running
    }

    fn verify(
        &self,
        sources: [&Path; 3],
        staged: &Staged,
        draft: &Path,
        installer: &str,
    ) -> Result<Version, NotDone> {
        let copies = [
            (draft, "the installer"),
            (staged.checksums.as_path(), CHECKSUMS_FILE),
            (staged.signature.as_path(), CHECKSUMS_SIGNATURE_FILE),
        ];
        for (source, (copy, what)) in sources.into_iter().zip(copies) {
            copy_file(source, copy, what)?;
        }
        let read = |path: &Path| {
            fs::read(path).map_err(|error| {
                NotDone::Failed(format!("could not read {}: {error}", path.display()))
            })
        };
        let checksums = read(&staged.checksums)?;
        let signature = read(&staged.signature)?;
        if UnparsedPublicKey::new(&ED25519, &self.public_key)
            .verify(&checksums, &signature)
            .is_err()
        {
            return Err(NotDone::Refused(
                UpdateRefusal::SignatureInvalid,
                format!("{CHECKSUMS_SIGNATURE_FILE} does not verify"),
            ));
        }
        let listed = listed_checksum(&checksums, installer)
            .map_err(|message| NotDone::Refused(UpdateRefusal::ChecksumMismatch, message))?;
        let actual = sha256(draft).map_err(|error| {
            NotDone::Failed(format!("could not read {}: {error}", draft.display()))
        })?;
        if actual != listed {
            return Err(NotDone::Refused(
                UpdateRefusal::ChecksumMismatch,
                format!("{installer} does not match {CHECKSUMS_FILE}"),
            ));
        }
        let Some(version) = installer_version(installer, os::INSTALLER_EXTENSION) else {
            return Err(NotDone::Refused(
                UpdateRefusal::VersionNotNewer,
                format!(
                    "{installer} is not named FortiVPNManager-<version>.{}",
                    os::INSTALLER_EXTENSION
                ),
            ));
        };
        if version <= self.current {
            return Err(NotDone::Refused(
                UpdateRefusal::VersionNotNewer,
                format!("{version} is not newer than {}", self.current),
            ));
        }
        Ok(version)
    }

    fn record(&self, install_id: u64) -> Result<PathBuf, NotDone> {
        let failed = |error: io::Error| {
            NotDone::Failed(format!(
                "could not write the update records in {}: {error}",
                self.records.display()
            ))
        };
        os::create_records_directory(&self.records).map_err(failed)?;
        for entry in fs::read_dir(&self.records).map_err(failed)? {
            let path = entry.map_err(failed)?.path();
            let earlier = path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| {
                    [UPDATE_STARTED_SUFFIX, UPDATE_RESULT_SUFFIX, RUN_SUFFIX]
                        .iter()
                        .any(|suffix| name.ends_with(suffix))
                });
            if earlier {
                fs::remove_file(&path).map_err(failed)?;
            }
        }
        let started = self
            .records
            .join(format!("{install_id}{UPDATE_STARTED_SUFFIX}"));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&started)
            .map_err(failed)?;
        Ok(started)
    }
}

fn copy_file(source: &Path, copy: &Path, what: &str) -> Result<(), NotDone> {
    let unreadable = |error: io::Error| {
        NotDone::Refused(
            UpdateRefusal::FilesUnreadable,
            format!("could not read {what}: {error}"),
        )
    };
    let unwritable = |error: io::Error| {
        NotDone::Failed(format!(
            "could not copy {what} to {}: {error}",
            copy.display()
        ))
    };
    let mut from = os::open_source(source).map_err(unreadable)?;
    if !from.metadata().map_err(unreadable)?.is_file() {
        return Err(NotDone::Refused(
            UpdateRefusal::FilesUnreadable,
            format!("{what} is not a regular file"),
        ));
    }
    match fs::remove_file(copy) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(unwritable(error)),
    }
    let mut to = run_directory::create_private_file(copy).map_err(unwritable)?;
    let mut buffer = vec![0; 65_536];
    loop {
        let count = match from.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(unreadable(error)),
        };
        to.write_all(&buffer[..count]).map_err(unwritable)?;
    }
}

fn listed_checksum<'a>(list: &'a [u8], installer: &str) -> Result<&'a str, String> {
    let malformed = || format!("{CHECKSUMS_FILE} is not a sha256sum list");
    let lines = str::from_utf8(list)
        .ok()
        .and_then(|text| text.strip_suffix('\n'))
        .ok_or_else(malformed)?;
    let mut listed = None;
    for line in lines.split('\n') {
        let (hash, name) = line
            .split_at_checked(64)
            .and_then(|(hash, rest)| Some((hash, rest.strip_prefix("  ")?)))
            .filter(|(hash, name)| {
                !name.is_empty()
                    && hash
                        .bytes()
                        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            })
            .ok_or_else(malformed)?;
        if name == installer && listed.replace(hash).is_some() {
            return Err(format!("{CHECKSUMS_FILE} lists {installer} more than once"));
        }
    }
    listed.ok_or_else(|| format!("{CHECKSUMS_FILE} does not list {installer}"))
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut context = Context::new(&SHA256);
    let mut buffer = vec![0; 65_536];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => context.update(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(context
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
