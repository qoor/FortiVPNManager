//! The log file: the VPN client's output and the explanatory lines, unmasked, appended and rotated
//! by size. Its line format is also the Log pane's Copy All text.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fvm_contract::app_internal::{LogLine, LogSource};
use jiff::tz::TimeZone;

use crate::files;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// The current file and the rotated `.1` to `.4`.
const KEPT_FILES: u32 = 5;

/// `2026-11-09T09:00:38.543+09:00 [out] text`, in the local time zone with milliseconds.
pub(crate) fn line(line: &LogLine, time_zone: &TimeZone) -> String {
    let since_1970 = line.time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let timestamp = jiff::Timestamp::from_millisecond(
        i64::try_from(since_1970.as_millis()).expect("a time within the calendar"),
    )
    .expect("a time within the calendar");
    let zoned = timestamp.to_zoned(time_zone.clone());
    let tag = match line.source {
        LogSource::Stdout => "out",
        LogSource::Stderr => "err",
        LogSource::Note => "▸",
    };
    format!(
        "{} [{tag}] {}",
        zoned.strftime("%Y-%m-%dT%H:%M:%S%.3f%:z"),
        line.text
    )
}

pub(crate) struct LogFile {
    path: PathBuf,
    file: Option<File>,
    size: u64,
    failure_reported: bool,
}

impl LogFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            file: None,
            size: 0,
            failure_reported: false,
        }
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Appends the lines; the first failure in a row is returned to be logged, the rest are quiet.
    pub(crate) fn append(&mut self, lines: &[LogLine], time_zone: &TimeZone) -> Option<String> {
        if lines.is_empty() {
            return None;
        }
        let text: String = lines
            .iter()
            .map(|entry| line(entry, time_zone) + "\n")
            .collect();
        match self.write(text.as_bytes()) {
            Ok(()) => {
                self.failure_reported = false;
                None
            }
            Err(error) => {
                self.file = None;
                (!std::mem::replace(&mut self.failure_reported, true))
                    .then(|| format!("log file write failed: {}: {error}", self.path.display()))
            }
        }
    }

    /// Makes the written lines durable, before the app quits.
    pub(crate) fn flush(&mut self) {
        if let Some(file) = &self.file {
            let _ = file.sync_data();
        }
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.file.is_none() {
            self.open()?;
        }
        if self.size > 0 && self.size + bytes.len() as u64 > MAX_FILE_BYTES {
            self.rotate()?;
        }
        self.file.as_mut().expect("opened above").write_all(bytes)?;
        self.size += bytes.len() as u64;
        Ok(())
    }

    fn open(&mut self) -> io::Result<()> {
        if let Some(directory) = self.path.parent() {
            files::make_directory(directory)?;
        }
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // A symlink planted at the log path is not written through.
            options
                .mode(0o600)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
        }
        let file = options.open(&self.path)?;
        files::set_private(&file)?;
        self.size = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        let rotated = |index: u32| PathBuf::from(format!("{}.{index}", self.path.display()));
        let _ = fs::remove_file(rotated(KEPT_FILES - 1));
        for index in (1..KEPT_FILES - 1).rev() {
            let _ = fs::rename(rotated(index), rotated(index + 1));
        }
        match fs::rename(&self.path, rotated(1)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.open()
    }
}

/// The current time, for an explanatory line written now.
pub(crate) fn note(text: impl Into<String>) -> LogLine {
    LogLine {
        time: SystemTime::now(),
        source: LogSource::Note,
        text: text.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fixture {
        time_zone: String,
        message: Message,
        copy_all: Vec<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Message {
        log_lines: Lines,
    }

    #[derive(Deserialize)]
    struct Lines {
        lines: Vec<FixtureLine>,
    }

    #[derive(Deserialize)]
    struct FixtureLine {
        time: u64,
        source: String,
        text: String,
    }

    /// The fixture the Swift UI's Copy All is tested against.
    #[test]
    fn lines_match_the_shared_copy_all_fixture() {
        let fixture: Fixture = serde_json::from_str(include_str!(
            "../../contract/tests/fixtures/app-internal/log_copy_all.json"
        ))
        .unwrap();
        let time_zone = TimeZone::get(&fixture.time_zone).unwrap();
        let written: Vec<String> = fixture
            .message
            .log_lines
            .lines
            .iter()
            .map(|entry| {
                let source = match entry.source.as_str() {
                    "stdout" => LogSource::Stdout,
                    "stderr" => LogSource::Stderr,
                    _ => LogSource::Note,
                };
                let entry = LogLine {
                    time: UNIX_EPOCH + Duration::from_millis(entry.time),
                    source,
                    text: entry.text.clone(),
                };
                line(&entry, &time_zone)
            })
            .collect();
        assert_eq!(written, fixture.copy_all);
    }

    #[test]
    fn a_full_file_rotates_and_keeps_four_old_ones() {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let directory = std::env::temp_dir().join(format!("fvm-runtime-log-{name}"));
        let path = directory.join("openfortivpn.log");
        let mut log = LogFile::new(path.clone());
        let big = LogLine {
            time: UNIX_EPOCH,
            source: LogSource::Stdout,
            text: "x".repeat(1024 * 1024),
        };
        for _ in 0..14 {
            assert_eq!(log.append(std::slice::from_ref(&big), &TimeZone::UTC), None);
        }
        let mut names: Vec<String> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "openfortivpn.log",
                "openfortivpn.log.1",
                "openfortivpn.log.2",
                "openfortivpn.log.3",
                "openfortivpn.log.4"
            ]
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
