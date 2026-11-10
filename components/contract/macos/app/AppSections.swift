import Foundation

extension AppInternal {
    /// Domain sections, drawn as they are (components/contract/src/app_internal/section.rs). Each
    /// value appears once; the UI may show it in several places.
    enum Section: Decodable, Equatable, Sendable {
        case connection(ConnectionSection)
        case schedule(ScheduleSection)
        case readiness(ReadinessSection)
        case gmail(GmailSection)
        case bypassRules(BypassRulesSection)
        case config(ConfigSection)
    }

    // MARK: Connection

    struct ConnectionSection: Decodable, Equatable, Sendable {
        var icon: ConnectionIcon
        var status: ConnectionStatus
        var waitReason: WaitReason?
        var connect: ButtonState
        var disconnect: ButtonState
        var reconnectNow: ButtonState
        /// The account change applies from the next connection.
        var accountAppliesNextConnection: Bool
    }

    enum ConnectionIcon: String, Decodable, Sendable {
        case connected
        case disconnected
        case inProgress
        case error
    }

    enum ConnectionStatus: Decodable, Equatable, Sendable {
        case notConnected
        /// Also the line shown with the button that opens Settings.
        case setupRequired(items: [SetupItem])
        case connecting(reconnecting: Bool, progress: ConnectProgress)
        /// The status line ends with the planned proactive reconnect, or else with the schedule
        /// outlook when it is `suspended` or `holdingWhileAway`.
        case connected(since: Date, tunnelIp: String?, proactiveReconnect: ProactiveReconnect)
        case reconnectOnHold(until: Date)
        /// `until` is also the next attempt's time.
        case waitingToReconnect(until: Date)
        case waitingForNetwork
        case disconnecting(then: AfterDisconnect)
        /// Also the line shown with the button that opens Settings.
        case failed(reason: FailureReason)
    }

    enum ConnectProgress: String, Decodable, Sendable {
        case checkingReadiness
        case connectingToGmail
        case contactingServer
        case loggingIn
        case waitingForOtp
        case verifyingOtp
        case openingTunnel
    }

    enum ProactiveReconnect: Decodable, Equatable, Sendable {
        case at(at: Date)
        case pausedBySchedule
        case off
    }

    enum AfterDisconnect: String, Decodable, Sendable {
        case idle
        case reconnect
        case sleep
        case quit
    }

    enum FailureReason: Decodable, Equatable, Sendable {
        case vpnLoginRejected
        case otpNotReceived(inSpam: Bool, waitedSeconds: Int)
        case otpRejected
        case tunnelSetupFailed
        case tunnelModeRejected
        case droppedEarly(withinSeconds: Int)
        case certificateChangedAfterLogin
        case gmailRejected(method: OtpMethod)
        case tooManyReconnects(windowSeconds: Int)
        case network
        case helper(code: HelperErrorCode)
    }

    /// The Helper's error codes (components/contract/src/app_helper.rs).
    enum HelperErrorCode: String, Decodable, Sendable {
        case busy
        case invalidRequest
        case unknownAttempt
        case wrongPhase
        case clientUnavailable = "binaryUnavailable"
        case notAuthorized
        case internalError
    }

    enum WaitReason: Decodable, Equatable, Sendable {
        case network
        case serverEndedSession
        case forcedTermination
        case authFailure(reason: FailureReason)
        case authCooldown(delaySeconds: Int)
        case sleep
        case userAway
    }

    enum SetupItem: Decodable, Equatable, Sendable {
        case configFile
        case accountDetails
        case vpnPassword
        case gmailCredential(method: OtpMethod)
        case helperApproval
        case helperVersion
    }

    // MARK: Schedule

    struct ScheduleSection: Decodable, Equatable, Sendable {
        var enabled: Bool
        /// Monday first.
        var days: [Weekday]
        var start: TimeOfDay
        var end: TimeOfDay
        var awayMinutes: MinutesSetting
        /// nil while the schedule is off.
        var outlook: ScheduleOutlook?
        var hoursError: ScheduleHoursError?
    }

    enum ScheduleOutlook: Decodable, Equatable, Sendable {
        case connects(at: Date)
        case noDays
        case withinHours(end: Date)
        case outsideHoursPresent
        case holdingWhileAway(until: Date)
        case suspended
    }

    enum ScheduleHoursError: Decodable, Equatable, Sendable {
        case endNotAfterStart
        case notSaved(error: ConfigFileError)
    }

    // MARK: Readiness and Helper

    struct ReadinessSection: Decodable, Equatable, Sendable {
        var helper: HelperState
        var registerHelper: ButtonState
        var registerError: RegistrationError?
        var client: VpnClientInfo?
    }

    enum HelperState: Decodable, Equatable, Sendable {
        case checking
        case notRegistered
        case requiresApproval
        case running(build: String)
        case versionMismatch(helperBuild: String, appBuild: String)
        case unreachable(failure: HelperChannelFailure)
    }

    struct VpnClientInfo: Decodable, Equatable, Sendable {
        var path: String?
        var version: String?
        var untestedVersion: UntestedVersion?
        var problem: ClientProblem?
    }

    struct UntestedVersion: Decodable, Equatable, Sendable {
        var testedSeries: String
    }

    /// components/contract/src/backend.rs
    struct ClientProblem: Decodable, Equatable, Sendable {
        var kind: ClientProblemKind
        var path: String
    }

    enum ClientProblemKind: Decodable, Equatable, Sendable {
        case notFound
        case outsideAllowedLocation
        case notRegularFile
        case writableByOthers(mode: UInt32)
        case notExecutable
    }

    // MARK: Gmail and Google account

    struct GmailSection: Decodable, Equatable, Sendable {
        var method: OtpMethod
        var methodError: ConfigFileError?
        var googleAccount: GoogleAccountStatus
        var connectGoogle: ButtonState
        var disconnectGoogle: ButtonState
        var cancelSignIn: ButtonState
        var connectBlocker: GoogleConnectBlocker?
        var googleError: GoogleError?
        var oauthClient: OAuthClientSource
        var importedClientSecretMissing: Bool
        var useBuiltInClient: ButtonState
        var importClient: ButtonState
        var test: GmailTest
        var testButton: ButtonState
    }

    enum GoogleAccountStatus: Decodable, Equatable, Sendable {
        case notConnected
        case signingIn
        case disconnecting
        case connected(address: String)
    }

    enum GoogleConnectBlocker: String, Decodable, Sendable {
        case noOAuthClient
        case enterAddress
        case saveAddressFirst
    }

    enum OAuthClientSource: Decodable, Equatable, Sendable {
        case builtIn(clientId: String)
        case imported(clientId: String)
        case notConfigured
    }

    enum GoogleError: Decodable, Equatable, Sendable {
        case clientFileUnreadable(problem: FileProblem)
        case clientFileInvalid(problem: OAuthClientFileProblem)
        /// The details of this and the other network failures are Rust's English text, shown as
        /// they are.
        case listenerFailed(detail: String)
        case browserUnavailable
        case timedOut(afterSeconds: Int)
        case cancelledInBrowser
        case denied(code: String)
        case tokenRejected(detail: String)
        case clientRejected
        case noRefreshToken
        case scopeNotGranted
        case wrongAccount(expected: String, actual: String)
        case gmailApiRefused(reason: String)
        case network(detail: String)
        case secretStore(failure: SecretStoreFailure)
        case config(error: ConfigFileError)
    }

    enum OAuthClientFileProblem: Decodable, Equatable, Sendable {
        case notJson
        case webClient
        case noDesktopClient
        case missingField(name: String)
        case invalidField(name: String)
    }

    enum GmailTest: Decodable, Equatable, Sendable {
        case idle
        case running
        case imapSucceeded(archive: String, trash: String, spam: String?)
        case apiSucceeded(address: String)
        case failed(failure: GmailTestFailure)
    }

    enum GmailTestFailure: Decodable, Equatable, Sendable {
        case enterAddress
        case enterAppPassword
        case appPasswordUnreadable(failure: SecretStoreFailure)
        case imapLoginRejected(serverText: String?)
        case mailboxesMissing(allMail: Bool, trash: Bool)
        case imapUnreachable(failure: ImapFailure)
        case noOAuthClient
        case notSignedIn
        case accessRevoked
        case clientRejected
        case secretStore(failure: SecretStoreFailure)
        case otherAccount(authorized: String, expected: String)
        case accessTokenRejected
        case gmailApiRefused(reason: String)
        case rateLimited
        case googleUnreachable(detail: String)
        case googleReturnedError(detail: String)
    }

    enum ImapFailure: Decodable, Equatable, Sendable {
        case network(detail: String)
        case timedOut(step: ImapStep)
        case serverClosed(text: String?)
        case `protocol`(detail: String)
        case rejected(command: String, status: String, text: String)
    }

    enum ImapStep: Decodable, Equatable, Sendable {
        case connecting
        case command(name: String)
    }

    // MARK: Exception rules

    struct BypassRulesSection: Decodable, Equatable, Sendable {
        var currentNetwork: CurrentNetwork?
        var networks: RuleList
        var targets: RuleList
        var notice: BypassNotice?
        var status: BypassStatus
        var hasChanges: Bool
        var addCurrentNetwork: ButtonState
        var discard: ButtonState
        var apply: ButtonState
    }

    struct CurrentNetwork: Decodable, Equatable, Sendable {
        var serviceName: String
        var address: String
        var prefixLength: Int?
        /// The saved network the address is in; nil outside all of them.
        var within: String?
    }

    struct RuleList: Decodable, Equatable, Sendable {
        var rows: [String]
        var problemRows: [Int]
        var addRow: ButtonState
    }

    enum BypassNotice: Decodable, Equatable, Sendable {
        case rowProblem(row: String, problem: RowProblem)
        case applyFailed(error: ConfigFileError)
    }

    enum RowProblem: Decodable, Equatable, Sendable {
        case invalid(problem: AddressProblem)
        case duplicate(entry: String)
        case overlap(other: String)
    }

    enum BypassStatus: Decodable, Equatable, Sendable {
        case checking
        case noConfigFile
        case applied(serviceName: String, router: String, onLinkTargets: Int, notImported: Int, keptThroughOtherGateways: Int)
        case outsideNetworks(keptThroughOtherGateways: Int)
        case noNetworksListed(keptThroughOtherGateways: Int)
        case configInvalid
        case noNetwork
        case failed(failure: BypassFailure, retryInSeconds: Int?, stoppedAfterRetries: Int?)
    }

    enum BypassFailure: Decodable, Equatable, Sendable {
        case routesNotKept
        case networkChanged
        case helperRefused(message: String)
        case settingsNotChanged(message: String)
        case importNotSaved(error: ConfigFileError)
        case helperUnreachable(failure: HelperChannelFailure)
        case helperBuildMismatch(helperBuild: String, appBuild: String)
    }

    // MARK: Settings

    struct ConfigSection: Decodable, Equatable, Sendable {
        var file: ConfigFileState
        var account: AccountFormState
        var options: ConnectionOptions
        var optionsError: ConfigFileError?
        var openAtLogin: OpenAtLogin
        var logFile: String
    }

    struct ConfigFileState: Decodable, Equatable, Sendable {
        var path: String
        var exists: Bool
        var loadError: ConfigFileError?
        var warnings: [ConfigWarning]
    }

    struct ConnectionOptions: Decodable, Equatable, Sendable {
        var autoConnectOnLaunch: Bool
        var sessionMaxMinutes: MinutesSetting
        var proactiveReconnect: Bool
        var marginMinutes: MinutesSetting
        var reconnectAfterSleep: Bool
        var minSleepMinutes: MinutesSetting
    }

    struct OpenAtLogin: Decodable, Equatable, Sendable {
        var available: Bool
        var enabled: Bool
        var requiresApproval: Bool
        var error: RegistrationError?
    }

    /// The Account pane's draft. Replace the typed fields only when `revision` changes.
    struct AccountFormState: Decodable, Equatable, Sendable {
        var revision: UInt64
        var host: String
        var port: String
        var username: String
        var realm: String
        var gmailAddress: String
        var vpnPasswordEntered: Bool
        var appPasswordEntered: Bool
        var vpnPasswordStored: Bool
        var appPasswordStored: Bool
        var savedAccountProblem: AccountValueProblem?
        var feedback: AccountFeedback?
        var save: ButtonState
        var revert: ButtonState
        var importReport: ClientSettingsImport?
        var defaultImportFile: String?
    }

    enum AccountFeedback: Decodable, Equatable, Sendable {
        case saved
        case failed(error: AccountSaveError)
    }

    enum AccountSaveError: Decodable, Equatable, Sendable {
        case invalidPort
        case passwordNeedsAccount
        case appPasswordNeedsAddress
        case accountValue(problem: AccountValueProblem)
        case invalidVpnPassword(maxBytes: Int)
        case config(error: ConfigFileError)
        case secretStore(failure: SecretStoreFailure)
    }

    enum ClientSettingsImport: Decodable, Equatable, Sendable {
        case imported(report: ImportReport)
        case failed(error: ImportError)
    }

    struct ImportReport: Decodable, Equatable, Sendable {
        var source: String
        var host: String
        var port: Int
        var username: String
        var realm: String
        var trustedCerts: Int
        /// nil when the file has no password.
        var password: ImportedPassword?
        var unknownKeys: [String]
        var ignoredKeys: [String]
        var unreadableLines: [Int]
    }

    enum ImportedPassword: Decodable, Equatable, Sendable {
        case saved
        case notSavedWithoutUsername
        case notSaved(failure: SecretStoreFailure)
    }

    enum ImportError: Decodable, Equatable, Sendable {
        case sourceNotFound(path: String)
        case unreadable(path: String, problem: FileProblem)
        case missingHost(path: String)
        case config(error: ConfigFileError)
    }
}
