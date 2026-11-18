//! Messages between the Helper's OS side and the Helper entry in the Helper process: the Helper's
//! Swift side on macOS, the service's own part on Windows. The OS side accepts and checks
//! app connections and answers the version check itself; every other app request goes to the entry
//! unread. The entry decides which exception routes to add and remove, and the OS side changes the
//! routing table as it asks, as it repairs the routes a run left. The JSON follows Swift's enum
//! coding (see `app_helper`), so the Swift side, `components/contract/macos/HelperInternal.swift`,
//! uses synthesized `Codable`. The entry only receives `ToEntry` and only sends `FromEntry`.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};

use serde::{Deserialize, Serialize};

use crate::app_helper::{BypassRoute, BypassRoutesSnapshot, HelperError};
use crate::backend::{IpNetwork, NetworkSettings, RouteRepairOutcome, TunnelDevice};

/// Sent by the OS side.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToEntry {
    /// An app connection passed the OS side's check, the signature on macOS and the pipe's access
    /// rights on Windows; `app` names it in later messages. `uid` tells the apps' users apart: the
    /// uid on macOS, the session id on Windows, where only one user may connect. The OS side holds
    /// the connection until `FromEntry::Admitted` answers.
    AppConnected {
        app: u64,
        uid: u32,
    },
    /// The connection is gone, whether the app quit, crashed or was killed.
    AppDisconnected {
        app: u64,
    },
    /// `id` names the request in its reply.
    Request {
        id: u64,
        app: u64,
        request: AppRequest,
    },
    /// macOS: the Helper's own executable was replaced on disk. Once Swift has seen the
    /// replacement, it sends this before announcing any new app connection, so Rust can refuse
    /// that connection. Windows cannot replace a running executable.
    ExecutableReplaced {},
    /// The OS asked the Helper to stop: SIGTERM on macOS, the service stop on Windows.
    TerminationRequested {},
    /// Answers `FromEntry::RepairRoutes`.
    RoutesRepaired {
        outcome: RouteRepairOutcome,
    },
    /// Answers `FromEntry::ChangeBypassRoutes`.
    BypassRoutesChanged {
        outcome: BypassRoutesOutcome,
    },
    NetworkChanged {
        outcome: NetworkOutcome,
    },
}

/// An app request with its values exactly as the app sent them; Rust checks them, so a bad value
/// becomes an error reply rather than an unreadable message.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppRequest {
    /// `json` is the app's `RunRequest` payload.
    Connect {
        json: String,
    },
    SubmitOtp {
        attempt: String,
        code: String,
    },
    /// `reason` is a `StopReason` raw value.
    Disconnect {
        attempt: String,
        reason: String,
    },
    Status {},
    /// macOS only: the Windows service answers it itself, since Windows does not start it again.
    ExitWhenIdle {},
    ReadBypassRoutes {},
    /// `json` is the app's `BypassRoutesRequest` payload.
    WriteBypassRoutes {
        json: String,
    },
}

/// Redacts the connect payload, which holds the password, and the OTP: Debug output can reach panic
/// messages and so crash reports.
impl fmt::Debug for AppRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = format_args!("<redacted>");
        match self {
            Self::Connect { .. } => f.debug_struct("Connect").field("json", &redacted).finish(),
            Self::SubmitOtp { attempt, .. } => f
                .debug_struct("SubmitOtp")
                .field("attempt", attempt)
                .field("code", &redacted)
                .finish(),
            Self::Disconnect { attempt, reason } => f
                .debug_struct("Disconnect")
                .field("attempt", attempt)
                .field("reason", reason)
                .finish(),
            Self::Status {} => f.write_str("Status"),
            Self::ExitWhenIdle {} => f.write_str("ExitWhenIdle"),
            Self::ReadBypassRoutes {} => f.write_str("ReadBypassRoutes"),
            Self::WriteBypassRoutes { json } => f
                .debug_struct("WriteBypassRoutes")
                .field("json", json)
                .finish(),
        }
    }
}

/// Sent by Rust. Swift passes events and output to each app connection in the order they arrive,
/// because the app reads a run's end from the last event, not from a reply.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FromEntry {
    /// Leftovers of an earlier Helper are cleaned up; accept app connections from now on.
    Ready {},
    /// Answers `ToEntry::AppConnected`. A refused connection is dropped without
    /// `ToEntry::AppDisconnected`.
    Admitted {
        app: u64,
        accept: bool,
    },
    /// Answers a connect, submitOtp, disconnect, status or exception-route request with the data
    /// for the app; `None` is the empty (success) reply.
    Reply {
        id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        json: Option<String>,
    },
    /// Answers an exitWhenIdle request.
    ReplyFlag {
        id: u64,
        value: bool,
    },
    /// A `HelperEvent` for that app connection.
    SendEvent {
        app: u64,
        json: String,
    },
    /// A batch of `OutputLine`s for that app connection.
    SendOutput {
        app: u64,
        json: String,
    },
    /// Check the routing table a client may have left broken and repair it; `gateway` is the VPN
    /// gateway it connected to, if known.
    RepairRoutes {
        #[serde(skip_serializing_if = "Option::is_none")]
        gateway: Option<Ipv4Addr>,
    },
    /// Change the exception routes in the routing table.
    ChangeBypassRoutes {
        change: BypassRoutesChange,
    },
    ChangeNetwork {
        change: NetworkChange,
    },
    /// Accept no more connections and exit once what was sent to apps has left.
    ExitAfterDraining {},
    /// Exit at once, even if something sent to apps has not left yet.
    ExitNow {},
    /// A line for the unified log.
    Log {
        level: LogLevel,
        category: LogCategory,
        message: String,
    },
}

/// Exception route work for the OS side, which answers with `ToEntry::BypassRoutesChanged`. It
/// reads the routing table once, and the primary network unless `network` skips it; then removes
/// and adds, one route at a time; then, if it changed anything, reads the table once more to see
/// what it now has. A route counts as in the table only as the Helper adds one: with the same
/// destination, subnet mask and gateway, a static route on macOS (not one the system copied from
/// another route), and on Windows on the interface it belongs to.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BypassRoutesChange {
    pub network: NetworkCheck,
    /// Routes the Helper added, each removed where the table has it.
    pub remove: Vec<AddedRoute>,
    /// Each added through the primary network where it does not have it yet. The OS side checks
    /// them, since they come from an app: at most as many as the settings file's target list
    /// holds, no repeats, and each to an exception target through the primary network's router.
    /// Otherwise it changes nothing.
    pub add: Vec<BypassRoute>,
    /// Looked up on the primary network at the end; none without a network.
    pub look_up: Vec<BypassRoute>,
}

/// An exception route the Helper added, with the network service it went through: the interface
/// GUID on Windows, where a route belongs to an interface; the service ID on macOS. As recorded
/// for a later Helper.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedRoute {
    pub route: BypassRoute,
    #[serde(rename = "serviceID")]
    pub service_id: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkCheck {
    /// Only removing: the network does not matter.
    Skip {},
    Read {},
    /// Change nothing unless the primary network is still exactly `network`.
    Expect {
        network: BypassRoutesSnapshot,
    },
}

/// What came of a `BypassRoutesChange`.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BypassRoutesOutcome {
    Changed {
        /// The primary network as read; `None` when skipped.
        #[serde(default)]
        network: Option<BypassRoutesSnapshot>,
        /// The routes to add that the table did not have and has now.
        added: Vec<BypassRoute>,
        /// The routes to look up that the table has.
        found: Vec<BypassRoute>,
        /// The routes to remove that the table still has.
        left: Vec<AddedRoute>,
    },
    /// Nothing was changed: the table or the primary network could not be read, there is no
    /// primary network, it is not the one expected, or a route to add is not one the Helper
    /// writes.
    Refused { error: HelperError },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkChange {
    LookUp {
        tunnel: TunnelDevice,
        outside: Vec<IpNetwork>,
    },
    Apply {
        settings: Box<NetworkSettings>,
        #[serde(skip_serializing_if = "Option::is_none")]
        previous: Option<Box<NetworkSettings>>,
        remove: Vec<OutsideRoute>,
        add: Vec<OutsideRoute>,
    },
    Prepare {
        settings: Box<NetworkSettings>,
        add: Vec<OutsideRoute>,
    },
    Switch {
        settings: Box<NetworkSettings>,
        replaced: Box<NetworkSettings>,
    },
    Revert {
        changes: OutsideChanges,
    },
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutsideRoute {
    pub destination: IpNetwork,
    pub router: IpAddr,
    #[serde(rename = "serviceID")]
    pub service_id: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutsideChanges {
    #[serde(rename = "dnsServiceID")]
    pub dns_service_id: String,
    pub routes: Vec<OutsideRoute>,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkOutcome {
    LookedUp {
        #[serde(rename = "dnsServiceID")]
        dns_service_id: String,
        missing: Vec<OutsideRoute>,
    },
    Applied {
        added: Vec<OutsideRoute>,
        left: Vec<OutsideRoute>,
    },
    Reverted {
        left: Vec<OutsideRoute>,
    },
    Failed {
        step: NetworkStep,
        error: String,
        added: Vec<OutsideRoute>,
        left: Vec<OutsideRoute>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkStep {
    Network,
    Addresses,
    Mtu,
    Routes,
    OutsideRoutes,
    Dns,
    Wins,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Info,
    Notice,
    Error,
}

/// The unified-log category.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogCategory {
    Helper,
    Routes,
}
