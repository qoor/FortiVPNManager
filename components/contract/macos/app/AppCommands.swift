extension AppInternal {
    /// User actions (components/contract/src/app_internal/command.rs).
    enum Command: Encodable, Sendable {
        case connect
        case disconnect
        case reconnectNow
        case settingsOpened
        /// Passwords typed in the Account pane are forgotten.
        case settingsClosed

        /// The Account pane's fields as typed, after any edit, with the form's `revision` they were
        /// typed against; Rust drops an edit against an older revision.
        case editAccount(revision: UInt64, form: AccountForm)
        case saveAccount
        case revertAccount
        /// The VPN client's own settings file; nil is its default location.
        case importClientSettings(file: String?)
        case dismissImportReport
        case setOtpMethod(method: OtpMethod)
        case testGmail
        case connectGoogleAccount
        case cancelGoogleSignIn
        case disconnectGoogleAccount
        case importOAuthClient(file: String)
        case useBuiltInOAuthClient

        case setConnectionOption(option: ConnectionOption)
        /// On every change of either time picker.
        case setScheduleHours(start: TimeOfDay, end: TimeOfDay)
        case setOpenAtLogin(enabled: Bool)
        case registerHelper
        case openHelperApprovalSettings

        case setBypassRows(list: BypassList, rows: [String])
        case addCurrentNetwork
        case discardBypassChanges
        case applyBypassRules
    }

    /// An empty password field keeps the stored password.
    struct AccountForm: Encodable, Sendable {
        var host: String
        var port: String
        var username: String
        var realm: String
        var gmailAddress: String
        var vpnPassword: String
        var appPassword: String
    }

    /// Minutes are the number as typed.
    enum ConnectionOption: Encodable, Sendable {
        case scheduleEnabled(enabled: Bool)
        case scheduleDay(day: Weekday, selected: Bool)
        case awayMinutes(minutes: Int)
        case autoConnectOnLaunch(enabled: Bool)
        case sessionMaxMinutes(minutes: Int)
        case proactiveReconnect(enabled: Bool)
        case proactiveMarginMinutes(minutes: Int)
        case reconnectAfterSleep(enabled: Bool)
        case minSleepMinutes(minutes: Int)
    }

    enum BypassList: String, Encodable, Sendable {
        case networks
        case targets
    }
}
