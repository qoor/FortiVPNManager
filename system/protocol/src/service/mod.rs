use std::sync::mpsc::Sender;

use fvm_common_base::protocol::{AppRequest, AppResponse};

/// One request to the main service, with everything it takes to act on it.
pub enum ServiceMessage {
    /// A request an app sent.
    AppRequested(Requester, AppRequest),
    /// An app's connection closed.
    AppDisconnected(AppId),
    /// The answer to an app's request.
    Replied(Requester, AppResponse),
    /// A connection reached its next stage.
    ConnectProgressed(ConnectProgress),
    /// A connection ended as up, failed or cancelled.
    ConnectFinished(ConnectOutcome),
    /// A replacement ended as done, failed or cancelled.
    RefreshFinished(RefreshOutcome),
    /// A session is logged in, with what a later Helper needs to end it.
    SessionOpened(SessionRecord),
    /// A session ended.
    SessionEnded(SessionEnd),
    /// A session's tunnel settings are to be applied to the system.
    TunnelSetupRequested(TunnelSetup),
    /// A session's tunnel settings were applied, or could not be.
    TunnelSetupDone(TunnelSetupOutcome),
    /// Tunnel settings are about to be applied to the system, with what it takes to undo them.
    TunnelSettingsApplied(TunnelSettingsRecord),
    /// Tunnel settings were undone.
    TunnelSettingsReverted(TunnelSettingsRecord),
    /// The exception routes in the system change; it carries all that are in place or about
    /// to be added.
    ExceptionRoutesChanged(ExceptionRoutesRecord),
    /// Traffic started or stopped going through a session's tunnel.
    TunnelRouteStateChanged(TunnelRouteState),
    /// Exception routes were applied, or could not be.
    ExceptionRouteStateChanged(ExceptionRouteState),
    /// An earlier Helper left a session on the server.
    LeftoverSessionFound(SessionRecord),
    /// A session an earlier Helper left was logged out.
    LeftoverSessionLoggedOut(SessionRecord),
    /// An earlier Helper left tunnel settings in the system.
    LeftoverTunnelSettingsFound(TunnelSettingsRecord),
    /// An earlier Helper left exception routes in the system.
    LeftoverExceptionRoutesFound(ExceptionRoutesRecord),
    /// Everything an earlier Helper left has been reported.
    LeftoverCheckFinished,
    /// The OS told the Helper service to stop.
    StopRequested,
}

/// What names one connected app in the Helper.
pub struct AppId;

/// Who sent a request: the app, its user and the number the app gave the request.
pub struct Requester;

/// The stage a connection reached.
pub struct ConnectProgress;

/// How a connection ended, with the tunnel address or the reason.
pub struct ConnectOutcome;

/// How a replacement ended, with the reason when it did not happen.
pub struct RefreshOutcome;

/// What a later Helper needs to log a session out of the server.
pub struct SessionRecord;

/// The session that ended and why.
pub struct SessionEnd;

/// The tunnel settings the gateway gave a session.
pub struct TunnelSetup;

/// Whether a session's tunnel settings were applied, and the step that failed.
pub struct TunnelSetupOutcome;

/// What it takes to undo the tunnel settings applied for a session.
pub struct TunnelSettingsRecord;

/// The exception routes the Helper has put in the system.
pub struct ExceptionRoutesRecord;

/// Whether traffic for the VPN's networks goes through a session's tunnel.
pub struct TunnelRouteState;

/// Which exception routes are in place and which could not be added.
pub struct ExceptionRouteState;

/// Puts messages into the main service's queue.
pub struct ServiceClient;

impl ServiceClient {
    /// Creates a client of the main service's queue.
    #[must_use]
    pub fn new(_queue: Sender<ServiceMessage>) -> Self {
        todo!("keep the sending end of the main service's queue");
    }

    /// Puts one message into the main service's queue.
    pub fn push(&self, _message: ServiceMessage) {
        todo!("put the message into the main service's queue");
    }
}
