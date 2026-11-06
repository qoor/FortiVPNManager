//! Messages between the OS integration Helper (Swift) and the Helper entry (Rust) in the Helper
//! process. Swift accepts and checks app connections, answers the version check, repairs routes and
//! runs networksetup itself; every other app request goes to Rust unread. The JSON follows Swift's
//! enum coding (see `app_helper`), so the Swift side, `components/contract/macos/HelperInternal.swift`,
//! uses synthesized `Codable`. Rust only receives `ToEntry` and only sends `FromEntry`.

use std::fmt;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

/// Sent by Swift.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToEntry {
    /// An app connection passed the signature check; `app` names it in later messages.
    AppConnected { app: u64, uid: u32 },
    /// The connection is gone, whether the app quit, crashed or was killed.
    AppDisconnected { app: u64 },
    /// `id` names the request in its reply.
    Request {
        id: u64,
        app: u64,
        request: AppRequest,
    },
    /// The Helper's own executable was replaced on disk. Once Swift has seen the replacement, it
    /// sends this before announcing any new app connection, so Rust can refuse that connection.
    ExecutableReplaced {},
    /// The process received SIGTERM.
    TerminationRequested {},
    /// Answers `FromEntry::RepairRoutes`.
    RoutesRepaired { outcome: RouteRepairOutcome },
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
    /// An empty `attempt` means whatever runs; `reason` is a `StopReason` raw value.
    Disconnect {
        attempt: String,
        reason: String,
    },
    Status {},
    ExitWhenIdle {},
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
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RouteRepairOutcome {
    Repaired,
    NotNeeded,
    Failed,
}

/// Sent by Rust. Swift passes events and output to each app connection in the order they arrive,
/// because the app reads a run's end from the last event, not from a reply.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FromEntry {
    /// Leftovers of an earlier Helper are cleaned up; accept app connections from now on.
    Ready {},
    /// Answers a connect, submitOtp, disconnect or status request with the data for the app;
    /// `None` is the empty (success) reply.
    Reply {
        id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        json: Option<String>,
    },
    /// Answers an exitWhenIdle request.
    ReplyFlag { id: u64, value: bool },
    /// A `HelperEvent` for that app connection.
    SendEvent { app: u64, json: String },
    /// A batch of `OutputLine`s for that app connection.
    SendOutput { app: u64, json: String },
    /// Check the routing table a client may have left broken and repair it; `gateway` is the VPN
    /// gateway it connected to, if known.
    RepairRoutes {
        #[serde(skip_serializing_if = "Option::is_none")]
        gateway: Option<Ipv4Addr>,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Info,
    Notice,
    Error,
}

/// The unified-log category, kept from the Swift Helper so existing log filters still match.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogCategory {
    Helper,
    Routes,
}
