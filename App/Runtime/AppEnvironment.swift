import AppKit
import FortiCore
import Observation
import ServiceManagement

/// Creates and wires the app's services (DESIGN §4). The UI reads the stores and controllers exposed
/// here. The OTP session is an `OTPSessionRouter` over Gmail IMAP and the Gmail API (§5.3).
@Observable
final class AppEnvironment {
    let configStore: ConfigStore
    let stateStore: StateStore
    let credentials: CredentialStore
    let helperInstaller: HelperInstaller
    let helperClient: HelperClient
    let readinessChecker: ReadinessChecker
    let systemMonitor: SystemMonitor
    let logStore: LogStore
    let loginItems: LoginItemService
    let importer: FirstRunImporter
    let connection: ConnectionController
    let otpSession: any OTPSessionService
    let gmailTester: any GmailConnectionTesting
    /// Google OAuth for `otp.method: gmail_api`: client JSON import, browser sign-in, tokens.
    let googleOAuth: GoogleOAuthService
    /// "Test Gmail API" (profile check with the stored authorization).
    let gmailAPITester: any GmailAPIConnectionTesting

    /// Outcome of the automatic import on the very first launch (`nil` when nothing was imported).
    /// The UI opens the Account tab to show it.
    private(set) var firstLaunchImport: Result<OpenFortiVPNImportReport, OpenFortiVPNImportError>?

    @ObservationIgnored private var started = false
    @ObservationIgnored private var activationObserver: NSObjectProtocol?

    init() {
        let configStore = ConfigStore()
        let credentials = CredentialStore()
        let googleOAuth = GoogleOAuthService(configStore: configStore, credentials: credentials)
        self.configStore = configStore
        self.credentials = credentials
        self.googleOAuth = googleOAuth
        otpSession = OTPSessionRouter(
            imap: GmailOTPSession(),
            api: GmailAPIOTPSession(tokens: googleOAuth),
            availability: { [googleOAuth] address in googleOAuth.availability(address: address) }
        )
        gmailTester = GmailConnectionTester()
        gmailAPITester = GmailAPIConnectionTester(oauth: googleOAuth)
        stateStore = StateStore()
        helperInstaller = HelperInstaller()
        helperClient = HelperClient()
        readinessChecker = ReadinessChecker(
            configStore: configStore, credentials: credentials, installer: helperInstaller, helper: helperClient
        )
        systemMonitor = SystemMonitor()
        logStore = LogStore()
        loginItems = LoginItemService(installer: helperInstaller)
        importer = FirstRunImporter(configStore: configStore, credentials: credentials)
        connection = ConnectionController(
            configStore: configStore,
            stateStore: stateStore,
            credentials: credentials,
            helper: helperClient,
            readinessChecker: readinessChecker,
            otpSession: otpSession,
            logStore: logStore
        )
    }

    /// First-run import → load config → login item → monitors → `appLaunched`. Call once at launch.
    func start() {
        guard !started else { return }
        started = true

        firstLaunchImport = importer.importOnFirstLaunch()
        configStore.start()
        loginItems.registerOnFirstInstalledLaunch(state: stateStore)

        // Keychain changes raise no config notification.
        googleOAuth.onAuthorizationChanged = { [weak self] in
            self?.connection.recheckReadiness()
        }
        configStore.onChange = { [weak self] change in
            self?.systemMonitor.setGatewayHost(change.new.vpn.host)
            self?.connection.configDidChange(change)
        }
        systemMonitor.setGatewayHost(configStore.config.vpn.host)
        systemMonitor.onEvent = { [weak self] event in
            self?.connection.handleSystemEvent(event)
        }
        systemMonitor.processAlive = { [weak self] in
            guard let self, connection.model.activeAttemptID != nil else { return false }
            return (try? await helperClient.status().childAlive) ?? false
        }

        connection.launch(
            autoConnect: configStore.config.connection.autoConnectOnLaunch,
            restoredLedger: stateStore.ledger
        )
        systemMonitor.start()

        activationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { @Sendable [weak self] _ in
            MainActor.assumeIsolated { self?.applicationDidBecomeActive() }
        }
    }

    /// Re-reads config.yaml and re-checks readiness and the login item (§5.4, §5.7).
    func applicationDidBecomeActive() {
        configStore.poll()
        loginItems.refresh()
        connection.recheckReadiness()
    }

    /// Forward from `NSApplicationDelegate.applicationShouldTerminate(_:)`.
    func applicationShouldTerminate() -> NSApplication.TerminateReply {
        connection.applicationShouldTerminate()
    }

    /// "Register Helper" (only from /Applications), then watch for the approval in System Settings.
    func registerHelper() throws {
        try helperInstaller.register()
        connection.recheckReadinessWhileSystemSettingsOpen()
    }

    /// "Open System Settings" (Login Items & Extensions), then re-check readiness every 2 s for 2 minutes.
    func openHelperSystemSettings() {
        SMAppService.openSystemSettingsLoginItems()
        connection.recheckReadinessWhileSystemSettingsOpen()
    }
}
