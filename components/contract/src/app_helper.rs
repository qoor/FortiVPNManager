//! What the app and the Helper exchange. Every message uses Swift's standard Codable coding
//! (`XPCCoding`), since the OS integration Helper (Swift) answers the bypass-route payloads itself,
//! reading and writing networksetup: an enum value with data is an object keyed by its case name,
//! data without a label uses the key `_0`, a case without data is `{}`, dates are seconds since
//! 1970, and absent optional values are left out. Each type derives only the directions its two
//! users need: the Helper receives the run request and sends everything else, and the app's runtime
//! does the opposite.

use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};

use crate::backend::{ClientProblem, UntestedVersion};

/// One connection attempt, as the app's UUID text. Kept in uppercase, the form the app writes, so
/// equal attempts compare equal however the text was cased.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AttemptId(String);

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidAttemptId;

impl fmt::Display for InvalidAttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a UUID")
    }
}

impl std::error::Error for InvalidAttemptId {}

impl FromStr for AttemptId {
    type Err = InvalidAttemptId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let is_uuid = text.len() == 36
            && text.bytes().enumerate().all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => byte == b'-',
                _ => byte.is_ascii_hexdigit(),
            });
        if is_uuid {
            Ok(Self(text.to_ascii_uppercase()))
        } else {
            Err(InvalidAttemptId)
        }
    }
}

impl TryFrom<String> for AttemptId {
    type Error = InvalidAttemptId;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<AttemptId> for String {
    fn from(id: AttemptId) -> Self {
        id.0
    }
}

impl fmt::Display for AttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Everything the Helper needs for one run. The Helper checks every value again before use.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    #[serde(rename = "attemptID")]
    pub attempt_id: AttemptId,
    pub host: String,
    /// Not narrowed here: an out-of-range port is a validation error, not an unreadable request.
    pub port: i64,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
    /// Gateway certificate values to trust.
    pub trusted_certs: Vec<String>,
    pub password: String,
}

/// Redacts the password: Debug output can reach panic messages and so crash reports.
impl fmt::Debug for RunRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunRequest")
            .field("attempt_id", &self.attempt_id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("realm", &self.realm)
            .field("trusted_certs", &self.trusted_certs)
            .field("password", &format_args!("<redacted>"))
            .finish()
    }
}

/// Progress of one run, in the order it normally happens. Sent as its position, starting at 0.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(into = "u8", try_from = "u8")]
#[repr(u8)]
pub enum Milestone {
    Spawned,
    ConnectedToGateway,
    OtpPrompted,
    OtpSubmitted,
    Authenticated,
    TunnelUp,
}

impl From<Milestone> for u8 {
    fn from(milestone: Milestone) -> Self {
        milestone as u8
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct UnknownMilestone(pub u8);

impl fmt::Display for UnknownMilestone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no milestone at position {}", self.0)
    }
}

impl TryFrom<u8> for Milestone {
    type Error = UnknownMilestone;

    fn try_from(position: u8) -> Result<Self, Self::Error> {
        use Milestone::*;
        [
            Spawned,
            ConnectedToGateway,
            OtpPrompted,
            OtpSubmitted,
            Authenticated,
            TunnelUp,
        ]
        .get(usize::from(position))
        .copied()
        .ok_or(UnknownMilestone(position))
    }
}

/// Why the Helper stopped, or was asked to stop, a run.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    UserDisconnect,
    ProactiveReconnect,
    /// An app-initiated restart other than the proactive one.
    Reconnect,
    OtpTimeout,
    /// The OTP session failed while the client waited for the OTP.
    OtpUnavailable,
    AttemptTimeout,
    Sleep,
    AppQuit,
    /// The app connection that owns the run went away.
    #[serde(rename = "clientInvalidated")]
    AppDisconnected,
    /// A step of the run took too long.
    Watchdog,
    HelperShutdown,
}

/// Parses the raw value the app sends with a disconnect request.
impl FromStr for StopReason {
    type Err = serde::de::value::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::deserialize(text.into_deserializer())
    }
}

/// Who ended the run.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum StopInitiator {
    /// The client exited on its own (server cut, failed login, crash, ...).
    #[serde(rename = "none")]
    Client {},
    #[serde(rename = "helper")]
    Helper {
        #[serde(rename = "_0")]
        reason: StopReason,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessExitStatus {
    Exited {
        code: i32,
    },
    Signaled {
        signal: i32,
    },
    SpawnFailed {
        errno: i32,
    },
    /// The status could not be collected.
    Unknown {},
}

/// What the Helper recorded about a finished run; the app classifies the end from these facts.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitFacts {
    /// `None` when the client could not be started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub furthest_milestone: Option<Milestone>,
    pub stop_initiator: StopInitiator,
    /// Certificate values the client printed when it rejected the gateway, without repeats.
    pub cert_rejected_digests: Vec<String>,
    /// The milestone reached when the first certificate rejection was printed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_rejected_at: Option<Milestone>,
    /// The gateway refused tunnel mode after authentication.
    pub tunnel_mode_rejected: bool,
    /// From tunnel up to exit, if the tunnel came up.
    #[serde(
        default,
        serialize_with = "seconds::optional_duration",
        deserialize_with = "seconds::read_optional_duration",
        skip_serializing_if = "Option::is_none"
    )]
    pub up_duration: Option<Duration>,
    pub exit_status: ProcessExitStatus,
}

/// Sent by the Helper to the app that owns the run.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct HelperEvent {
    #[serde(rename = "attemptID")]
    pub attempt_id: AttemptId,
    pub kind: HelperEventKind,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HelperEventKind {
    Milestone {
        #[serde(rename = "_0")]
        milestone: Milestone,
        #[serde(rename = "tunnelIP", skip_serializing_if = "Option::is_none")]
        tunnel_ip: Option<Ipv4Addr>,
    },
    /// The run's last event.
    Exited {
        #[serde(rename = "_0")]
        facts: ExitFacts,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputSource {
    Stdout,
    Stderr,
    /// Lines the Helper writes itself.
    Helper,
}

/// One line for the Log tab. The app receives them in batches (a JSON array).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct OutputLine {
    #[serde(
        serialize_with = "seconds::time",
        deserialize_with = "seconds::read_time"
    )]
    pub date: SystemTime,
    pub source: OutputSource,
    pub text: String,
}

/// Answer to the app's status request.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct HelperStatus {
    #[serde(rename = "childAlive")]
    pub client_running: bool,
    #[serde(rename = "openfortivpnPath", skip_serializing_if = "Option::is_none")]
    pub client_path: Option<String>,
    #[serde(
        rename = "openfortivpnVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_version: Option<String>,
    #[serde(rename = "clientProblem", skip_serializing_if = "Option::is_none")]
    pub client_problem: Option<ClientProblem>,
    /// The backend adapter's judgment of `client_version`.
    #[serde(rename = "untestedVersion", skip_serializing_if = "Option::is_none")]
    pub untested_version: Option<UntestedVersion>,
}

/// Error answer to an app request; `message` is English and reaches the app as it is.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct HelperError {
    pub code: HelperErrorCode,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HelperErrorCode {
    Busy,
    InvalidRequest,
    UnknownAttempt,
    WrongPhase,
    #[serde(rename = "binaryUnavailable")]
    ClientUnavailable,
    NotAuthorized,
    InternalError,
}

/// One additional route of a network service, as `networksetup -getadditionalroutes` prints it.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalRoute {
    pub destination: String,
    pub subnet_mask: String,
    pub gateway: String,
}

/// The primary network service, the one macOS sends default traffic through, and its additional
/// routes.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BypassRoutesSnapshot {
    /// Stable across renames.
    #[serde(rename = "serviceID")]
    pub service_id: String,
    /// The name networksetup uses, such as `Ethernet` or `Wi-Fi`.
    pub service_name: String,
    /// The service's IPv4 router; exception routes go through it.
    pub router: String,
    /// This Mac's IPv4 address on the service and its subnet mask.
    pub address: String,
    pub subnet_mask: String,
    pub routes: Vec<AdditionalRoute>,
}

/// The complete additional route list for the service in `expected`. The Helper writes it only
/// while the primary service, its router and its current routes are still exactly `expected`, and
/// only with routes through that router or routes the service already has.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct BypassRoutesRequest {
    /// What the app read and planned from.
    pub expected: BypassRoutesSnapshot,
    pub routes: Vec<AdditionalRoute>,
}

/// Answer to reading or writing the bypass routes.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BypassRoutesReply {
    Success {
        #[serde(rename = "_0")]
        snapshot: BypassRoutesSnapshot,
    },
    Failure {
        #[serde(rename = "_0")]
        error: HelperError,
    },
}

/// Seconds since 1970 or durations in seconds, as numbers.
mod seconds {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde::{Deserialize, Deserializer, Serializer, de, ser};

    fn write<S: Serializer>(duration: Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(duration.as_secs_f64())
    }

    pub fn time<S: Serializer>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
        let since_1970 = time
            .duration_since(UNIX_EPOCH)
            .map_err(ser::Error::custom)?;
        write(since_1970, serializer)
    }

    pub fn optional_duration<S: Serializer>(
        duration: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match duration {
            Some(duration) => write(*duration, serializer),
            None => serializer.serialize_none(),
        }
    }

    fn read<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        Duration::try_from_secs_f64(f64::deserialize(deserializer)?).map_err(de::Error::custom)
    }

    pub fn read_time<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
        Ok(UNIX_EPOCH + read(deserializer)?)
    }

    pub fn read_optional_duration<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        read(deserializer).map(Some)
    }
}
