use fvm_common_base::protocol::{
    ConnectRequest, DisconnectRequest, OtpSubmission, RefreshRequest, SessionId, SessionListRequest,
};
use fvm_system_protocol::service::{
    AppId, Requester, ServiceClient, SessionRecord, TunnelSetupOutcome,
};

/// One piece of work for the VPN connection manager.
pub enum VpnWork {
    /// Open a new session and bring its tunnel up.
    Connect(Requester, ConnectRequest),
    /// Open a new session in place of one that is up.
    Refresh(Requester, RefreshRequest),
    /// End one session.
    Disconnect(Requester, DisconnectRequest),
    /// End every session of the user whose app is quitting.
    DisconnectUser(Requester),
    /// Cancel the connection an app was making when its connection closed.
    CancelConnecting(AppId),
    /// The OTP for the session that is waiting for one.
    SubmitOtp(Requester, OtpSubmission),
    /// What came of applying a session's tunnel settings.
    TunnelSetupDone(TunnelSetupOutcome),
    /// Answer with the user's sessions.
    ListSessions(Requester, SessionListRequest),
    /// Log out a session an earlier Helper left on the server.
    LogOutLeftover(SessionRecord),
    /// The logout of a session an earlier Helper left ended.
    LeftoverLogoutEnded(SessionRecord),
    /// Something a session's thread saw, with the session it saw it in.
    Session(SessionId, SessionEvent),
    Stop,
}

/// What a session's thread saw while `OpenConnect` ran.
pub enum SessionEvent {
    GatewayConnected,
    OtpPrompted,
    Authenticated,
    TunnelSettingsReady,
    CertificateRejected,
    CredentialsAskedAgain,
    Ended,
}

/// The Helper's VPN sessions: opening them, replacing them and ending them.
pub struct VpnConnectionManager;

impl VpnConnectionManager {
    /// Creates the manager with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the manager");
        // 1. Create the queue that holds the manager's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Makes what the manager needs before it can run.
    pub fn init(&self) {
        todo!("get the OpenConnect library ready for use");
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: VpnWork) {
        todo!("put the work into the work queue");
    }

    /// Takes the manager's work one at a time until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                VpnWork::Connect(requester, request) => self.connect(&requester, &request),
                VpnWork::Refresh(requester, request) => self.refresh(&requester, &request),
                VpnWork::Disconnect(requester, request) => self.disconnect(&requester, &request),
                VpnWork::DisconnectUser(requester) => self.disconnect_user(&requester),
                VpnWork::ListSessions(requester, request) => {
                    self.list_sessions(&requester, &request);
                }
                VpnWork::LogOutLeftover(session) => self.log_out_leftover(&session),
                VpnWork::Stop => break,
                other => self.handle(&other),
            }
        }
        self.stop();
    }

    fn next_work(&self) -> VpnWork {
        todo!("wait for the next work in the work queue");
    }

    fn connect(&self, _requester: &Requester, _request: &ConnectRequest) {
        todo!("open a new session and bring its tunnel up");
        // 1. Start a session for the request on a thread of its own, and answer that the
        //    request was taken.
        // 2. When the gateway accepts the connection, report that the gateway is reached.
        // 3. When the gateway asks for an OTP, report it, wait for the OTP the app submits,
        //    answer that it was taken and give it to the session.
        // 4. When the login succeeds, report it, and report what a later Helper needs to log
        //    the session out.
        // 5. When the gateway gives the tunnel settings, ask for them to be applied to the
        //    system and wait for the outcome.
        // 6. Report the connection as up, with its tunnel address.
        //
        // While waiting in any step:
        // - A disconnect request, or the closing of the app's connection, cancels the connection.
        // - The session ending by itself fails the connection, with the reason it gave.
        // - A new connect request is refused.
        // - A stop cancels the connection and ends the run.
        // - Any other work is handled right away, the same way as while no connection is being
        //   made.
        //
        // A session that ends after its login is also reported as ended, so that its record is
        // removed and what was applied of its tunnel settings is undone.
    }

    fn refresh(&self, _requester: &Requester, _request: &RefreshRequest) {
        todo!("open a new session and end the one it replaces");
        // 1. Answer that the request was taken, and connect the new session while the old one
        //    keeps carrying traffic.
        // 2. Once the new session is up, end the old one and report the replacement.
        // 3. If the new session fails or is cancelled, keep the old one and report why.
    }

    fn disconnect(&self, _requester: &Requester, _request: &DisconnectRequest) {
        todo!("ask the session to stop");
        // The request is answered once the session was asked to stop.
        // The session ends on its own thread, and its end is reported when it arrives.
        // A request for a session the manager does not hold is refused.
    }

    fn disconnect_user(&self, _requester: &Requester) {
        todo!("answer the request and ask every session the user opened to stop");
        // 1. Answer that the request was taken.
        // 2. Ask every session the user opened to stop.
        //
        // Each session ends on its own thread, and its end is reported when it arrives.
    }

    fn list_sessions(&self, _requester: &Requester, _request: &SessionListRequest) {
        todo!("answer with the sessions the user has, so that the user's app can take them over");
    }

    fn log_out_leftover(&self, _session: &SessionRecord) {
        todo!("start logging out a session an earlier Helper left on the server");
        // The logout runs on a thread of its own, with the server address and the cookie from
        // the record, so that it does not hold up the manager's other work.
        // Its end arrives as work.
    }

    fn handle(&self, _work: &VpnWork) {
        todo!("handle work that arrives while no connection is being made");
        // A session that ended is forgotten, and its end is reported with the reason.
        // The logout of a leftover session that ended is reported, so that its record can be
        // removed.
        // An OTP or a tunnel setup outcome that no connection is waiting for is refused.
        // A request to cancel what an app was connecting finds nothing to cancel.
    }

    fn stop(&self) {
        todo!("ask every session to stop and wait for its thread");
    }
}
