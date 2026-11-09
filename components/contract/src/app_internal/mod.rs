//! Messages between the Swift parts of the app, the UI and the OS integration, and the runtime
//! (Rust) in the app process, over the stream of `crate::stream`. The Swift side is
//! `components/contract/macos/app/`, with synthesized `Codable`.
//!
//! - The UI sends user actions as commands and draws the domain sections and log lines it gets.
//!   Sections carry values and kinds, never sentences: the UI builds and translates every sentence,
//!   and the values it puts in them, such as time limits, come with the kinds. Descriptions from
//!   outside (Google, the IMAP server, the Helper, macOS) are carried as they came; so are the
//!   English details of the runtime's own network and file failures, which the UI shows untranslated
//!   (DESIGN §7.2, approved change 4). Button states are decided here, not in the UI.
//! - The OS integration answers port requests, carries out port actions and reports system events.
//!   It reports facts only; combining them, such as into "away", is the runtime's job. Helper
//!   payloads cross it as JSON text it never reads.
//!
//! The JSON follows Swift's enum coding: a case is an object keyed by its name, data without a label
//! uses the key `_0`, a case without data is `{}` unless the whole enum has no data (then it is a
//! string), and absent optional values are left out. Times are milliseconds since 1970, whole when
//! Rust writes them; Swift may write a fraction. Durations shown to the user are whole seconds.
//! Each type derives only the direction it travels.

pub mod command;
pub mod event;
pub mod port;
pub mod problem;
pub mod section;

use std::fmt;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use self::command::Command;
use self::event::SystemEvent;
use self::port::{PortAction, PortRequest, PortResponse};
use self::section::Section;

/// Sent by Swift.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToRuntime {
    /// Always the first message.
    Launched { facts: AppFacts },
    Command {
        #[serde(rename = "_0")]
        command: Command,
    },
    /// Answers the `FromRuntime::PortRequest` with the same `id`.
    PortResponse { id: u64, response: PortResponse },
    Event {
        #[serde(rename = "_0")]
        event: SystemEvent,
    },
}

/// Sent by Rust.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FromRuntime {
    /// One domain section, whole. Every section is sent once first, then only changed ones.
    Section {
        #[serde(rename = "_0")]
        section: Section,
    },
    /// Log lines not sent before, oldest first.
    LogLines { lines: Vec<LogLine> },
    /// Answered by `ToRuntime::PortResponse` with the same `id`.
    PortRequest { id: u64, request: PortRequest },
    /// Carried out without an answer.
    PortAction {
        #[serde(rename = "_0")]
        action: PortAction,
    },
    /// Show the settings window. Sent at most once per launch, after the sections it rests on, and
    /// only when the first config section carries an import report (the automatic first-launch
    /// import) or the first readiness evaluation finds setup required. Setup that becomes required
    /// later in the run opens nothing. The UI only picks the tab: Account when the config section
    /// has an import report, otherwise the tab of the first setup item in the connection status.
    OpenSettings {},
}

/// What the app knows about itself at launch.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppFacts {
    /// The build number the Helper must also report.
    pub app_build: String,
    /// This copy is the installed one, the only copy allowed to register the Helper and the login
    /// item.
    pub installed_copy: bool,
    /// The OAuth client JSON built into the app, if this build has one. Its secret is not
    /// confidential (PKCE protects the sign-in), but it stays out of debug output all the same.
    pub built_in_oauth_client: Option<SecretText>,
    /// The system time zone's identifier, such as `Asia/Seoul`.
    pub time_zone: String,
    /// The pages the Google sign-in listener shows in the browser, in the UI's language: the UI
    /// makes every sentence the user sees.
    pub sign_in_pages: SignInPages,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInPages {
    pub completed: String,
    /// Google came back without an authorization, such as after the user declined.
    pub not_completed: String,
    /// The answer does not belong to the running sign-in.
    pub invalid: String,
}

/// One line for the Log tab. The UI keeps the lines it shows and makes "Copy All" from them, in the
/// log file's format.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct LogLine {
    #[serde(serialize_with = "millis::time")]
    pub time: SystemTime,
    pub source: LogSource,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogSource {
    /// The VPN client's standard output.
    Stdout,
    /// The VPN client's standard error, mostly ordinary messages of its helper processes.
    Stderr,
    /// An explanation line from the app or the Helper.
    Note,
}

/// How OTP emails are read (`otp.method`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OtpMethod {
    /// IMAP with an app password.
    Imap,
    /// The Gmail API with a connected Google account.
    GmailApi,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

/// A local wall-clock time.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ButtonState {
    Hidden,
    Enabled,
    Disabled,
}

/// A setting in minutes with the range the settings file accepts, for the field and its stepper.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub struct MinutesSetting {
    pub value: u32,
    pub minimum: u32,
    pub maximum: u32,
}

/// A password, an OTP, a token or a client secret. Debug output redacts it: it can reach panic
/// messages and so crash reports.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretText(pub String);

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

mod millis {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde::{Deserialize, Deserializer, Serializer, de, ser};

    pub fn time<S: Serializer>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
        let since_1970 = time
            .duration_since(UNIX_EPOCH)
            .map_err(ser::Error::custom)?;
        let millis = u64::try_from(since_1970.as_millis()).map_err(ser::Error::custom)?;
        serializer.serialize_u64(millis)
    }

    pub fn read_time<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
        let millis = f64::deserialize(deserializer)?;
        let since_1970 = Duration::try_from_secs_f64(millis / 1000.0).map_err(de::Error::custom)?;
        Ok(UNIX_EPOCH + since_1970)
    }
}
