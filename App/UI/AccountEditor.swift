import AppKit
import FortiConfig
import FortiCore
import FortiShared
import Observation
import os
import UniformTypeIdentifiers

/// Edit buffer behind the Account tab (DESIGN §5.6 tab 1). Fields start from config.yaml; "Save"
/// writes config.yaml (`ConfigStore.update`) and the two passwords (Keychain). Passwords are never
/// read back into the form: an empty password field means "keep the stored one".
///
/// The OTP method and the Google account controls act immediately instead of waiting for "Save":
/// the method is written to config.yaml when picked, and the OAuth client import, Google sign-in
/// and disconnect go through `GoogleOAuthService`. Sign-in uses the saved Gmail address, because
/// sessions look the Google authorization up by that address.
@Observable
final class AccountEditor {
    /// The non-secret fields, as typed.
    struct Fields: Equatable {
        var host: String
        var port: String
        var username: String
        var realm: String
        var gmailAddress: String

        init(_ config: AppConfig) {
            host = config.vpn.host
            port = String(config.vpn.port)
            username = config.vpn.username
            realm = config.vpn.realm
            gmailAddress = config.otp.gmailAddress
        }
    }

    enum Feedback: Equatable {
        case saved
        case error(String)
    }

    enum GmailTestState: Equatable {
        case idle
        case running
        case succeeded(String)
        case failed(String)
    }

    var fields: Fields
    var vpnPassword = ""
    var appPassword = ""
    /// Result of the last save; cleared by the next edit.
    private(set) var feedback: Feedback?
    private(set) var gmailTest: GmailTestState = .idle
    /// Last import (the automatic first-launch one, or the button's).
    private(set) var importOutcome: Result<OpenFortiVPNImportReport, OpenFortiVPNImportError>?
    /// Whether the Keychain holds a password for the typed `username@host` / Gmail address.
    private(set) var vpnPasswordStored = false
    private(set) var appPasswordStored = false
    /// Why the helper would refuse the saved host/username/realm (e.g. after a hand edit or an import).
    private(set) var storedAccountProblem: String?

    /// Why writing `otp.method` failed (e.g. the config file is broken).
    private(set) var methodError: String?
    /// Last OAuth client import, Google sign-in or disconnect failure.
    private(set) var googleError: String?
    /// The imported OAuth client's secret is in the Keychain.
    private(set) var googleClientStored = false
    /// The saved Gmail address has a Google authorization (client secret + refresh token).
    private(set) var googleAuthorized = false
    /// "Disconnect" is deleting and revoking the authorization.
    private(set) var isDisconnectingGoogle = false
    /// "Import JSON…" is looking for the newest client file or showing the open panel.
    private(set) var isChoosingGoogleClient = false

    /// The config values `fields` were loaded from; edits are changes relative to this.
    @ObservationIgnored private var base: Fields
    @ObservationIgnored private let environment: AppEnvironment
    @ObservationIgnored private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "ui")

    init(environment: AppEnvironment) {
        self.environment = environment
        let loaded = Fields(environment.configStore.config)
        fields = loaded
        base = loaded
        storedAccountProblem = Self.accountProblem(host: loaded.host, username: loaded.username, realm: loaded.realm)
    }

    var hasChanges: Bool {
        fields != base || !vpnPassword.isEmpty || !appPassword.isEmpty
    }

    /// Keychain lookup key for the typed account (drives `refreshStoredSecrets`).
    var secretsKey: String {
        "\(trimmed(fields.username))@\(trimmed(fields.host)) \(trimmed(fields.gmailAddress))"
    }

    /// The user started editing again: the last save's result no longer applies.
    func editingChanged() {
        if hasChanges { feedback = nil }
    }

    /// Picks up config.yaml changes (window opened, file changed). Fields the user is editing keep
    /// their draft; untouched fields follow the file, so a later save never reverts a hand edit.
    func reloadIfUnchanged() {
        // A failed `otp.method` write is stale once the file changed (e.g. it was fixed by hand).
        methodError = nil
        guard fields != base else {
            reload()
            return
        }
        let latest = Fields(environment.configStore.config)
        if fields.host == base.host { fields.host = latest.host }
        if fields.port == base.port { fields.port = latest.port }
        if fields.username == base.username { fields.username = latest.username }
        if fields.realm == base.realm { fields.realm = latest.realm }
        if fields.gmailAddress == base.gmailAddress { fields.gmailAddress = latest.gmailAddress }
        base = latest
        storedAccountProblem = Self.accountProblem(host: latest.host, username: latest.username, realm: latest.realm)
        refreshStoredSecrets()
    }

    /// The settings window closed: forget typed secrets so they don't linger in memory.
    func windowClosed() {
        vpnPassword = ""
        appPassword = ""
        reloadIfUnchanged()
    }

    /// "Revert": drops every edit.
    func revert() {
        vpnPassword = ""
        appPassword = ""
        feedback = nil
        reload()
    }

    /// Looks up whether the Keychain items exist, without reading them (no access prompt).
    func refreshStoredSecrets() {
        refreshGoogleState()
        let credentials = environment.credentials
        let host = trimmed(fields.host)
        let username = trimmed(fields.username)
        let address = trimmed(fields.gmailAddress)
        do {
            vpnPasswordStored = try !host.isEmpty && !username.isEmpty
                && credentials.hasVPNPassword(username: username, host: host)
            appPasswordStored = try !address.isEmpty && credentials.hasGmailAppPassword(address: address)
        } catch {
            logger.error("keychain lookup failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    // MARK: - Save

    /// Writes config.yaml first, then the passwords under the (possibly new) `username@host` and address.
    func save() {
        feedback = nil
        guard let port = Int(trimmed(fields.port)), ConfigLimits.port.contains(port) else {
            feedback = .error(String(localized: "The port must be a number from 1 to 65535."))
            return
        }
        let host = trimmed(fields.host)
        let username = trimmed(fields.username)
        let realm = trimmed(fields.realm)
        let address = trimmed(fields.gmailAddress)
        if !vpnPassword.isEmpty, host.isEmpty || username.isEmpty {
            feedback = .error(String(localized: "Enter the server address and username to save the VPN password."))
            return
        }
        if !appPassword.isEmpty, address.isEmpty {
            feedback = .error(String(localized: "Enter the Gmail address to save the app password."))
            return
        }
        if let problem = Self.accountProblem(host: host, username: username, realm: realm) {
            feedback = .error(problem)
            return
        }
        if !vpnPassword.isEmpty, (try? ConfigRenderer.validatePassword(vpnPassword)) == nil {
            feedback = .error(String(localized: "The VPN password can't contain line breaks and must be at most 255 bytes."))
            return
        }

        // Only the fields the user changed are written; everything else keeps the value on disk
        // (which may have been hand-edited since the form was loaded).
        let edited = fields
        let original = base
        let saved: AppConfig
        do throws(ConfigStoreError) {
            saved = try environment.configStore.update { config in
                if edited.host != original.host { config.vpn.host = host }
                if edited.port != original.port { config.vpn.port = port }
                if edited.username != original.username { config.vpn.username = username }
                if edited.realm != original.realm { config.vpn.realm = realm }
                if edited.gmailAddress != original.gmailAddress { config.otp.gmailAddress = address }
            }
        } catch {
            feedback = .error(error.message)
            return
        }

        // Keychain keys come from what was actually saved, not from the draft.
        do {
            if !vpnPassword.isEmpty {
                try environment.credentials.setVPNPassword(vpnPassword, username: saved.vpn.username, host: saved.vpn.host)
                vpnPassword = ""
            }
            if !appPassword.isEmpty {
                try environment.credentials.setGmailAppPassword(appPassword, address: saved.otp.gmailAddress)
                appPassword = ""
            }
            feedback = .saved
        } catch {
            feedback = .error(String(localized: "Couldn't save the password to Keychain: \(error.localizedDescription)"))
        }
        reload()
        // Keychain changes raise no config notification.
        environment.connection.recheckReadiness()
    }

    /// The first host/username/realm problem the helper would refuse at connect time
    /// (`ConfigRenderer` rules), localized. Empty host or username is left to readiness ("Account details").
    private static func accountProblem(host: String, username: String, realm: String) -> String? {
        do throws(ConfigRenderer.ValidationError) {
            if !host.isEmpty { try ConfigRenderer.validateHost(host) }
            if !username.isEmpty { try ConfigRenderer.validateUsername(username) }
            try ConfigRenderer.validateRealm(realm)
            return nil
        } catch {
            let name = switch error.field {
            case .host: String(localized: "Server address")
            case .username: String(localized: "Username")
            default: error.field.rawValue
            }
            return switch error.problem {
            case .invalidHost:
                String(localized: "Enter only a host name or IPv4 address as the server address (no https://, :port, or /). Enter the port in the Port field.")
            case let .tooLong(maxBytes):
                String(localized: "\(name) is too long (at most \(maxBytes) bytes).")
            case .leadingOrTrailingWhitespace:
                String(localized: "\(name) can't start or end with a space.")
            default:
                String(localized: "\(name) contains characters that aren't allowed.")
            }
        }
    }

    // MARK: - Gmail test

    /// "Test Gmail Connection" for the configured method: IMAP with the typed address and the typed
    /// (or else the stored) app password, or the Gmail API with the typed address's Google authorization.
    func testGmail() async {
        guard gmailTest != .running else { return }
        let method = environment.configStore.otpMethod
        let outcome: GmailTestState
        switch method {
        case .imap: outcome = await testIMAP()
        case .gmailAPI: outcome = await testGmailAPI()
        }
        // A result for the method that is no longer selected would be misleading.
        gmailTest = environment.configStore.otpMethod == method ? outcome : .idle
    }

    private func testIMAP() async -> GmailTestState {
        let address = trimmed(fields.gmailAddress)
        guard !address.isEmpty else {
            return .failed(String(localized: "Enter the Gmail address."))
        }
        let account: GmailAccount
        if !appPassword.isEmpty {
            account = GmailAccount(address: address, appPassword: CredentialStore.normalizedAppPassword(appPassword))
        } else {
            do {
                guard let stored = try environment.credentials.gmailAccount(address: address) else {
                    return .failed(String(localized: "Enter the app password."))
                }
                account = stored
            } catch {
                return .failed(String(localized: "Couldn't read the app password from Keychain: \(error.localizedDescription)"))
            }
        }
        gmailTest = .running
        let result = await environment.gmailTester.test(account: account)
        return switch result {
        case let .success(allMail, trash, junk):
            .succeeded(
                junk.map { String(localized: "Connection succeeded · Archive: \(allMail), Trash: \(trash), Spam: \($0)") }
                    ?? String(localized: "Connection succeeded · Archive: \(allMail), Trash: \(trash)")
            )
        case let .authenticationFailed(message), let .mailboxesMissing(message), let .networkError(message):
            .failed(message)
        }
    }

    private func testGmailAPI() async -> GmailTestState {
        let address = trimmed(fields.gmailAddress)
        guard !address.isEmpty else {
            return .failed(String(localized: "Enter the Gmail address."))
        }
        gmailTest = .running
        let result = await environment.gmailAPITester.test(address: address)
        return switch result {
        case let .success(address):
            .succeeded(String(localized: "Connection succeeded · Gmail API (\(address))"))
        case let .notAuthorized(message), let .authenticationFailed(message), let .networkError(message):
            .failed(message)
        }
    }

    /// A previous result no longer describes the typed account.
    func resetGmailTest() {
        if gmailTest != .running { gmailTest = .idle }
    }

    // MARK: - OTP method and Google account

    /// The Gmail address in config.yaml. Sign-in and disconnect use it rather than the draft.
    var savedGmailAddress: String { environment.configStore.config.otp.gmailAddress }

    /// Why "Connect Google Account" can't run now (shown under the row), or `nil`.
    var googleConnectBlocker: String? {
        guard googleClientStored else { return String(localized: "Import the OAuth client JSON first.") }
        let typed = trimmed(fields.gmailAddress)
        guard !typed.isEmpty else { return String(localized: "Enter the Gmail address.") }
        guard typed == savedGmailAddress else { return String(localized: "Save the Gmail address first.") }
        return nil
    }

    /// Picker "Connection method": writes `otp.method` to config.yaml right away.
    func setOTPMethod(_ method: AppConfig.OTP.Method) {
        guard method != environment.configStore.otpMethod else { return }
        do throws(ConfigStoreError) {
            try environment.configStore.setOTPMethod(method)
            methodError = nil
        } catch {
            methodError = error.message
        }
        googleError = nil
        gmailTest = .idle
    }

    /// Looks up the Google client and authorization state (Keychain existence checks only).
    func refreshGoogleState() {
        let oauth = environment.googleOAuth
        googleClientStored = oauth.hasClientSecret
        let address = savedGmailAddress
        googleAuthorized = !address.isEmpty && oauth.isAuthorized(address: address)
    }

    /// "Import JSON…": an open panel in ~/Downloads with the newest `client_secret_*.json` selected.
    /// The service reads the file; this code only lists the folder.
    func importGoogleClient() async {
        guard !isChoosingGoogleClient else { return }
        isChoosingGoogleClient = true
        defer { isChoosingGoogleClient = false }
        googleError = nil
        let downloads = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
        var newest: URL?
        if let downloads {
            newest = await Self.newestGoogleClientFile(in: downloads)
        }
        guard let url = await Self.chooseGoogleClientFile(directory: downloads, preselecting: newest) else { return }
        do throws(GoogleOAuthError) {
            try environment.googleOAuth.importClientFile(at: url)
        } catch {
            googleError = error.localizedDescription
        }
        gmailTest = .idle
        refreshGoogleState()
        environment.connection.recheckReadiness()
    }

    /// "Connect Google Account": the browser sign-in for the saved Gmail address. Closing the settings
    /// window does not stop it; "Cancel" does.
    func connectGoogle() async {
        guard googleConnectBlocker == nil, !isDisconnectingGoogle else { return }
        googleError = nil
        gmailTest = .idle
        do throws(GoogleOAuthError) {
            try await environment.googleOAuth.connect(address: savedGmailAddress)
        } catch .cancelled {
            // The user pressed Cancel; nothing to report.
        } catch {
            googleError = error.localizedDescription
        }
        refreshGoogleState()
        environment.connection.recheckReadiness()
    }

    func cancelGoogleConnect() {
        environment.googleOAuth.cancelConnect()
    }

    /// "Disconnect": deletes the saved address's refresh token and revokes it at Google. "Connect"
    /// stays disabled until the revocation is answered, so it cannot revoke a newer sign-in.
    func disconnectGoogle() async {
        guard !isDisconnectingGoogle else { return }
        isDisconnectingGoogle = true
        defer { isDisconnectingGoogle = false }
        googleError = nil
        gmailTest = .idle
        do throws(GoogleOAuthError) {
            try await environment.googleOAuth.disconnect(address: savedGmailAddress)
        } catch {
            googleError = error.localizedDescription
        }
        refreshGoogleState()
        environment.connection.recheckReadiness()
    }

    /// The most recently modified `client_secret_*.json` in `directory`, if any. Runs off the main
    /// actor: listing ~/Downloads may wait for the macOS privacy prompt.
    @concurrent
    nonisolated private static func newestGoogleClientFile(in directory: URL) async -> URL? {
        let keys: Set<URLResourceKey> = [.isRegularFileKey, .contentModificationDateKey]
        guard let entries = try? FileManager.default.contentsOfDirectory(
            at: directory, includingPropertiesForKeys: Array(keys), options: [.skipsHiddenFiles]
        ) else { return nil }
        var newest: (url: URL, date: Date)?
        for url in entries
        where url.lastPathComponent.hasPrefix("client_secret_") && url.pathExtension.lowercased() == "json" {
            guard let values = try? url.resourceValues(forKeys: keys), values.isRegularFile == true else { continue }
            let date = values.contentModificationDate ?? .distantPast
            if newest.map({ date > $0.date }) ?? true {
                newest = (url, date)
            }
        }
        return newest?.url
    }

    /// Shows the panel as a sheet on the settings window (a free-standing panel if that window is
    /// gone). Never `runModal()`: this runs inside a main-actor task, and a modal loop entered from
    /// one does not drain the main queue, so the connection engine, OTP polling and timers would
    /// all stall until the panel closes.
    private static func chooseGoogleClientFile(directory: URL?, preselecting file: URL?) async -> URL? {
        let panel = NSOpenPanel()
        panel.message = String(localized: "Choose the OAuth client JSON (Desktop app) downloaded from Google Cloud Console.")
        panel.prompt = String(localized: "Import")
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [.json]
        // A file URL opens its folder with the file selected.
        panel.directoryURL = file ?? directory
        let host = NSApp.windows.first { $0 is SettingsWindow && $0.isVisible && $0.attachedSheet == nil }
        let response = if let host {
            await panel.beginSheetModal(for: host)
        } else {
            await panel.begin()
        }
        return response == .OK ? panel.url : nil
    }

    // MARK: - Import

    /// "Import openfortivpn Settings": the default file, or one the user picks when there is none.
    func importOpenFortiVPN() {
        let importer = environment.importer
        guard let source = importer.defaultSource() ?? Self.chooseSourceFile() else { return }
        do throws(OpenFortiVPNImportError) {
            importOutcome = .success(try importer.import(from: source))
        } catch {
            importOutcome = .failure(error)
        }
        vpnPassword = ""
        feedback = nil
        reload()
        environment.connection.recheckReadiness()
    }

    /// Shows the automatic first-launch import.
    func presentImport(_ outcome: Result<OpenFortiVPNImportReport, OpenFortiVPNImportError>) {
        importOutcome = outcome
    }

    func dismissImportReport() {
        importOutcome = nil
    }

    private static func chooseSourceFile() -> URL? {
        let panel = NSOpenPanel()
        panel.message = String(localized: "Choose an openfortivpn config file.")
        panel.prompt = String(localized: "Import")
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.showsHiddenFiles = true
        panel.directoryURL = FileManager.default.homeDirectoryForCurrentUser.appending(path: ".config")
        return panel.runModal() == .OK ? panel.url : nil
    }

    // MARK: -

    private func reload() {
        let loaded = Fields(environment.configStore.config)
        base = loaded
        fields = loaded
        storedAccountProblem = Self.accountProblem(host: loaded.host, username: loaded.username, realm: loaded.realm)
        refreshStoredSecrets()
    }

    private func trimmed(_ value: String) -> String {
        value.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
