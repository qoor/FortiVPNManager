use std::sync::mpsc::Sender;

use fvm_common_base::protocol::{
    AppRequest, AppResponse, ExceptionRouteStateChange, VpnStateChange,
};

/// One request to the main service, with everything it takes to act on it.
pub enum ServiceMessage {
    /// The user did something in the UI.
    UserActed(UserAction),
    /// A request is to be sent to the Helper.
    HelperRequested(AppRequest),
    /// The Helper is connected.
    HelperConnected,
    /// The connection to the Helper closed.
    HelperDisconnected,
    /// The Helper could not be started or reached.
    HelperStartFailed(HelperStartFailure),
    /// The Helper waits for the user's approval in the system settings.
    HelperApprovalNeeded,
    /// The Helper answered a request; the request comes with its answer.
    HelperReplied(AppRequest, AppResponse),
    /// A request could not be sent to the Helper, or the connection closed before its answer.
    HelperUnanswered(AppRequest),
    /// The Helper told of a change of the VPN connection state.
    HelperVpnStateChanged(VpnStateChange),
    /// The Helper told of a change of the exception routes' state.
    HelperExceptionRouteStateChanged(ExceptionRouteStateChange),
    /// The VPN account is needed to make a connection.
    VpnAccountNeeded,
    /// The VPN account to connect with.
    VpnAccountReady(VpnAccount),
    /// The gateway asked for an OTP.
    OtpNeeded(OtpNeed),
    /// The OTP asked for is no longer needed.
    OtpNoLongerNeeded,
    /// The OTP the gateway asked for.
    OtpReady(Otp),
    /// The OTP could not be got.
    OtpUnavailable(OtpFailure),
    /// Whether the accounts are ready to be used changed.
    AccountStateChanged(AccountState),
    /// The Gmail connection test ended.
    GmailTested(GmailTestOutcome),
    /// What the UI shows about the connection changed.
    ConnectionStateChanged(ConnectionState),
    /// The connection ended.
    ConnectionEnded,
    /// The connection is to be ended so that an update can be installed.
    DisconnectForInstallAsked,
    /// The installation the connection was ended for was given up.
    InstallAbandoned,
    /// The settings were read or changed; it carries all of them.
    SettingsChanged(Settings),
    /// The settings file has a problem.
    SettingsProblemFound(SettingsProblem),
    /// Loading the settings of an existing VPN client ended.
    ExistingSettingsLoaded(ExistingSettingsOutcome),
    /// The app state that was kept from the last run.
    AppStateLoaded(AppState),
    /// A value of the app state is to be kept for the next run.
    AppStateChanged(AppState),
    /// What the UI shows about updates changed.
    UpdateStateChanged(UpdateState),
    /// The time moved into or out of the schedule.
    ScheduleStateChanged(ScheduleState),
    /// The user left the device alone or came back to it.
    IdleStateChanged(IdleState),
    /// The device went to sleep or woke up.
    SleepStateChanged(SleepState),
    /// Something the user is to be notified of.
    NoticeRaised(Notice),
    /// The app can quit now.
    QuitReady,
    /// The app was asked to stop.
    StopRequested,
}

/// What the user did in the UI, sorted by the feature that acts on it.
pub enum UserAction {
    Connection(ConnectionAction),
    Config(ConfigAction),
    Auth(AuthAction),
    Update(UpdateAction),
}

/// What the user did about the connection.
pub enum ConnectionAction {
    Connect,
    Disconnect,
    ReconnectNow,
    Quit,
}

/// What the user did about the settings.
pub enum ConfigAction {
    Change(SettingChange),
    LoadExisting,
}

/// What the user did about the accounts.
pub enum AuthAction {
    SaveVpnPassword(Secret),
    SaveAppPassword(Secret),
    LinkGoogleAccount,
    UnlinkGoogleAccount,
    TestGmail,
}

/// What the user did about updates.
pub enum UpdateAction {
    Check,
    StartDownload,
    CancelDownload,
    StartInstall,
}

/// The setting the user changed and its new value.
pub struct SettingChange;

/// A value that is kept only in the secret store of the OS.
pub struct Secret;

/// Why the Helper could not be started or reached, and at which step.
pub struct HelperStartFailure;

/// The user name and the password to connect with.
pub struct VpnAccount;

/// The session that needs an OTP and when the gateway asked for it.
pub struct OtpNeed;

/// An OTP read from the OTP mail.
pub struct Otp;

/// Why the OTP could not be got.
pub struct OtpFailure;

/// Whether the VPN account and the mail account are ready, and what is missing.
pub struct AccountState;

/// How the Gmail connection test ended.
pub struct GmailTestOutcome;

/// What the UI shows about the connection: state, times, tunnel address, stage and reason.
pub struct ConnectionState;

/// All the settings as they are now.
pub struct Settings;

/// What is wrong in the settings file and where.
pub struct SettingsProblem;

/// What was loaded from the settings of an existing VPN client, or why nothing was.
pub struct ExistingSettingsOutcome;

/// The values the app keeps from one run to the next.
pub struct AppState;

/// What the UI shows about updates: version, progress and failure.
pub struct UpdateState;

/// Whether the time is in the schedule, and when the next start is.
pub struct ScheduleState;

/// Whether the user is away from the device.
pub struct IdleState;

/// Whether the device is asleep.
pub struct SleepState;

/// What the user is to be notified of.
pub struct Notice;

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
