use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, SystemTime};

pub use fvm_contract::app_helper::Version;
use fvm_contract::app_helper::installer_version;
use fvm_contract::app_internal::section::RemainingTime;
use serde::Deserialize;

const RELEASES_URL: &str = "https://github.com/qoor/FortiVPNManager/releases";
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/qoor/FortiVPNManager/releases/latest";
pub const CHECKSUMS_FILE: &str = "SHA256SUMS.txt";
pub const CHECKSUMS_SIGNATURE_FILE: &str = "SHA256SUMS.txt.sig";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const PAGE_CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub const DOWNLOADED_PERCENT: u8 = 60;
pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);
const RATE_WINDOW: Duration = Duration::from_secs(5);
const ESTIMATE_AFTER: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Platform {
    MacOs,
    Windows,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Release {
    pub version: Version,
    pub body: String,
    pub assets: Vec<Asset>,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InvalidRelease;

impl Release {
    pub fn from_json(bytes: &[u8]) -> Result<Self, InvalidRelease> {
        #[derive(Deserialize)]
        struct Wire {
            tag_name: String,
            body: Option<String>,
            assets: Vec<Asset>,
        }

        let wire: Wire = serde_json::from_slice(bytes).map_err(|_| InvalidRelease)?;
        Ok(Self {
            version: Version::from_tag(&wire.tag_name).ok_or(InvalidRelease)?,
            body: wire.body.unwrap_or_default(),
            assets: wire.assets,
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Check {
    UpToDate,
    Available(Update),
    Failed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Update {
    pub version: Version,
    pub installer_file_name: String,
    pub installer_size: u64,
    pub release_page_url: String,
    pub installer_url: String,
    pub checksums_url: String,
    pub checksums_signature_url: String,
    pub release_note_titles: BTreeMap<String, Vec<String>>,
}

pub fn check_due(last_check: SystemTime, now: SystemTime) -> bool {
    now < last_check || now >= next_check_at(last_check)
}

pub fn next_check_at(last_check: SystemTime) -> SystemTime {
    last_check + CHECK_INTERVAL
}

pub fn page_check_due(last_check: SystemTime, now: SystemTime) -> bool {
    now < last_check || now >= last_check + PAGE_CHECK_INTERVAL
}

pub fn check(current: Version, release: &Release, platform: Platform) -> Check {
    let version = release.version;
    if version <= current {
        return Check::UpToDate;
    }
    let extension = match platform {
        Platform::MacOs => "pkg",
        Platform::Windows => "msi",
    };
    let Some(installer) = release
        .assets
        .iter()
        .find(|asset| installer_version(&asset.name, extension) == Some(version))
    else {
        return Check::Failed;
    };
    let files = [CHECKSUMS_FILE, CHECKSUMS_SIGNATURE_FILE];
    if !files
        .iter()
        .all(|file| release.assets.iter().any(|asset| asset.name == *file))
    {
        return Check::Failed;
    }
    let installer_file_name = installer.name.clone();
    let download_url = |file: &str| format!("{RELEASES_URL}/download/v{version}/{file}");
    Check::Available(Update {
        version,
        release_page_url: format!("{RELEASES_URL}/tag/v{version}"),
        installer_url: download_url(&installer_file_name),
        checksums_url: download_url(CHECKSUMS_FILE),
        checksums_signature_url: download_url(CHECKSUMS_SIGNATURE_FILE),
        release_note_titles: release_note_titles(&release.body),
        installer_file_name,
        installer_size: installer.size,
    })
}

fn release_note_titles(body: &str) -> BTreeMap<String, Vec<String>> {
    let mut titles: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut language = None;
    for line in body.lines() {
        if let Some(code) = language_marker(line) {
            language = Some(code);
        } else if let (Some(code), Some(title)) = (
            language,
            line.strip_prefix("### ")
                .map(str::trim)
                .filter(|title| !title.is_empty()),
        ) {
            titles
                .entry(code.to_owned())
                .or_default()
                .push(title.to_owned());
        }
    }
    titles
}

fn language_marker(line: &str) -> Option<&str> {
    let code = line
        .trim()
        .strip_prefix("<!-- lang:")?
        .strip_suffix(" -->")?;
    (!code.is_empty() && code.bytes().all(|byte| byte.is_ascii_lowercase())).then_some(code)
}

pub fn failed_install_version(target: Version, running: Version) -> Option<Version> {
    (target > running).then_some(target)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DownloadProgress {
    total: u64,
    samples: VecDeque<(Duration, u64)>,
}

impl DownloadProgress {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            samples: VecDeque::from([(Duration::ZERO, 0)]),
        }
    }

    pub fn record(&mut self, elapsed: Duration, received: u64) {
        self.samples.push_back((elapsed, received));
        while self
            .samples
            .get(1)
            .is_some_and(|&(at, _)| at + RATE_WINDOW <= elapsed)
        {
            self.samples.pop_front();
        }
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn received(&self) -> u64 {
        self.newest().1
    }

    pub fn percent(&self) -> u8 {
        let done = u128::from(self.received().min(self.total)) * u128::from(DOWNLOADED_PERCENT);
        done.checked_div(u128::from(self.total))
            .map_or(0, |percent| {
                u8::try_from(percent).expect("at most DOWNLOADED_PERCENT")
            })
    }

    pub fn remaining(&self) -> RemainingTime {
        let (start, start_bytes) = self.samples[0];
        let (now, now_bytes) = self.newest();
        let bytes = now_bytes - start_bytes;
        if now < ESTIMATE_AFTER || bytes == 0 {
            return RemainingTime::Calculating {};
        }
        let left = u128::from(self.total.saturating_sub(now_bytes));
        let nanos = (left * (now - start).as_nanos()).div_ceil(u128::from(bytes));
        rounded(nanos)
    }

    fn newest(&self) -> (Duration, u64) {
        *self.samples.back().expect("the start is always a sample")
    }
}

fn rounded(nanos: u128) -> RemainingTime {
    const SECOND: u128 = 1_000_000_000;
    const MINUTE: u128 = 60 * SECOND;
    const STEP: u128 = 5 * SECOND;
    let seconds = (nanos.div_ceil(STEP) * 5).max(5);
    if seconds < 60 {
        RemainingTime::Seconds {
            seconds: u64::try_from(seconds).expect("under a minute"),
        }
    } else {
        RemainingTime::Minutes {
            minutes: u64::try_from(nanos.div_ceil(MINUTE)).unwrap_or(u64::MAX),
        }
    }
}
