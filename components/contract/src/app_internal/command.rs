//! User actions, sent by the UI. Actions that stay inside the UI send nothing: clearing or copying
//! the Log tab, revealing a file in Finder, choosing a file, opening the settings window. Quitting
//! goes through the app's normal termination, which the OS integration reports as
//! `SystemEvent::QuitRequested`.

use serde::Deserialize;

use super::{OtpMethod, SecretText, TimeOfDay, Weekday};

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Command {
    Connect {},
    Disconnect {},
    ReconnectNow {},
    /// The settings window opened or came forward.
    SettingsOpened {},
    /// The settings window closed: passwords typed in the Account tab are forgotten.
    SettingsClosed {},

    /// The Account tab's fields as typed, after any edit. The draft lives in the runtime, which
    /// decides what changed and what Save writes. `revision` is the form's revision the edit was
    /// typed against; an edit against an older one is dropped, since the runtime has replaced
    /// those fields since.
    EditAccount {
        revision: u64,
        form: AccountForm,
    },
    SaveAccount {},
    RevertAccount {},
    /// Imports the VPN client's own settings file; `None` is its default location.
    ImportClientSettings {
        file: Option<String>,
    },
    DismissImportReport {},
    SetOtpMethod {
        method: OtpMethod,
    },
    /// Tests Gmail with the selected method and what the Account tab holds.
    TestGmail {},
    ConnectGoogleAccount {},
    CancelGoogleSignIn {},
    DisconnectGoogleAccount {},
    /// Imports a Google Cloud "Desktop app" OAuth client JSON.
    ImportOAuthClient {
        file: String,
    },
    UseBuiltInOAuthClient {},

    /// Saves one connection or schedule option to the settings file at once.
    SetConnectionOption {
        option: ConnectionOption,
    },
    /// Sent on every change of either time picker; saved only while the end is after the start.
    SetScheduleHours {
        start: TimeOfDay,
        end: TimeOfDay,
    },
    SetOpenAtLogin {
        enabled: bool,
    },
    RegisterHelper {},
    /// Opens the system settings where the Helper is approved, then follows the approval.
    OpenHelperApprovalSettings {},

    /// The rows of one exception rules list as edited, blank rows included.
    SetBypassRows {
        list: BypassList,
        rows: Vec<String>,
    },
    AddCurrentNetwork {},
    DiscardBypassChanges {},
    ApplyBypassRules {},
}

/// The Account tab as typed. An empty password field keeps the stored password.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountForm {
    pub host: String,
    pub port: String,
    pub username: String,
    pub realm: String,
    pub gmail_address: String,
    pub vpn_password: SecretText,
    pub app_password: SecretText,
}

/// Minutes are the number as typed; the settings file's rules say whether it is accepted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConnectionOption {
    ScheduleEnabled { enabled: bool },
    ScheduleDay { day: Weekday, selected: bool },
    AwayMinutes { minutes: i64 },
    AutoConnectOnLaunch { enabled: bool },
    SessionMaxMinutes { minutes: i64 },
    ProactiveReconnect { enabled: bool },
    ProactiveMarginMinutes { minutes: i64 },
    ReconnectAfterSleep { enabled: bool },
    MinSleepMinutes { minutes: i64 },
}

/// The two exception rules lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BypassList {
    /// Where the rules apply (`routes.networks`).
    Networks,
    /// What skips the VPN there (`routes.bypass_vpn`).
    Targets,
}
