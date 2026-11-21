//! What the app and the Helper exchange. Every message uses Swift's standard Codable coding
//! (`XPCCoding`), so the Helper's Swift side can write the few answers it gives itself, such as a
//! `HelperError` for a payload it cannot pass on: an enum value with data is an object keyed by its
//! case name, data without a label uses the key `_0`, a case without data is `{}`, dates are seconds
//! since 1970, and absent optional values are left out. The Helper receives the run request, the
//! exception-route request and the update prepare and install requests and sends everything else,
//! and the app's runtime does the opposite; the Helper entry, in Rust, reads and writes them on
//! every OS.

use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::time::SystemTime;

use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};

use crate::app_internal::problem::AddressProblem;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub u64);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:010}", self.0)
    }
}

/// Everything the Helper needs for one run. The Helper checks every value again before use.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    #[serde(rename = "attemptID")]
    pub attempt_id: AttemptId,
    #[serde(
        rename = "replacingAttemptID",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub replacing: Option<AttemptId>,
    #[serde(rename = "sessionID")]
    pub session_id: SessionId,
    pub host: String,
    /// Not narrowed here: an out-of-range port is a validation error, not an unreadable request.
    pub port: i64,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
    /// Gateway certificate values to trust.
    pub trusted_certs: Vec<String>,
    pub dtls: bool,
    pub password: String,
}

/// Redacts the password: Debug output can reach panic messages and so crash reports.
impl fmt::Debug for RunRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunRequest")
            .field("attempt_id", &self.attempt_id)
            .field("replacing", &self.replacing)
            .field("session_id", &self.session_id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("realm", &self.realm)
            .field("trusted_certs", &self.trusted_certs)
            .field("dtls", &self.dtls)
            .field("password", &format_args!("<redacted>"))
            .finish()
    }
}

/// Progress of one run, in the order it normally happens. Sent as its position, starting at 0.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(into = "u8", try_from = "u8")]
#[repr(u8)]
pub enum Milestone {
    Started,
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
            Started,
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
    StageLimit,
    /// The client asked again for the password or the OTP it was given: the gateway rejected it.
    PromptRepeated,
    NetworkSetupFailed,
    HelperShutdown,
    Replaced,
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
    /// The session ended on its own (server cut, failed login, ...).
    #[serde(rename = "none")]
    Session {},
    #[serde(rename = "helper")]
    Helper {
        #[serde(rename = "_0")]
        reason: StopReason,
    },
}

/// What the Helper recorded about a finished run; the app classifies the end from these facts.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndFacts {
    pub furthest_milestone: Milestone,
    pub stop_initiator: StopInitiator,
    /// Certificate values the client printed when it rejected the gateway, without repeats.
    pub cert_rejected_digests: Vec<String>,
    /// The milestone reached when the first certificate rejection was printed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_rejected_at: Option<Milestone>,
    pub tunnel_setup_failed: bool,
    pub tunnel_address_in_use: bool,
    pub end_code: i32,
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
        facts: EndFacts,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputSource {
    Info,
    Error,
    /// Lines the Helper writes itself.
    Helper,
}

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

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct OutputBatch {
    #[serde(rename = "attemptID")]
    pub attempt_id: AttemptId,
    pub lines: Vec<OutputLine>,
}

/// Answer to the app's status request.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct HelperStatus {
    #[serde(rename = "sessionRunning")]
    pub session_running: bool,
    pub backend: BackendStatus,
    #[serde(
        rename = "routingComplete",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub routing_complete: Option<bool>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
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
    NotAuthorized,
    InternalError,
}

/// One exception route: a destination network, as an address and a subnet mask, through the router
/// `gateway`. A host's mask is `255.255.255.255`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BypassRoute {
    pub destination: String,
    pub subnet_mask: String,
    pub gateway: String,
}

/// Most entries one list may hold; the settings file refuses more.
pub const MAX_ENTRIES: usize = 100;

pub const MIN_PREFIX_LENGTH: u32 = 8;
pub const MAX_PREFIX_LENGTH: u32 = 32;

/// An IPv4 host (a /32) or network as the exception rules use it: no bits set after the prefix,
/// a prefix of 8 to 32, and an address that can be routed. Shown as `10.30.161.16` for a host and
/// `10.30.170.0/24` for a network.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Ipv4Network {
    address: u32,
    prefix_length: u32,
}

impl Ipv4Network {
    /// A route's destination: an address and a subnet mask.
    pub fn from_route(route: &BypassRoute) -> Result<Self, AddressProblem> {
        let (address, prefix_length) = address_and_mask(&route.destination, &route.subnet_mask)?;
        Self::new(address, prefix_length)
    }

    /// The network an interface address belongs to, such as `10.20.161.0/24` for `10.20.161.20`
    /// with `255.255.255.0`: the host bits are cleared instead of refused.
    pub fn enclosing(address: &str, subnet_mask: &str) -> Result<Self, AddressProblem> {
        let (address, prefix_length) = address_and_mask(address, subnet_mask)?;
        Self::new(address & mask(prefix_length), prefix_length)
    }

    /// The single host `address`, if it can be an exception target.
    pub fn host(address: Ipv4Addr) -> Option<Self> {
        Self::new(u32::from(address), MAX_PREFIX_LENGTH).ok()
    }

    fn new(address: u32, prefix_length: u32) -> Result<Self, AddressProblem> {
        if !(MIN_PREFIX_LENGTH..=MAX_PREFIX_LENGTH).contains(&prefix_length) {
            return Err(AddressProblem::PrefixOutOfRange {
                length: prefix_length,
                minimum: MIN_PREFIX_LENGTH,
                maximum: MAX_PREFIX_LENGTH,
            });
        }
        // 0.0.0.0/8, loopback, and multicast, reserved and broadcast from 224.0.0.0 up.
        let first_octet = address >> 24;
        if first_octet == 0 || first_octet == 127 || first_octet >= 224 {
            return Err(AddressProblem::Unroutable {});
        }
        let network = address & mask(prefix_length);
        if network != address {
            let network = Self {
                address: network,
                prefix_length,
            };
            return Err(AddressProblem::HostBitsSet {
                network: network.to_string(),
            });
        }
        Ok(Self {
            address,
            prefix_length,
        })
    }

    pub fn address(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.address)
    }

    pub fn prefix_length(&self) -> u8 {
        u8::try_from(self.prefix_length).expect("prefix lengths are at most 32")
    }

    pub fn contains_address(&self, address: Ipv4Addr) -> bool {
        u32::from(address) & mask(self.prefix_length) == self.address
    }

    /// Whether all of `other` is inside this network.
    pub fn contains(&self, other: &Self) -> bool {
        other.prefix_length >= self.prefix_length
            && other.address & mask(self.prefix_length) == self.address
    }

    /// Whether the two share an address, which means one contains the other.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.contains(other) || other.contains(self)
    }

    /// The exception route that sends this network through `gateway`.
    pub fn route_through(&self, gateway: &str) -> BypassRoute {
        BypassRoute {
            destination: self.address().to_string(),
            subnet_mask: Ipv4Addr::from(mask(self.prefix_length)).to_string(),
            gateway: gateway.to_owned(),
        }
    }
}

/// Reads `a.b.c.d` or `a.b.c.d/n` without surrounding spaces; `a.b.c.d/32` is `a.b.c.d`. The
/// address follows the contract's IPv4 rule (`Ipv4Addr`'s: four decimal octets without leading
/// zeros); the prefix is one or two digits without a leading zero.
impl FromStr for Ipv4Network {
    type Err = AddressProblem;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address = address
            .parse::<Ipv4Addr>()
            .map_err(|_| AddressProblem::Malformed {})?;
        let prefix_length = match prefix {
            None => MAX_PREFIX_LENGTH,
            Some(prefix) => {
                let plain = (1..=2).contains(&prefix.len())
                    && prefix.bytes().all(|byte| byte.is_ascii_digit())
                    && (prefix.len() == 1 || !prefix.starts_with('0'));
                if !plain {
                    return Err(AddressProblem::Malformed {});
                }
                prefix.parse().expect("one or two ASCII digits")
            }
        };
        Self::new(u32::from(address), prefix_length)
    }
}

impl fmt::Display for Ipv4Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.prefix_length == MAX_PREFIX_LENGTH {
            write!(f, "{}", self.address())
        } else {
            write!(f, "{}/{}", self.address(), self.prefix_length)
        }
    }
}

fn mask(prefix_length: u32) -> u32 {
    u32::MAX.checked_shl(32 - prefix_length).unwrap_or(0)
}

fn address_and_mask(address: &str, subnet_mask: &str) -> Result<(u32, u32), AddressProblem> {
    let (Ok(address), Ok(subnet_mask)) =
        (address.parse::<Ipv4Addr>(), subnet_mask.parse::<Ipv4Addr>())
    else {
        return Err(AddressProblem::Malformed {});
    };
    let subnet_mask = u32::from(subnet_mask);
    let prefix_length = subnet_mask.leading_ones();
    if mask(prefix_length) != subnet_mask {
        return Err(AddressProblem::NonContiguousMask {});
    }
    Ok((u32::from(address), prefix_length))
}

/// The primary network: the physical interface the OS sends default traffic through, never a
/// tunnel, with its IPv4 router and this device's IPv4 address on it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BypassRoutesSnapshot {
    /// Stable across renames: the network service on macOS, the interface GUID on Windows.
    #[serde(rename = "serviceID")]
    pub service_id: String,
    /// The name the user knows it by, such as `Ethernet` or `Wi-Fi`.
    pub service_name: String,
    /// Exception routes go through it.
    pub router: String,
    pub address: String,
    pub subnet_mask: String,
}

/// What an app asks of its own exception routes. The Helper keeps them in the routing table, not in
/// the OS's saved settings, and removes them when the app's connection goes away or the app asks.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BypassRoutesRequest {
    /// The app's exception routes become `routes`, all through the router of `expected`. The Helper
    /// changes nothing unless the primary network is still exactly `expected`, and adds only
    /// routes to exception targets through that router.
    Write {
        /// What the app read and planned from.
        expected: BypassRoutesSnapshot,
        routes: Vec<BypassRoute>,
    },
    /// The app quits: its exception routes go, whatever the network.
    Remove {},
}

/// Answer to reading or writing the exception routes.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BypassRoutesReply {
    /// The primary network now, and those of the app's exception routes the routing table has:
    /// after a write, of the routes written; after a read, of the routes the app last wrote.
    Success {
        snapshot: BypassRoutesSnapshot,
        routes: Vec<BypassRoute>,
    },
    /// Answers `BypassRoutesRequest::Remove`.
    Removed {},
    Failure {
        #[serde(rename = "_0")]
        error: HelperError,
    },
}

pub const UPDATE_RECORDS_DIRECTORY_MACOS: &str = "/var/run/com.qoor.FortiVPNManager.Helper.update";
pub const UPDATE_RECORDS_DIRECTORY_WINDOWS: &str = r"C:\Program Files\FortiVPNManager\AutoUpdate";
pub const UPDATE_STARTED_SUFFIX: &str = ".started";
pub const UPDATE_RESULT_SUFFIX: &str = ".result";
pub const CHECKSUMS_FILE: &str = "SHA256SUMS.txt";
pub const CHECKSUMS_SIGNATURE_FILE: &str = "SHA256SUMS.txt.sig";

/// Sent as its text, such as `26.2.0`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let number = |part: &str| {
            let plain = !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'));
            if plain { part.parse().ok() } else { None }
        };
        let parts: Vec<&str> = text.split('.').collect();
        let [major, minor, patch] = parts[..] else {
            return None;
        };
        Some(Self {
            major: number(major)?,
            minor: number(minor)?,
            patch: number(patch)?,
        })
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        Self::parse(tag.strip_prefix('v')?)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidVersion;

impl fmt::Display for InvalidVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a version of three numbers")
    }
}

impl std::error::Error for InvalidVersion {}

impl TryFrom<String> for Version {
    type Error = InvalidVersion;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text).ok_or(InvalidVersion)
    }
}

impl From<Version> for String {
    fn from(version: Version) -> Self {
        version.to_string()
    }
}

pub fn installer_version(file_name: &str, extension: &str) -> Option<Version> {
    let version = file_name
        .strip_prefix("FortiVPNManager-")?
        .strip_suffix(extension)?
        .strip_suffix('.')?;
    Version::parse(version)
}

/// The downloaded update for the Helper to copy into a folder only administrators can change and to
/// check there: the checksum list's signature, the installer's checksum and that the installer's
/// version is newer than the Helper's. Files prepared before are replaced.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePrepareRequest {
    pub installer_path: String,
    pub checksums_path: String,
    pub signature_path: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePrepareReply {
    /// The version of the installer now prepared.
    Prepared { version: Version },
    Refused {
        reason: UpdateRefusal,
        message: String,
    },
    Failure {
        #[serde(rename = "_0")]
        error: HelperError,
    },
}

/// Starts the prepared installer. `install_id` names the update records of this install.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct UpdateInstallRequest {
    #[serde(rename = "installID")]
    pub install_id: u64,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateInstallReply {
    Started {
        #[serde(
            rename = "installerProcessID",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        installer_process_id: Option<u32>,
    },
    Refused {
        reason: UpdateRefusal,
        message: String,
    },
    Failure {
        #[serde(rename = "_0")]
        error: HelperError,
    },
}

/// Answers the question which update the Helper holds prepared.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreparedUpdateReply {
    Prepared {
        version: Version,
    },
    NotPrepared {},
    Failure {
        #[serde(rename = "_0")]
        error: HelperError,
    },
}

/// Answers the request to delete the prepared update.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateDiscardReply {
    Discarded {},
    Failure {
        #[serde(rename = "_0")]
        error: HelperError,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateRefusal {
    SignatureInvalid,
    ChecksumMismatch,
    VersionNotNewer,
    AlreadyInstalling,
    FilesUnreadable,
    NothingPrepared,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    #[serde(rename = "installID")]
    pub install_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// Seconds since 1970, as numbers.
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

    fn read<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        Duration::try_from_secs_f64(f64::deserialize(deserializer)?).map_err(de::Error::custom)
    }

    pub fn read_time<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
        Ok(UNIX_EPOCH + read(deserializer)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(text: &str) -> Ipv4Network {
        text.parse().unwrap()
    }

    fn parse(text: &str) -> Result<Ipv4Network, AddressProblem> {
        text.parse()
    }

    fn out_of_range(length: u32) -> AddressProblem {
        AddressProblem::PrefixOutOfRange {
            length,
            minimum: 8,
            maximum: 32,
        }
    }

    fn route(destination: &str, mask: &str) -> BypassRoute {
        BypassRoute {
            destination: destination.into(),
            subnet_mask: mask.into(),
            gateway: "10.20.161.254".into(),
        }
    }

    fn bypass_network() -> Ipv4Network {
        net("10.30.170.0/24")
    }
    fn office() -> Ipv4Network {
        net("10.20.161.0/24")
    }

    #[test]
    fn hosts_and_networks_in_canonical_spelling() {
        for (raw, canonical, prefix_length) in [
            ("10.30.161.16", "10.30.161.16", 32),
            ("10.30.161.16/32", "10.30.161.16", 32),
            ("10.30.170.0/24", "10.30.170.0/24", 24),
            ("10.0.0.0/8", "10.0.0.0/8", 8),
            ("192.168.50.128/25", "192.168.50.128/25", 25),
            ("223.255.255.255", "223.255.255.255", 32),
            ("1.0.0.0/8", "1.0.0.0/8", 8),
        ] {
            let network = parse(raw).unwrap();
            assert_eq!(network.to_string(), canonical);
            assert_eq!(u32::from(network.prefix_length()), prefix_length);
        }
    }

    #[test]
    fn malformed_spellings() {
        for raw in [
            "",
            " 10.0.0.1",
            "10.0.0.1 ",
            "10.0.0",
            "10.0.0.1.2",
            "10.0.0.256",
            "010.0.0.1",
            "10.0.0.01",
            "10.0.0.1/",
            "10.0.0.1/024",
            "10.0.0.1/24/1",
            "10.0.0.1/+8",
            "0x0a.0.0.1",
            "10.0.0.-1",
            "10..0.1",
            "host.example.com",
            "10.0.0.1/33a",
            "\u{FF11}0.0.0.1",
            "::1",
            "10.0.0.1/100",
        ] {
            assert_eq!(parse(raw), Err(AddressProblem::Malformed {}), "{raw:?}");
        }
    }

    #[test]
    fn prefix_lengths_outside_8_to_32() {
        assert_eq!(parse("10.0.0.0/7"), Err(out_of_range(7)));
        assert_eq!(parse("10.0.0.0/0"), Err(out_of_range(0)));
        assert_eq!(parse("10.0.0.1/33"), Err(out_of_range(33)));
    }

    #[test]
    fn addresses_that_cannot_be_routed() {
        for raw in [
            "0.0.0.0",
            "0.1.2.3",
            "127.0.0.1",
            "127.0.0.0/8",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.0/8",
            "255.255.255.255",
        ] {
            assert_eq!(parse(raw), Err(AddressProblem::Unroutable {}), "{raw}");
        }
    }

    #[test]
    fn host_bits_after_the_prefix_suggest_the_network() {
        assert_eq!(
            parse("10.30.170.5/24"),
            Err(AddressProblem::HostBitsSet {
                network: "10.30.170.0/24".into()
            })
        );
    }

    #[test]
    fn route_destinations_with_subnet_masks() {
        assert_eq!(
            Ipv4Network::from_route(&route("10.30.161.16", "255.255.255.255"))
                .unwrap()
                .to_string(),
            "10.30.161.16"
        );
        assert_eq!(
            Ipv4Network::from_route(&route("10.30.170.0", "255.255.255.0"))
                .unwrap()
                .to_string(),
            "10.30.170.0/24"
        );
        assert_eq!(
            Ipv4Network::from_route(&route("10.0.0.0", "255.0.255.0")),
            Err(AddressProblem::NonContiguousMask {})
        );
        assert_eq!(
            Ipv4Network::from_route(&route("10.0.0.0", "0.0.0.0")),
            Err(out_of_range(0))
        );
        assert_eq!(
            bypass_network().route_through("10.20.161.254"),
            route("10.30.170.0", "255.255.255.0")
        );
    }

    #[test]
    fn containment() {
        assert!(office().contains_address(Ipv4Addr::new(10, 20, 161, 20)));
        assert!(!office().contains_address(Ipv4Addr::new(10, 20, 162, 20)));
        assert!(office().contains(&net("10.20.161.16")));
        assert!(office().contains(&office()));
        assert!(!office().contains(&net("10.20.0.0/16")));
        assert_eq!(
            Ipv4Network::enclosing("10.20.161.20", "255.255.255.0"),
            Ok(office())
        );
        assert_eq!(
            Ipv4Network::enclosing("10.20.161.20", "255.0.255.0"),
            Err(AddressProblem::NonContiguousMask {})
        );
    }
}
