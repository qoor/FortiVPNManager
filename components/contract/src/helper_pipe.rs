//! Messages between the Windows app's OS integration (C#) and the Windows Helper service over the
//! Helper's named pipe, in XPC's place on macOS. Each message is framed as in `stream`: a 4-byte
//! big-endian length and that many bytes of UTF-8 JSON. The app numbers its requests; the Helper
//! answers each one once with that number, and sends a run's events and output in the order they
//! happened on the same pipe. The JSON payloads are the `app_helper` messages, which the OS
//! integration passes to and from the runtime unread, as on macOS. The coding follows
//! `app_helper`'s, so the C# side reads the same shapes as Swift. Rust only reads requests and only
//! writes messages, so each type derives that one direction; the C# side does the opposite.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The Helper's pipe. Only the user chosen at install time and the system account may open it, and
/// the system account owns it.
pub const PIPE_NAME: &str = r"\\.\pipe\com.qoor.FortiVPNManager.Helper";

/// The largest message the Helper accepts from an app. Other programs of the same user may connect
/// too, and a bigger length must not make the Helper allocate it.
pub const MAX_MESSAGE_BYTES: u32 = 1 << 20;

/// Sent by the app. `id` names the request in the Helper's answer.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PipeRequest {
    /// The build check, answered by `PipeMessage::Build`. `app` is the app's build; a connection
    /// whose app build equals the Helper's may send connect, submitOtp and the bypass-route
    /// requests. Goes between builds, so its shape and its answer's never change.
    Build { id: u64, app: String },
    /// `json` is a `RunRequest`. Answered by `Reply`: nothing, or a `HelperError`.
    Connect { id: u64, json: String },
    /// Answered by `Reply`: nothing, or a `HelperError`.
    SubmitOtp {
        id: u64,
        attempt: String,
        code: String,
    },
    /// `reason` is a `StopReason` raw value. Answered by `Reply` once the stop is accepted: nothing,
    /// or a `HelperError`; the run's end is its exit event.
    Disconnect {
        id: u64,
        attempt: String,
        reason: String,
    },
    /// Answered by `Reply` with a `HelperStatus`.
    Status { id: u64 },
    /// Answered by `ReplyFlag`; the Windows Helper always answers false, since Windows does not
    /// start a service again that stopped by itself. Goes between builds, so its shape and its
    /// answer's never change.
    ExitWhenIdle { id: u64 },
    /// Answered by `Reply` with a `BypassRoutesReply`.
    ReadBypassRoutes { id: u64 },
    /// `json` is a `BypassRoutesRequest`. Answered by `Reply` with a `BypassRoutesReply`.
    WriteBypassRoutes { id: u64, json: String },
}

impl PipeRequest {
    pub fn id(&self) -> u64 {
        match *self {
            Self::Build { id, .. }
            | Self::Connect { id, .. }
            | Self::SubmitOtp { id, .. }
            | Self::Disconnect { id, .. }
            | Self::Status { id }
            | Self::ExitWhenIdle { id }
            | Self::ReadBypassRoutes { id }
            | Self::WriteBypassRoutes { id, .. } => id,
        }
    }
}

/// Redacts the connect payload, which holds the password, and the OTP: Debug output can reach panic
/// messages and so crash reports.
impl fmt::Debug for PipeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = format_args!("<redacted>");
        match self {
            Self::Build { id, app } => f
                .debug_struct("Build")
                .field("id", id)
                .field("app", app)
                .finish(),
            Self::Connect { id, .. } => f
                .debug_struct("Connect")
                .field("id", id)
                .field("json", &redacted)
                .finish(),
            Self::SubmitOtp { id, attempt, .. } => f
                .debug_struct("SubmitOtp")
                .field("id", id)
                .field("attempt", attempt)
                .field("code", &redacted)
                .finish(),
            Self::Disconnect {
                id,
                attempt,
                reason,
            } => f
                .debug_struct("Disconnect")
                .field("id", id)
                .field("attempt", attempt)
                .field("reason", reason)
                .finish(),
            Self::Status { id } => f.debug_struct("Status").field("id", id).finish(),
            Self::ExitWhenIdle { id } => f.debug_struct("ExitWhenIdle").field("id", id).finish(),
            Self::ReadBypassRoutes { id } => {
                f.debug_struct("ReadBypassRoutes").field("id", id).finish()
            }
            Self::WriteBypassRoutes { id, json } => f
                .debug_struct("WriteBypassRoutes")
                .field("id", id)
                .field("json", json)
                .finish(),
        }
    }
}

/// Sent by the Helper.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PipeMessage {
    /// Answers `PipeRequest::Build` with the Helper's build.
    Build { id: u64, build: String },
    /// Answers a request with its JSON payload; `None` is the empty (success) reply.
    Reply {
        id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        json: Option<String>,
    },
    /// Answers `PipeRequest::ExitWhenIdle`.
    ReplyFlag { id: u64, value: bool },
    /// A `HelperEvent` of a run this connection started.
    Event { json: String },
    /// A batch of `OutputLine`s of a run this connection started.
    Output { json: String },
}
