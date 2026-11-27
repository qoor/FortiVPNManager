/// What names one VPN session to the app, the Helper and the logs.
pub struct SessionId;

/// One request an app sends to the Helper on the request channel.
pub enum AppRequest {
    Connect(ConnectRequest),
    Refresh(RefreshRequest),
    Disconnect(DisconnectRequest),
    SubmitOtp(OtpSubmission),
    ListSessions(SessionListRequest),
    ReadExceptionRules(ExceptionRulesReadRequest),
    WriteExceptionRules(ExceptionRulesWriteRequest),
    PrepareUpdate(UpdatePrepareRequest),
    InstallUpdate(UpdateInstallRequest),
    ReadPreparedUpdate(PreparedUpdateReadRequest),
    DiscardPreparedUpdate(PreparedUpdateDiscardRequest),
    Quit(AppQuitRequest),
}

/// Asks for a new session to be opened and its tunnel brought up.
pub struct ConnectRequest;

/// Asks for a new session to be opened in place of one that is up.
pub struct RefreshRequest;

/// Asks for one session to be ended.
pub struct DisconnectRequest;

/// The OTP for a session that is waiting for one.
pub struct OtpSubmission;

/// Asks which sessions the user has in the Helper.
pub struct SessionListRequest;

/// Asks for the user's exception rules and the state of their routes.
pub struct ExceptionRulesReadRequest;

/// The user's exception rules, to replace the ones the Helper holds.
pub struct ExceptionRulesWriteRequest;

/// Asks for a downloaded installer to be made ready to install.
pub struct UpdatePrepareRequest;

/// Asks for the prepared update to be installed.
pub struct UpdateInstallRequest;

/// Asks which update is prepared.
pub struct PreparedUpdateReadRequest;

/// Asks for the prepared update to be deleted.
pub struct PreparedUpdateDiscardRequest;

/// Tells the Helper that the app is quitting.
pub struct AppQuitRequest;

/// The Helper's answer to one request, on the request channel.
pub struct AppResponse;

/// A change of a state the Helper manages, on the propagation channel.
pub enum StateChange {
    Vpn(VpnStateChange),
    ExceptionRoute(ExceptionRouteStateChange),
}

/// A change of a VPN connection's state.
pub struct VpnStateChange;

/// A change of the state of the exception routes.
pub struct ExceptionRouteStateChange;

/// One message as it arrived on the OS connection, with the channel it is marked for.
pub struct ChannelMessage;

/// One recorded line, as the Helper sends it to the apps on the log channel.
pub struct LogLine;
