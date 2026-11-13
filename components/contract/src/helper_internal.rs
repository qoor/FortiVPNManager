//! Messages between the Helper's OS side and the Helper entry in the Helper process: Swift's OS
//! integration Helper on macOS, the service's own part on Windows. The OS side accepts and checks
//! app connections, answers the version check, repairs routes and reads and writes the bypass
//! routes itself; every other app request goes to the entry unread. The JSON follows Swift's enum
//! coding (see `app_helper`), so the Swift side, `components/contract/macos/HelperInternal.swift`,
//! uses synthesized `Codable`. The entry only receives `ToEntry` and only sends `FromEntry`.

use std::fmt;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::backend::RouteRepairOutcome;

/// Sent by the OS side.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToEntry {
    /// An app connection passed the OS side's check, the signature on macOS and the pipe's access
    /// rights on Windows; `app` names it in later messages. `uid` tells the apps' users apart: the
    /// uid on macOS, the session id on Windows, where only one user may connect. The OS side holds
    /// the connection until `FromEntry::Admitted` answers.
    AppConnected { app: u64, uid: u32 },
    /// The connection is gone, whether the app quit, crashed or was killed.
    AppDisconnected { app: u64 },
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
    /// `reason` is a `StopReason` raw value.
    Disconnect {
        attempt: String,
        reason: String,
    },
    Status {},
    /// macOS only: the Windows service answers it itself, since Windows does not start it again.
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

/// Sent by Rust. Swift passes events and output to each app connection in the order they arrive,
/// because the app reads a run's end from the last event, not from a reply.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FromEntry {
    /// Leftovers of an earlier Helper are cleaned up; accept app connections from now on.
    Ready {},
    /// Answers `ToEntry::AppConnected`. A refused connection is dropped without
    /// `ToEntry::AppDisconnected`.
    Admitted { app: u64, accept: bool },
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

/// The unified-log category.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogCategory {
    Helper,
    Routes,
}
