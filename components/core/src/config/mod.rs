//! config.yaml: strict reading and validation, and the settings the app writes back.
//!
//! The file text is the boundary: `read` checks everything in it, and the settings it returns hold
//! only valid values. `update` decides what a write puts in the file. The runtime does the file
//! work: reading the latest bytes, the atomic write that keeps a symbolic link, and trying again
//! when the file changes in between.
//!
//! The VPN client's own rules come in as `ClientRules`, since the core does not know the client.

mod reader;
mod template;
mod text;
mod tree;

use std::ops::RangeInclusive;

use fvm_contract::app_internal::problem::{ConfigProblem, ConfigWarning};
use fvm_contract::app_internal::{OtpMethod, TimeOfDay, Weekday};

pub use crate::bypass_rules::Ipv4Network;

/// Largest file the app reads or writes.
pub const MAX_FILE_BYTES: usize = 64 * 1024;
pub const PORTS: RangeInclusive<i64> = 1..=65_535;
pub const SESSION_MAX_MINUTES: RangeInclusive<i64> = 10..=1440;
/// Also less than the session's maximum minutes.
pub const MARGIN_MINUTES: RangeInclusive<i64> = 1..=60;
pub const MIN_SLEEP_MINUTES: RangeInclusive<i64> = 1..=1440;
pub const AWAY_MINUTES: RangeInclusive<i64> = 1..=1440;
/// Certificate values kept in `vpn.trusted_certs`, as the file's comments promise.
pub const MAX_TRUSTED_CERTS: usize = 4;
/// Entries in each of `routes.networks` and `routes.bypass_vpn`.
pub const MAX_ROUTE_ENTRIES: usize = 100;

/// The `schema_version` this build reads and writes.
const SCHEMA_VERSION: i64 = 1;

/// Spellings in the file, in the order the settings keep them.
const OTP_METHODS: [(&str, OtpMethod); 2] = [
    ("imap", OtpMethod::Imap),
    ("gmail_api", OtpMethod::GmailApi),
];
const WEEKDAYS: [(&str, Weekday); 7] = [
    ("mon", Weekday::Mon),
    ("tue", Weekday::Tue),
    ("wed", Weekday::Wed),
    ("thu", Weekday::Thu),
    ("fri", Weekday::Fri),
    ("sat", Weekday::Sat),
    ("sun", Weekday::Sun),
];

/// What the file needs from the VPN client's backend adapter.
pub struct ClientRules<'a> {
    /// The client's name, as the comments of a written file show it.
    pub name: &'a str,
    /// The canonical form of a gateway certificate value the client accepts, or `None` when the
    /// text is not one.
    pub certificate_value: &'a dyn Fn(&str) -> Option<String>,
}

/// Everything config.yaml holds. Passwords are never part of it. Missing keys and null values
/// read as the defaults.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Settings {
    pub vpn: Vpn,
    pub otp: Otp,
    pub connection: Connection,
    pub routes: Routes,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Vpn {
    /// The gateway's host name or IPv4 address.
    pub host: String,
    pub port: i64,
    pub username: String,
    /// Empty means none.
    pub realm: String,
    /// Gateway certificate values to trust, newest first, in the client's canonical form. Managed
    /// by the app.
    pub trusted_certs: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Otp {
    /// The Gmail address that receives the OTP emails.
    pub gmail_address: String,
    pub method: OtpMethod,
    /// The imported Google OAuth client; empty when none is imported.
    pub google_client_id: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Connection {
    pub auto_connect_on_launch: bool,
    /// How long the server keeps a session.
    pub session_max_minutes: i64,
    pub proactive_reconnect: ProactiveReconnect,
    pub reconnect_after_sleep: ReconnectAfterSleep,
    pub schedule: Schedule,
}

/// Reconnecting `margin_minutes` before the session's maximum time runs out.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProactiveReconnect {
    pub enabled: bool,
    pub margin_minutes: i64,
}

/// Reconnecting on wake after a sleep of at least `min_sleep_minutes`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReconnectAfterSleep {
    pub enabled: bool,
    pub min_sleep_minutes: i64,
}

/// Connecting at `start` on `days`. The end never disconnects; after it, reconnects wait while the
/// user is away and stop once the user has been away for `away_minutes`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Schedule {
    pub enabled: bool,
    /// Monday first, without repeats.
    pub days: Vec<Weekday>,
    /// Earlier than `end`.
    pub start: TimeOfDay,
    pub end: TimeOfDay,
    pub away_minutes: i64,
}

/// The exception rules. Entries of one list never overlap.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Routes {
    /// Where the rules apply.
    pub networks: Vec<Ipv4Network>,
    /// What skips the VPN there.
    pub bypass_vpn: Vec<Ipv4Network>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vpn: Vpn {
                host: String::new(),
                port: 443,
                username: String::new(),
                realm: String::new(),
                trusted_certs: Vec::new(),
            },
            otp: Otp {
                gmail_address: String::new(),
                method: OtpMethod::Imap,
                google_client_id: String::new(),
            },
            connection: Connection {
                auto_connect_on_launch: true,
                session_max_minutes: 180,
                proactive_reconnect: ProactiveReconnect {
                    enabled: true,
                    margin_minutes: 5,
                },
                reconnect_after_sleep: ReconnectAfterSleep {
                    enabled: false,
                    min_sleep_minutes: 30,
                },
                schedule: Schedule {
                    enabled: false,
                    days: vec![
                        Weekday::Mon,
                        Weekday::Tue,
                        Weekday::Wed,
                        Weekday::Thu,
                        Weekday::Fri,
                    ],
                    start: TimeOfDay { hour: 9, minute: 0 },
                    end: TimeOfDay {
                        hour: 18,
                        minute: 0,
                    },
                    away_minutes: 120,
                },
            },
            routes: Routes {
                networks: Vec::new(),
                bypass_vpn: Vec::new(),
            },
        }
    }
}

/// A file that passed, with what was ignored or noticed in it, in file order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loaded {
    pub settings: Settings,
    pub warnings: Vec<ConfigWarning>,
}

/// What a change does to the file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Update {
    /// The settings in the file stay as they are, so nothing is written.
    Unchanged(Settings),
    /// Write `text`, which reads back as `settings`. A file written this way loses the comments
    /// the user added and the keys this version does not know.
    Write { settings: Settings, text: String },
}

/// Why nothing is written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UpdateError {
    /// The file does not pass, or a newer app wrote it.
    File(ConfigProblem),
    /// The changed settings break a rule; the problem has no location.
    Settings(ConfigProblem),
}

/// Reads the file's bytes. An empty file, or one with only comments or a null document, holds
/// the defaults; a missing file is the caller's to treat the same way.
pub fn read(bytes: &[u8], client: &ClientRules<'_>) -> Result<Loaded, ConfigProblem> {
    let text = text::check(bytes)?;
    let tree = tree::build(text)?;
    reader::read(&tree, client)
}

/// Applies `change` to the settings in `current`, the file's latest bytes (`None` when there is no
/// file), and decides what to write. A file that does not pass is never written over, so a newer
/// app's file stays as it is.
pub fn update(
    current: Option<&[u8]>,
    client: &ClientRules<'_>,
    change: impl FnOnce(&mut Settings),
) -> Result<Update, UpdateError> {
    let base = match current {
        Some(bytes) => Some(read(bytes, client).map_err(UpdateError::File)?.settings),
        None => None,
    };
    let mut settings = base.clone().unwrap_or_default();
    change(&mut settings);
    if base.as_ref() == Some(&settings) {
        return Ok(Update::Unchanged(settings));
    }
    match template::write(&settings, client.name) {
        Ok(text) => Ok(Update::Write { settings, text }),
        Err(kind) => Err(UpdateError::Settings(ConfigProblem {
            kind,
            location: None,
        })),
    }
}

/// Without repeats, Monday first.
fn normalized_days(days: &[Weekday]) -> Vec<Weekday> {
    WEEKDAYS
        .iter()
        .map(|&(_, day)| day)
        .filter(|day| days.contains(day))
        .collect()
}

fn minutes(time: TimeOfDay) -> u32 {
    u32::from(time.hour) * 60 + u32::from(time.minute)
}
