//! config.yaml on disk: reading it when it changes, keeping the last settings that passed, and
//! replacing it atomically when the app changes a setting.
//!
//! A symlinked file stays a symlink: the write replaces its target. A file a newer app wrote, or
//! one that does not pass, is never written over. When the file changes between reading and
//! renaming, the whole update runs again.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use fvm_contract::app_internal::problem::{
    ConfigFileError, ConfigProblem, ConfigProblemKind, ConfigWarning, ConfigWarningKind,
};
use fvm_core::config::{self, Settings, Update, UpdateError};

use crate::{client, files};

/// How often the file is looked at for edits made elsewhere; also on app activation.
pub(crate) const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const UPDATE_ATTEMPTS: u32 = 3;

/// What tells a changed file from an unchanged one without reading it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Fingerprint {
    real_path: PathBuf,
    inode: u64,
    size: u64,
    modified: Option<std::time::SystemTime>,
}

pub(crate) struct ConfigFile {
    path: PathBuf,
    /// The last settings that passed; the defaults until a file did.
    settings: Settings,
    exists: bool,
    load_error: Option<ConfigFileError>,
    warnings: Vec<ConfigWarning>,
    seen: Option<Fingerprint>,
}

impl ConfigFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            settings: Settings::default(),
            exists: false,
            load_error: None,
            warnings: Vec::new(),
            seen: None,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn settings(&self) -> &Settings {
        &self.settings
    }

    pub(crate) fn exists(&self) -> bool {
        self.exists
    }

    /// The file is in use without an error.
    pub(crate) fn is_valid(&self) -> bool {
        self.load_error.is_none()
    }

    pub(crate) fn load_error(&self) -> Option<&ConfigFileError> {
        self.load_error.as_ref()
    }

    pub(crate) fn warnings(&self) -> &[ConfigWarning] {
        &self.warnings
    }

    /// Reads the file when it may have changed; true when the settings, the error or the warnings
    /// changed. A file that does not pass keeps the last settings that did.
    pub(crate) fn poll(&mut self) -> bool {
        let fingerprint = match fingerprint(&self.path) {
            Ok(fingerprint) => fingerprint,
            Err(error) => return self.apply_error(error),
        };
        if fingerprint.is_some() && fingerprint == self.seen {
            return false;
        }
        self.seen = fingerprint.clone();
        let before = (
            self.settings.clone(),
            self.exists,
            self.load_error.clone(),
            self.warnings.clone(),
        );
        match read(&self.path) {
            Ok(None) => {
                self.exists = false;
                self.load_error = None;
                self.warnings.clear();
                self.settings = Settings::default();
            }
            Ok(Some((bytes, file_warnings))) => {
                self.exists = true;
                match config::read(&bytes, &client::rules()) {
                    Ok(loaded) => {
                        self.settings = loaded.settings;
                        self.load_error = None;
                        self.warnings = loaded.warnings;
                        self.warnings.extend(file_warnings);
                    }
                    Err(problem) => {
                        self.load_error = Some(ConfigFileError::UnreadableFile {
                            path: display(&self.path),
                            problem,
                        });
                        self.warnings = file_warnings;
                    }
                }
            }
            Err(error) => {
                self.exists = true;
                self.load_error = Some(error);
            }
        }
        before
            != (
                self.settings.clone(),
                self.exists,
                self.load_error.clone(),
                self.warnings.clone(),
            )
    }

    fn apply_error(&mut self, error: ConfigFileError) -> bool {
        let changed = self.load_error.as_ref() != Some(&error);
        self.load_error = Some(error);
        self.seen = None;
        changed
    }

    /// Applies `change` to the latest file and writes the result when it differs; the settings
    /// in use follow what was written. Returns whether anything was written.
    pub(crate) fn update(
        &mut self,
        change: impl Fn(&mut Settings),
    ) -> Result<bool, ConfigFileError> {
        for attempt in 1..=UPDATE_ATTEMPTS {
            // Taken before reading: a change in between then fails the write instead of being lost.
            let before = fingerprint(&self.path)?;
            let current = read(&self.path)?;
            let result = config::update(
                current.as_ref().map(|(bytes, _)| bytes.as_slice()),
                &client::rules(),
                &change,
            );
            let (settings, text) = match result {
                Ok(Update::Unchanged(_)) => return Ok(false),
                Ok(Update::Write { settings, text }) => (settings, text),
                Err(UpdateError::File(problem)) => {
                    return Err(ConfigFileError::UnreadableFile {
                        path: display(&self.path),
                        problem,
                    });
                }
                Err(UpdateError::Settings(problem)) => {
                    return Err(ConfigFileError::InvalidSettings { problem });
                }
            };
            match write(&self.path, text.as_bytes(), before.as_ref()) {
                Ok(write_warnings) => {
                    self.settings = settings;
                    self.exists = true;
                    self.load_error = None;
                    self.warnings = write_warnings;
                    self.seen = fingerprint(&self.path)?;
                    return Ok(true);
                }
                Err(ConfigFileError::KeptChanging { .. }) if attempt < UPDATE_ATTEMPTS => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!("the last attempt returns")
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn io_error(operation: &str, path: &Path, error: &io::Error) -> ConfigFileError {
    ConfigFileError::Io {
        operation: operation.into(),
        path: display(path),
        errno: error.raw_os_error().unwrap_or(0),
    }
}

/// The resolved file's identity, or `None` when there is no file (a dangling symlink included).
fn fingerprint(path: &Path) -> Result<Option<Fingerprint>, ConfigFileError> {
    let real_path = match fs::canonicalize(path) {
        Ok(real) => real,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(io_error("realpath", path, &error)),
    };
    let metadata = match fs::metadata(&real_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("stat", &real_path, &error)),
    };
    Ok(Some(Fingerprint {
        inode: inode(&metadata),
        size: metadata.len(),
        modified: metadata.modified().ok(),
        real_path,
    }))
}

#[cfg(unix)]
fn inode(metadata: &fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::ino(metadata)
}

#[cfg(not(unix))]
fn inode(_metadata: &fs::Metadata) -> u64 {
    0
}

/// The file's bytes and the findings about the file itself.
type Contents = (Vec<u8>, Vec<ConfigWarning>);

/// The file's contents, or `None` when there is no file.
fn read(path: &Path) -> Result<Option<Contents>, ConfigFileError> {
    let real_path = match fs::canonicalize(path) {
        Ok(real) => real,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(io_error("realpath", path, &error)),
    };
    let metadata =
        fs::metadata(&real_path).map_err(|error| io_error("stat", &real_path, &error))?;
    if !metadata.is_file() {
        return Err(ConfigFileError::NotRegularFile {
            path: display(&real_path),
        });
    }
    let limit = config::MAX_FILE_BYTES as u64;
    if metadata.len() > limit {
        return Err(too_large(path, metadata.len()));
    }
    let bytes = match files::read_regular(&real_path, limit) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return Err(too_large(path, limit + 1));
        }
        Err(error) => return Err(io_error("read", &real_path, &error)),
    };
    Ok(Some((bytes, file_warnings(&metadata, false))))
}

fn too_large(path: &Path, byte_count: u64) -> ConfigFileError {
    ConfigFileError::UnreadableFile {
        path: display(path),
        problem: ConfigProblem {
            kind: ConfigProblemKind::FileTooLarge {
                byte_count,
                limit: config::MAX_FILE_BYTES as u64,
            },
            location: None,
        },
    }
}

/// Hard links and permissions other than 0600. Writing fixes the permissions, so then they are
/// reported as corrected.
#[cfg(unix)]
fn file_warnings(metadata: &fs::Metadata, writing: bool) -> Vec<ConfigWarning> {
    use std::os::unix::fs::MetadataExt;
    let mut warnings = Vec::new();
    if metadata.nlink() > 1 {
        let count = u32::try_from(metadata.nlink()).unwrap_or(u32::MAX);
        warnings.push(ConfigWarning {
            kind: ConfigWarningKind::MultipleHardLinks { count },
            location: None,
        });
    }
    let mode = metadata.mode() & 0o7777;
    if mode != 0o600 {
        let kind = if writing {
            ConfigWarningKind::PermissionsCorrected {
                previous_mode: mode,
            }
        } else {
            ConfigWarningKind::InsecurePermissions { mode }
        };
        warnings.push(ConfigWarning {
            kind,
            location: None,
        });
    }
    warnings
}

#[cfg(not(unix))]
fn file_warnings(_metadata: &fs::Metadata, _writing: bool) -> Vec<ConfigWarning> {
    Vec::new()
}

/// Replaces the file behind `path` with `bytes`, but only while it is still the file `expected`
/// describes (`None`: still no file). Returns what was found or fixed about the file.
fn write(
    path: &Path,
    bytes: &[u8],
    expected: Option<&Fingerprint>,
) -> Result<Vec<ConfigWarning>, ConfigFileError> {
    let target = resolve_target(path)?;
    let mut warnings = Vec::new();
    if let Ok(metadata) = fs::metadata(&target) {
        if !metadata.is_file() {
            return Err(ConfigFileError::NotRegularFile {
                path: display(&target),
            });
        }
        warnings = file_warnings(&metadata, true);
    }
    let (temporary, mut file) =
        files::create_temporary(&target).map_err(|error| io_error("mkstemp", &target, &error))?;
    let written = (|| {
        use std::io::Write;
        file.write_all(bytes)
            .map_err(|error| io_error("write", &temporary, &error))?;
        file.sync_all()
            .map_err(|error| io_error("fsync", &temporary, &error))?;
        drop(file);
        if fingerprint(path)?.as_ref() != expected {
            return Err(ConfigFileError::KeptChanging {
                path: display(path),
            });
        }
        fs::rename(&temporary, &target).map_err(|error| io_error("rename", &target, &error))
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written?;
    if let Some(directory) = target.parent()
        && let Ok(handle) = fs::File::open(directory)
    {
        let _ = handle.sync_all();
    }
    Ok(warnings)
}

/// The path the rename replaces: a symlink's final target when the file exists, otherwise the
/// resolved directory and the name, creating the directory if needed.
fn resolve_target(path: &Path) -> Result<PathBuf, ConfigFileError> {
    match fs::symlink_metadata(path) {
        Ok(link) => match fs::canonicalize(path) {
            Ok(real) => Ok(real),
            Err(error)
                if link.file_type().is_symlink() && error.kind() == io::ErrorKind::NotFound =>
            {
                Err(ConfigFileError::DanglingSymlink {
                    path: display(path),
                })
            }
            Err(error) => Err(io_error("realpath", path, &error)),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let directory = path.parent().unwrap_or_else(|| Path::new("."));
            files::make_directory(directory)
                .map_err(|error| io_error("mkdir", directory, &error))?;
            let real = fs::canonicalize(directory)
                .map_err(|error| io_error("realpath", directory, &error))?;
            Ok(real.join(path.file_name().unwrap_or_default()))
        }
        Err(error) => Err(io_error("lstat", path, &error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        std::env::temp_dir().join(format!("fvm-runtime-config-{name}"))
    }

    #[test]
    fn a_missing_file_gives_the_defaults_and_an_update_creates_it_private() {
        let directory = scratch();
        let path = directory.join("FortiVPNManager").join("config.yaml");
        let mut file = ConfigFile::new(path.clone());
        assert!(file.poll() || !file.exists());
        assert!(!file.exists());
        assert_eq!(file.settings(), &Settings::default());
        assert_eq!(
            file.update(|settings| settings.vpn.host = "vpn.example.com".into()),
            Ok(true)
        );
        assert_eq!(file.settings().vpn.host, "vpn.example.com");
        assert!(!file.poll(), "the app's own write is not a change");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            file.update(|settings| settings.vpn.host = "vpn.example.com".into()),
            Ok(false)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_broken_edit_keeps_the_last_good_settings_and_is_never_written_over() {
        let directory = scratch();
        let path = directory.join("config.yaml");
        let mut file = ConfigFile::new(path.clone());
        file.update(|settings| settings.vpn.port = 8443).unwrap();
        fs::write(&path, "vpn:\n  port: 0443\n").unwrap();
        assert!(file.poll());
        assert_eq!(file.settings().vpn.port, 8443);
        assert!(matches!(
            file.load_error(),
            Some(ConfigFileError::UnreadableFile { .. })
        ));
        assert!(matches!(
            file.update(|settings| settings.vpn.port = 10443),
            Err(ConfigFileError::UnreadableFile { .. })
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "vpn:\n  port: 0443\n");
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_stays_a_symlink() {
        let directory = scratch();
        files::make_directory(&directory).unwrap();
        let real = directory.join("dotfiles-config.yaml");
        fs::write(&real, "vpn:\n  port: 8443\n").unwrap();
        let link = directory.join("config.yaml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut file = ConfigFile::new(link.clone());
        file.poll();
        file.update(|settings| settings.vpn.port = 10443).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(fs::read_to_string(&real).unwrap().contains("port: 10443"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn loose_permissions_are_reported_then_corrected() {
        use std::os::unix::fs::PermissionsExt;
        let directory = scratch();
        files::make_directory(&directory).unwrap();
        let path = directory.join("config.yaml");
        fs::write(&path, "vpn:\n  port: 8443\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut file = ConfigFile::new(path.clone());
        file.poll();
        assert_eq!(
            file.warnings()[0].kind,
            ConfigWarningKind::InsecurePermissions { mode: 0o644 }
        );
        file.update(|settings| settings.vpn.port = 10443).unwrap();
        assert_eq!(
            file.warnings()[0].kind,
            ConfigWarningKind::PermissionsCorrected {
                previous_mode: 0o644
            }
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
