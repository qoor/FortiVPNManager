//! Domain sections: everything the menu and the settings window show, as values and kinds. The UI
//! draws them as they are; only sentences, relative times and which pane a button opens are
//! left to it.

use std::net::Ipv4Addr;
use std::time::SystemTime;

use serde::Serialize;

use super::port::{HelperChannelFailure, RegistrationError, SecretStoreFailure};
use super::problem::{
    AccountValueProblem, AddressProblem, ConfigFileError, ConfigWarning, FileProblem,
};
use super::{ButtonState, MinutesSetting, OtpMethod, TimeOfDay, Weekday, millis};
use crate::app_helper::HelperErrorCode;
use crate::backend::{ClientProblem, UntestedVersion};

/// Boxed, as the sections differ much in size.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Section {
    Connection {
        #[serde(rename = "_0")]
        section: Box<ConnectionSection>,
    },
    Schedule {
        #[serde(rename = "_0")]
        section: Box<ScheduleSection>,
    },
    /// Readiness and the Helper.
    Readiness {
        #[serde(rename = "_0")]
        section: Box<ReadinessSection>,
    },
    /// Gmail and the Google account.
    Gmail {
        #[serde(rename = "_0")]
        section: Box<GmailSection>,
    },
    /// The exception rules.
    BypassRules {
        #[serde(rename = "_0")]
        section: Box<BypassRulesSection>,
    },
    /// The settings: the settings file, the Account pane's form and the connection options.
    Config {
        #[serde(rename = "_0")]
        section: Box<ConfigSection>,
    },
}

// MARK: Connection

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSection {
    pub icon: ConnectionIcon,
    pub status: ConnectionStatus,
    /// Why the connection waits, shown under the status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_reason: Option<WaitReason>,
    pub connect: ButtonState,
    pub disconnect: ButtonState,
    pub reconnect_now: ButtonState,
    /// The VPN account in the settings differs from the one the running attempt or session started
    /// with; the change applies from the next connection.
    pub account_applies_next_connection: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionIcon {
    Connected,
    Disconnected,
    /// Connecting or reconnecting.
    InProgress,
    Error,
}

/// Each value appears once; the UI may show it in several places.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConnectionStatus {
    NotConnected {},
    /// Also the line shown with the button that opens Settings.
    SetupRequired {
        items: Vec<SetupItem>,
    },
    Connecting {
        /// Replacing or retrying a connection rather than making a fresh one.
        reconnecting: bool,
        progress: ConnectProgress,
    },
    /// The status line ends with the planned proactive reconnect when there is one; otherwise with
    /// the schedule's outlook when it is `Suspended` or `HoldingWhileAway`. The two never coincide.
    Connected {
        #[serde(serialize_with = "millis::time")]
        since: SystemTime,
        #[serde(skip_serializing_if = "Option::is_none")]
        tunnel_ip: Option<Ipv4Addr>,
        proactive_reconnect: ProactiveReconnect,
    },
    /// Waiting for an away user outside the scheduled hours, until the time the wait ends.
    ReconnectOnHold {
        #[serde(serialize_with = "millis::time")]
        until: SystemTime,
    },
    /// `until` is also the next attempt's time.
    WaitingToReconnect {
        #[serde(serialize_with = "millis::time")]
        until: SystemTime,
    },
    WaitingForNetwork {},
    Disconnecting {
        then: AfterDisconnect,
    },
    /// Also the line shown with the button that opens Settings.
    Failed {
        reason: FailureReason,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectProgress {
    CheckingReadiness,
    ConnectingToGmail,
    ContactingServer,
    LoggingIn,
    WaitingForOtp,
    VerifyingOtp,
    OpeningTunnel,
}

/// The next proactive reconnect of the session.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ProactiveReconnect {
    At {
        #[serde(serialize_with = "millis::time")]
        at: SystemTime,
    },
    PausedBySchedule {},
    Off {},
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AfterDisconnect {
    Idle,
    Reconnect,
    Sleep,
    Quit,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FailureReason {
    /// The server rejected the login before the OTP.
    VpnLoginRejected {},
    OtpNotReceived {
        /// The OTP email was found in the spam folder.
        in_spam: bool,
        /// How long the app looked for it.
        waited_seconds: u64,
    },
    OtpRejected {},
    TunnelSetupFailed {},
    TunnelModeRejected {},
    DroppedEarly {
        /// A session shorter than this counts as dropped early.
        within_seconds: u64,
    },
    CertificateChangedAfterLogin {},
    /// Gmail refused the credential the selected method uses.
    GmailRejected {
        method: OtpMethod,
    },
    TooManyReconnects {
        /// The window in which automatic sign-in attempts are limited.
        window_seconds: u64,
    },
    Network {},
    /// The Helper refused to start the VPN client.
    Helper {
        code: HelperErrorCode,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WaitReason {
    Network {},
    ServerEndedSession {},
    ForcedTermination {},
    /// One authentication failure was counted; one more try follows.
    AuthFailure {
        reason: FailureReason,
    },
    AuthCooldown {
        delay_seconds: u64,
    },
    Sleep {},
    UserAway {},
}

/// Something to set up before connecting, in the order readiness checks it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SetupItem {
    /// The settings file has an error.
    ConfigFile {},
    AccountDetails {},
    VpnPassword {},
    /// No Gmail credential: no app password and, with the Gmail API, no connected Google account.
    GmailCredential {
        method: OtpMethod,
    },
    /// The Helper is not registered and approved.
    HelperApproval {},
    /// The running Helper is another build.
    HelperVersion {},
}

// MARK: Schedule

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSection {
    pub enabled: bool,
    /// Monday first.
    pub days: Vec<Weekday>,
    pub start: TimeOfDay,
    pub end: TimeOfDay,
    pub away_minutes: MinutesSetting,
    /// What the schedule does next; `None` while it is off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outlook: Option<ScheduleOutlook>,
    /// Why the start and end times last sent were not saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours_error: Option<ScheduleHoursError>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ScheduleOutlook {
    /// Not connecting now; the next scheduled start connects.
    Connects {
        #[serde(serialize_with = "millis::time")]
        at: SystemTime,
    },
    /// Not connecting now, and no day is selected.
    NoDays {},
    /// Inside the scheduled hours; after `end`, reconnects wait while the user is away.
    WithinHours {
        #[serde(serialize_with = "millis::time")]
        end: SystemTime,
    },
    OutsideHoursPresent {},
    /// Reconnects wait for the away user and stop at `until`.
    HoldingWhileAway {
        #[serde(serialize_with = "millis::time")]
        until: SystemTime,
    },
    /// Reconnects stopped; the current connection is kept until it ends.
    Suspended {},
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ScheduleHoursError {
    EndNotAfterStart {},
    NotSaved { error: ConfigFileError },
}

// MARK: Readiness and Helper

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessSection {
    pub helper: HelperState,
    pub register_helper: ButtonState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub register_error: Option<RegistrationError>,
    /// What the Helper reports about the VPN client, once it answers with this build, as it
    /// reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<VpnClientInfo>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HelperState {
    Checking {},
    NotRegistered {},
    RequiresApproval {},
    Running {
        build: String,
    },
    VersionMismatch {
        helper_build: String,
        app_build: String,
    },
    /// Registered, but it did not answer.
    Unreachable {
        failure: HelperChannelFailure,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VpnClientInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untested_version: Option<UntestedVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<ClientProblem>,
}

// MARK: Gmail and Google account

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailSection {
    pub method: OtpMethod,
    /// Why the last method change was not saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method_error: Option<ConfigFileError>,
    pub google_account: GoogleAccountStatus,
    pub connect_google: ButtonState,
    pub disconnect_google: ButtonState,
    pub cancel_sign_in: ButtonState,
    /// Why connecting the Google account cannot start now; set only when it is to be shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_blocker: Option<GoogleConnectBlocker>,
    /// The last failure of a sign-in, a disconnect or an OAuth client change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_error: Option<GoogleError>,
    pub oauth_client: OAuthClientSource,
    /// An imported client is set, but its secret is missing from the secret storage.
    pub imported_client_secret_missing: bool,
    pub use_built_in_client: ButtonState,
    pub import_client: ButtonState,
    pub test: GmailTest,
    pub test_button: ButtonState,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GoogleAccountStatus {
    NotConnected {},
    /// Waiting for the sign-in in the browser.
    SigningIn {},
    Disconnecting {},
    Connected {
        address: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleConnectBlocker {
    NoOAuthClient,
    EnterAddress,
    /// The typed Gmail address is not the saved one.
    SaveAddressFirst,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OAuthClientSource {
    BuiltIn { client_id: String },
    Imported { client_id: String },
    NotConfigured {},
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GoogleError {
    ClientFileUnreadable {
        problem: FileProblem,
    },
    ClientFileInvalid {
        problem: OAuthClientFileProblem,
    },
    /// The local listener for Google's answer could not start. The detail is the runtime's English
    /// text, shown as it is by the user's choice; so are the other network details.
    ListenerFailed {
        detail: String,
    },
    BrowserUnavailable {},
    TimedOut {
        after_seconds: u64,
    },
    /// The user declined in the browser.
    CancelledInBrowser {},
    /// Google answered with another error code.
    Denied {
        code: String,
    },
    /// Google refused the authorization code, as Google describes it.
    TokenRejected {
        detail: String,
    },
    ClientRejected {},
    NoRefreshToken {},
    ScopeNotGranted {},
    WrongAccount {
        expected: String,
        actual: String,
    },
    /// The Gmail API refused the account check, with Google's reason.
    GmailApiRefused {
        reason: String,
    },
    Network {
        detail: String,
    },
    SecretStore {
        failure: SecretStoreFailure,
    },
    Config {
        error: ConfigFileError,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OAuthClientFileProblem {
    NotJson {},
    /// A web application client, which cannot use the local listener.
    WebClient {},
    NoDesktopClient {},
    MissingField {
        name: String,
    },
    InvalidField {
        name: String,
    },
}

/// The Gmail connection test of the Account pane.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GmailTest {
    Idle {},
    Running {},
    /// Mailbox names as the IMAP server shows them.
    ImapSucceeded {
        archive: String,
        trash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        spam: Option<String>,
    },
    ApiSucceeded {
        address: String,
    },
    Failed {
        failure: GmailTestFailure,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GmailTestFailure {
    EnterAddress {},
    EnterAppPassword {},
    AppPasswordUnreadable {
        failure: SecretStoreFailure,
    },
    ImapLoginRejected {
        #[serde(skip_serializing_if = "Option::is_none")]
        server_text: Option<String>,
    },
    /// Mailboxes the test needs are not visible over IMAP.
    MailboxesMissing {
        all_mail: bool,
        trash: bool,
    },
    ImapUnreachable {
        failure: ImapFailure,
    },
    NoOAuthClient {},
    NotSignedIn {},
    AccessRevoked {},
    ClientRejected {},
    SecretStore {
        failure: SecretStoreFailure,
    },
    /// The Google authorization belongs to another account.
    OtherAccount {
        authorized: String,
        expected: String,
    },
    AccessTokenRejected {},
    GmailApiRefused {
        reason: String,
    },
    RateLimited {},
    GoogleUnreachable {
        detail: String,
    },
    /// Google answered with an error, such as `HTTP 500`.
    GoogleReturnedError {
        detail: String,
    },
}

/// Why an IMAP session failed. Server texts are carried as they came; the other details are the
/// runtime's English text, shown as it is by the user's choice.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ImapFailure {
    Network {
        detail: String,
    },
    TimedOut {
        step: ImapStep,
    },
    ServerClosed {
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Protocol {
        detail: String,
    },
    /// The server answered a command with `NO` or `BAD`.
    Rejected {
        command: String,
        status: String,
        text: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ImapStep {
    Connecting {},
    Command { name: String },
}

// MARK: Exception rules

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BypassRulesSection {
    /// The primary network as last read; `None` when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_network: Option<CurrentNetwork>,
    pub networks: RuleList,
    pub targets: RuleList,
    /// The one problem to explain under the lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<BypassNotice>,
    pub status: BypassStatus,
    /// The lists as edited differ from the saved ones.
    pub has_changes: bool,
    pub add_current_network: ButtonState,
    pub discard: ButtonState,
    pub apply: ButtonState,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentNetwork {
    pub service_name: String,
    /// This Mac's IPv4 address on it.
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_length: Option<u8>,
    /// The saved network the address is in; `None` outside all of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleList {
    /// As edited.
    pub rows: Vec<String>,
    /// Indexes of rows with a problem.
    pub problem_rows: Vec<u32>,
    /// Adding a row; disabled once the list holds as many entries as the settings file allows.
    pub add_row: ButtonState,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BypassNotice {
    /// `row` as typed.
    RowProblem {
        row: String,
        problem: RowProblem,
    },
    ApplyFailed {
        error: ConfigFileError,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RowProblem {
    Invalid {
        problem: AddressProblem,
    },
    /// The row names `entry`, which an earlier row names too.
    Duplicate {
        entry: String,
    },
    /// The row overlaps the earlier row's `other`.
    Overlap {
        other: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BypassStatus {
    Checking {},
    /// Apply creates the settings file.
    NoConfigFile {},
    /// In effect on this network.
    Applied {
        service_name: String,
        router: String,
        /// Targets on this network itself, which need no route.
        on_link_targets: u32,
        /// Routes added elsewhere that did not fit in the full list.
        not_imported: u32,
        kept_through_other_gateways: u32,
    },
    /// This Mac is outside the listed networks.
    OutsideNetworks {
        kept_through_other_gateways: u32,
    },
    NoNetworksListed {
        kept_through_other_gateways: u32,
    },
    /// The settings file has an error, so the routes are left as they are.
    ConfigInvalid {},
    /// No active network with an IPv4 router.
    NoNetwork {},
    Failed {
        failure: BypassFailure,
        /// Set while a retry is planned.
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_in_seconds: Option<u64>,
        /// Set once the retries ran out: how many there were.
        #[serde(skip_serializing_if = "Option::is_none")]
        stopped_after_retries: Option<u32>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BypassFailure {
    /// The network settings did not keep the routes that were written.
    RoutesNotKept {},
    /// The network or its routes changed while the routes were being written.
    NetworkChanged {},
    /// The Helper refused the route list, with its message.
    HelperRefused {
        message: String,
    },
    /// Changing the network settings failed, with the Helper's message.
    SettingsNotChanged {
        message: String,
    },
    /// Routes found on the network could not be saved to the settings file.
    ImportNotSaved {
        error: ConfigFileError,
    },
    HelperUnreachable {
        failure: HelperChannelFailure,
    },
    /// The Helper is another build, so the routes were not sent.
    HelperBuildMismatch {
        helper_build: String,
        app_build: String,
    },
}

// MARK: Settings

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSection {
    pub file: ConfigFileState,
    pub account: AccountFormState,
    pub options: ConnectionOptions,
    /// Why the last connection or schedule option was not saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options_error: Option<ConfigFileError>,
    pub open_at_login: OpenAtLogin,
    /// Where the log file is.
    pub log_file: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFileState {
    pub path: String,
    pub exists: bool,
    /// Why the file is not in use; the last valid settings stay meanwhile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_error: Option<ConfigFileError>,
    pub warnings: Vec<ConfigWarning>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionOptions {
    pub auto_connect_on_launch: bool,
    pub session_max_minutes: MinutesSetting,
    pub proactive_reconnect: bool,
    pub margin_minutes: MinutesSetting,
    pub reconnect_after_sleep: bool,
    pub min_sleep_minutes: MinutesSetting,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAtLogin {
    /// Only the installed copy can set it.
    pub available: bool,
    pub enabled: bool,
    /// Registered, waiting for the user's approval in the system settings.
    pub requires_approval: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RegistrationError>,
}

/// The Account pane's draft. Passwords never come back; only whether one is typed.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountFormState {
    /// Changes only when the runtime replaces what was typed: on load, save, revert, import,
    /// closing Settings, or a settings file change merged into untouched fields. The UI replaces
    /// its fields only then, and sends the revision back with every edit.
    pub revision: u64,
    pub host: String,
    pub port: String,
    pub username: String,
    pub realm: String,
    pub gmail_address: String,
    pub vpn_password_entered: bool,
    pub app_password_entered: bool,
    /// The secret storage holds a VPN password for the typed username and server.
    pub vpn_password_stored: bool,
    /// The secret storage holds an app password for the typed Gmail address.
    pub app_password_stored: bool,
    /// Why the VPN client would refuse the saved account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_account_problem: Option<AccountValueProblem>,
    /// The last save's result, until the next edit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<AccountFeedback>,
    pub save: ButtonState,
    pub revert: ButtonState,
    /// The last import of the VPN client's settings, until dismissed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_report: Option<ClientSettingsImport>,
    /// The VPN client's settings file at its default location, if there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_import_file: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AccountFeedback {
    Saved {},
    Failed { error: AccountSaveError },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AccountSaveError {
    /// Not a number from 1 to 65535.
    InvalidPort {},
    /// A VPN password needs the server address and username it belongs to.
    PasswordNeedsAccount {},
    /// An app password needs the Gmail address it belongs to.
    AppPasswordNeedsAddress {},
    AccountValue {
        problem: AccountValueProblem,
    },
    /// The VPN client takes no line breaks and at most `max_bytes` bytes.
    InvalidVpnPassword {
        max_bytes: u32,
    },
    Config {
        error: ConfigFileError,
    },
    SecretStore {
        failure: SecretStoreFailure,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ClientSettingsImport {
    Imported { report: ImportReport },
    Failed { error: ImportError },
}

/// What an import did; the source file is never changed.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub source: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub realm: String,
    /// Certificate values now trusted.
    pub trusted_certs: u32,
    /// What became of the password written in the file in plain text; `None` when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<ImportedPassword>,
    /// Keys the VPN client does not know; it stops reading at the first one.
    pub unknown_keys: Vec<String>,
    /// Options the app does not carry over.
    pub ignored_keys: Vec<String>,
    /// 1-based numbers of lines that could not be used.
    pub unreadable_lines: Vec<u32>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ImportedPassword {
    Saved {},
    /// There is no username to keep it under.
    NotSavedWithoutUsername {},
    NotSaved {
        failure: SecretStoreFailure,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ImportError {
    SourceNotFound {
        path: String,
    },
    Unreadable {
        path: String,
        problem: FileProblem,
    },
    MissingHost {
        path: String,
    },
    Config {
        error: ConfigFileError,
    },
    /// FortiClient keeps no SSL VPN connection.
    NoFortiClientConnection {},
    /// The connection's server is not `host` or `host:port`.
    InvalidFortiClientServer {
        connection: String,
    },
}
