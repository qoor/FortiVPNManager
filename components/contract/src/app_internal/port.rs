//! What the runtime asks of the OS (port requests and their responses) and what it has the OS do
//! without an answer (port actions).

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use super::SecretText;
use super::section::BypassFailure;
use crate::app_helper::{AttemptId, StopReason};

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PortRequest {
    /// Answered by `Secret`, or `SecretStoreFailed`.
    ReadSecret { item: SecretItem },
    /// Answered by `HasSecret`, or `SecretStoreFailed`. Never reads the secret, so the OS asks the
    /// user for nothing.
    HasSecret { item: SecretItem },
    /// Answered by `SecretChanged`, or `SecretStoreFailed`.
    WriteSecret { item: SecretItem, value: SecretText },
    /// Answered by `SecretChanged`, or `SecretStoreFailed`; deleting a missing item succeeds.
    DeleteSecret { item: SecretItem },

    /// The Helper's registration as a system service. Answered by `Registration`.
    HelperRegistration {},
    /// Answered by `Registered`, or `RegistrationFailed`.
    RegisterHelper {},
    /// The running Helper's build. Answered by `HelperBuild`, or `HelperUnreachable`. A connection
    /// whose Helper answers with the app's build counts as verified. Retried once on a fresh
    /// connection when the connection fails: a Helper whose executable a reinstall replaced exits
    /// instead of answering, and the system starts the new one for the next connection.
    HelperBuild {},
    /// Answered by `HelperReply` (`HelperReplyFlag` for `ExitWhenIdle`), `HelperUnreachable`, or
    /// `HelperBuildMismatch`.
    CallHelper { call: HelperCall },

    /// The interface the routing table uses for `destination`. Answered by `RouteInterface`.
    RouteInterface { destination: Ipv4Addr },
    /// Answered by `InterfaceAddresses`.
    InterfaceAddresses {},
    /// The primary network's IPv4 router. Answered by `PrimaryRouter`.
    PrimaryRouter {},
    /// Answered by `ScreenState`.
    ScreenState {},

    /// Opens `url` in the default browser. Answered by `BrowserOpened`.
    OpenBrowser { url: String },
    /// The app's registration as a login item. Answered by `Registration`.
    LoginItemRegistration {},
    /// Answered by `Registered`, or `RegistrationFailed`.
    SetLoginItem { enabled: bool },

    /// The SSL VPN connections the official FortiClient keeps, read only; Windows imports its
    /// settings from them. Answered by `FortiClientConnections`.
    FortiClientConnections {},
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PortResponse {
    Secret {
        value: Option<SecretText>,
    },
    HasSecret {
        exists: bool,
    },
    SecretChanged {},
    SecretStoreFailed {
        failure: SecretStoreFailure,
    },

    Registration {
        status: RegistrationStatus,
    },
    Registered {},
    RegistrationFailed {
        error: RegistrationError,
    },
    HelperBuild {
        build: String,
    },
    /// The Helper's reply as it sent it: absent for an empty (success) reply.
    HelperReply {
        json: Option<String>,
    },
    HelperReplyFlag {
        value: bool,
    },
    HelperUnreachable {
        failure: HelperChannelFailure,
    },
    /// A call that needs a verified connection was not sent: the Helper answered with another
    /// build.
    HelperBuildMismatch {
        helper_build: String,
    },

    RouteInterface {
        interface: Option<String>,
    },
    InterfaceAddresses {
        addresses: Vec<InterfaceAddress>,
    },
    PrimaryRouter {
        router: Option<Ipv4Addr>,
    },
    ScreenState {
        locked: bool,
        displays_asleep: bool,
    },

    BrowserOpened {
        opened: bool,
    },

    /// Empty where FortiClient keeps none, or on an OS whose FortiClient the app doesn't read.
    FortiClientConnections {
        connections: Vec<FortiClientConnection>,
    },
}

/// An SSL VPN connection of FortiClient: its name and its `Server` value as FortiClient keeps it,
/// usually `host:port`.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FortiClientConnection {
    pub name: String,
    pub server: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PortAction {
    /// A line for the system log. Never with a secret.
    Log {
        level: LogLevel,
        category: LogCategory,
        message: String,
    },
    /// Shows a notification, replacing an earlier one of the same kind. The UI's string code
    /// builds its text; the OS integration only posts it.
    PostNotification { notification: UserNotification },
    /// Keeps the app from being suspended or throttled while a connection is wanted.
    HoldActivity { hold: bool },
    /// Opens the system settings where the Helper is approved.
    OpenHelperApprovalSettings {},
    /// Lets the app quit, after `SystemEvent::QuitRequested`. The runtime's last message.
    ApproveQuit {},
}

/// A secret the app keeps in the OS's secret storage.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SecretItem {
    VpnPassword {
        username: String,
        host: String,
    },
    GmailAppPassword {
        address: String,
    },
    /// Matches the address without regard to case.
    GoogleRefreshToken {
        address: String,
    },
    GoogleClientSecret {
        client_id: String,
    },
}

/// Why the secret storage failed, as the OS describes it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SecretStoreFailure {
    pub description: String,
}

/// One call on the Helper, over the one connection the OS integration keeps. It opens a new one
/// when there is none or the last one was lost, and reports `SystemEvent::HelperConnectionLost`
/// when the connection that carries the runs goes away.
///
/// - JSON payloads are the `app_helper` messages, written and read by the runtime only.
/// - `Connect`, `SubmitOtp`, `ReadBypassRoutes` and `WriteBypassRoutes` go only over a verified
///   connection. When it is not verified yet, the OS integration asks for the build first and
///   answers `HelperBuildMismatch` if it differs.
/// - `Status`, `ReadBypassRoutes` and `WriteBypassRoutes` are retried once on a fresh connection
///   when the connection fails, since repeating them is harmless. The other calls are never
///   retried or moved to another connection.
/// - The runtime sends the next call only after the previous one was answered, except the route
///   calls: the Helper handles routes apart from the tunnel, so they may run alongside.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HelperCall {
    /// `json` is a `RunRequest`. Replied to with nothing, or a `HelperError`.
    Connect { json: SecretText },
    /// Replied to with nothing, or a `HelperError`.
    SubmitOtp {
        attempt: AttemptId,
        code: SecretText,
    },
    /// Replied to with nothing once the stop is accepted, or a `HelperError`; the run's end is
    /// its exit event.
    Disconnect {
        attempt: AttemptId,
        reason: StopReason,
    },
    /// Replied to with a `HelperStatus`.
    Status {},
    /// Asks a Helper with no run to exit, so the system starts this build's Helper next time.
    /// Replied to with a flag: whether it will exit. Works across builds. After the reply the OS
    /// integration drops the connection, which clears its verified mark and reports
    /// `SystemEvent::HelperConnectionLost`; the runtime expects that, as no run is going on.
    ExitWhenIdle {},
    /// Replied to with a `BypassRoutesReply`.
    ReadBypassRoutes {},
    /// `json` is a `BypassRoutesRequest`. Replied to with a `BypassRoutesReply`.
    WriteBypassRoutes { json: String },
}

/// Why a Helper call got no reply.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HelperChannelFailure {
    /// The app is not signed in a way the Helper connection can check.
    AppNotSigned {},
    /// As the OS describes it.
    Connection { description: String },
    /// The connection was lost before the call could be sent.
    ConnectionLost {},
    /// No reply within the time limit.
    NoReply {},
    /// The Helper's build reply could not be read, as the decoder describes it.
    UnreadableReply { description: String },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegistrationStatus {
    NotRegistered,
    Enabled,
    /// Registered, waiting for the user's approval in the system settings.
    RequiresApproval,
    /// The OS cannot find what should be registered.
    NotFound,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RegistrationError {
    /// Only the installed copy of the app may register.
    NotInstalledCopy {},
    /// As the OS describes it.
    Failed { description: String },
}

/// One address of a network interface; an interface appears once per address.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceAddress {
    pub name: String,
    pub up: bool,
    pub ipv4: Option<Ipv4Addr>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Info,
    Notice,
    Error,
}

/// The system log category.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogCategory {
    Connection,
    Readiness,
    Config,
    State,
    Logfile,
    Import,
    Oauth,
    Otp,
    BypassRoutes,
    LoginItem,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UserNotification {
    /// Writing the exception routes kept failing; the runtime stopped retrying.
    ExceptionRulesNotApplied {
        retries: u32,
        failure: BypassFailure,
    },
}
