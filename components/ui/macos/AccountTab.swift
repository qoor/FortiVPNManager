import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The Account tab: the VPN account and the Gmail account that receives the OTP, saved together.
/// No certificate fields: gateway certificates are trusted automatically. The OTP method and the
/// Google account controls act at once instead of waiting for Save.
struct AccountTab: View {
    let ui: AppUI

    @State private var showsAdvanced = false
    @State private var showsIMAPFallback = false
    @State private var showsOAuthClient = false

    var body: some View {
        if let config = ui.config, let gmail = ui.gmail {
            VStack(spacing: 0) {
                Form {
                    if ui.connection?.accountAppliesNextConnection == true {
                        Section {
                            NoticeRow(text: String(localized: "Account changes apply from the next connection."), style: .info)
                        }
                    }
                    if let report = config.account.importReport {
                        Section("Imported openfortivpn Settings") {
                            ImportReportView(report: report)
                            Button("Close") { ui.send(.dismissImportReport) }
                        }
                    }
                    vpnSection(config.account)
                    GmailSection(ui: ui, account: config.account, gmail: gmail,
                                 showsIMAPFallback: $showsIMAPFallback, showsOAuthClient: $showsOAuthClient)
                }
                .formStyle(.grouped)

                Divider()
                footer(config.account)
                    .padding(12)
            }
        }
    }

    private func vpnSection(_ account: AppInternal.AccountFormState) -> some View {
        Section("VPN") {
            if let problem = account.savedAccountProblem {
                NoticeRow(text: String(localized: "Can't connect with the saved account details. \(problem.message)"), style: .error)
            }
            AccountRow("Server address") {
                TextField("Server address", text: accountField(ui, \.host), prompt: Text(verbatim: "vpn.example.com"))
            }
            AccountRow("Port") {
                TextField("Port", text: accountField(ui, \.port), prompt: Text(verbatim: "443"))
            }
            AccountRow("Username") {
                TextField("Username", text: accountField(ui, \.username))
            }
            AccountRow("Password") {
                SecureField("Password", text: accountField(ui, \.vpnPassword), prompt: storedPrompt(account.vpnPasswordStored))
            }
            DisclosureGroup("Advanced", isExpanded: $showsAdvanced) {
                AccountRow("Realm") {
                    TextField("Realm", text: accountField(ui, \.realm), prompt: Text("None"))
                }
            }
        }
    }

    private func footer(_ account: AppInternal.AccountFormState) -> some View {
        HStack(spacing: 10) {
            Button("Import openfortivpn Settings…") {
                Task { await importClientSettings(defaultFile: account.defaultImportFile) }
            }
            Spacer()
            switch account.feedback {
            case .saved?:
                NoticeRow(text: String(localized: "Saved."), style: .success)
            case let .failed(error)?:
                NoticeRow(text: error.message, style: .error)
                    .lineLimit(3)
            case nil:
                EmptyView()
            }
            StateButton("Revert", state: account.revert) { ui.send(.revertAccount) }
            StateButton("Save", state: account.save) { ui.send(.saveAccount) }
                .keyboardShortcut(.defaultAction)
        }
    }

    /// The VPN client's settings file at its default location, or one the user picks when there is
    /// none.
    private func importClientSettings(defaultFile: String?) async {
        if defaultFile != nil {
            ui.send(.importClientSettings(file: nil))
            return
        }
        let panel = NSOpenPanel()
        panel.message = String(localized: "Choose an openfortivpn config file.")
        panel.prompt = String(localized: "Import")
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.showsHiddenFiles = true
        panel.directoryURL = FileManager.default.homeDirectoryForCurrentUser.appending(path: ".config")
        guard let url = await chooseFile(panel) else { return }
        ui.send(.importClientSettings(file: url.path))
    }
}

/// A field of the Account tab: each edit is sent with all the fields as typed.
@MainActor
private func accountField(_ ui: AppUI, _ keyPath: ReferenceWritableKeyPath<AccountDraft, String>) -> Binding<String> {
    Binding(
        get: { ui.accountDraft[keyPath: keyPath] },
        set: { value in
            guard value != ui.accountDraft[keyPath: keyPath] else { return }
            ui.accountDraft[keyPath: keyPath] = value
            ui.accountEdited()
        }
    )
}

private func storedPrompt(_ stored: Bool) -> Text {
    stored ? Text("Saved in Keychain · enter only to change") : Text("Required")
}

/// The Gmail account that receives the OTP, and how it is read.
private struct GmailSection: View {
    let ui: AppUI
    let account: AppInternal.AccountFormState
    let gmail: AppInternal.GmailSection
    @Binding var showsIMAPFallback: Bool
    @Binding var showsOAuthClient: Bool
    @Environment(\.self) private var environment

    var body: some View {
        Section {
            AccountRow("Gmail address") {
                TextField("Gmail address", text: accountField(ui, \.gmailAddress), prompt: Text(verbatim: "name@example.com"))
            }
            AccountRow("Connection method") {
                Picker("Connection method", selection: method) {
                    Text("Google account (Gmail API)").tag(AppInternal.OtpMethod.gmailApi)
                    Text("App password (IMAP)").tag(AppInternal.OtpMethod.imap)
                }
                .pickerStyle(.menu)
                .fixedSize()
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            Text("Google account is recommended: the Gmail API detects the OTP email as soon as it arrives. Over IMAP, Gmail can show it several seconds later.")
                .footnoteStyle()
            if let error = gmail.methodError {
                NoticeRow(text: error.message, style: .error)
            }
            switch gmail.method {
            case .gmailApi:
                googleAccountRows
                DisclosureGroup("IMAP fallback (optional)", isExpanded: $showsIMAPFallback) {
                    appPasswordRow(required: false)
                    Text("If the Gmail API can't be used during a connection, the OTP email is read over IMAP with this app password instead.")
                        .footnoteStyle()
                }
                DisclosureGroup("OAuth client (advanced)", isExpanded: oauthClientExpanded) {
                    googleClientRows
                }
            case .imap:
                appPasswordRow(required: true)
            }
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                StateButton("Test Gmail Connection", state: gmail.testButton) { ui.send(.testGmail) }
                gmailTestResult
            }
        } header: {
            Text("2-Step Verification (Gmail)")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                switch gmail.method {
                case .gmailApi:
                    Text("Connecting opens Google sign-in in your web browser. Sign in with the Gmail address above.")
                case .imap:
                    Text("The app password is the 16-character code created under Google Account > Security > 2-Step Verification > App passwords. Spaces are ignored.")
                }
                Text("Recommended: create a Gmail filter (search `subject:AuthCode` → Never send it to Spam). If the OTP email lands in Spam, automatic entry fails.")
            }
            .footnoteStyle()
        }
    }

    private var method: Binding<AppInternal.OtpMethod> {
        Binding(
            get: { gmail.method },
            set: { method in
                if method != gmail.method { ui.send(.setOtpMethod(method: method)) }
            }
        )
    }

    /// The IMAP app password: required for the IMAP method, an optional fallback for the Gmail API.
    private func appPasswordRow(required: Bool) -> some View {
        AccountRow("App password") {
            SecureField(
                "App password",
                text: accountField(ui, \.appPassword),
                prompt: account.appPasswordStored || required ? storedPrompt(account.appPasswordStored) : Text("Optional")
            )
        }
    }

    // MARK: - Google account

    @ViewBuilder
    private var googleAccountRows: some View {
        AccountStatusRow("Google account") {
            googleAccountStatus
        } actions: {
            StateButton("Cancel", state: gmail.cancelSignIn) { ui.send(.cancelGoogleSignIn) }
            StateButton("Disconnect", state: gmail.disconnectGoogle) { ui.send(.disconnectGoogleAccount) }
            StateButton("Connect Google Account", state: gmail.connectGoogle) { ui.send(.connectGoogleAccount) }
        }
        if let blocker = gmail.connectBlocker {
            Text(blocker.message)
                .footnoteStyle()
        }
        if let error = gmail.googleError {
            NoticeRow(text: error.message(time: TimeStyle(now: Date(), environment: environment)), style: .error)
                .textSelection(.enabled)
        }
    }

    @ViewBuilder
    private var googleAccountStatus: some View {
        switch gmail.googleAccount {
        case .signingIn:
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Waiting for Google sign-in in the browser…")
                    .foregroundStyle(.secondary)
            }
        case .disconnecting:
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Disconnecting")
                    .foregroundStyle(.secondary)
            }
        case let .connected(address):
            NoticeRow(text: String(localized: "Connected (\(address))"), style: .success)
        case .notConnected:
            Text("Not connected")
                .foregroundStyle(.secondary)
        }
    }

    /// Expanded on its own when the build has no built-in client, so the import is easy to find.
    private var oauthClientExpanded: Binding<Bool> {
        Binding(
            get: { showsOAuthClient || gmail.oauthClient == .notConfigured },
            set: { showsOAuthClient = $0 }
        )
    }

    @ViewBuilder
    private var googleClientRows: some View {
        AccountStatusRow("OAuth client") {
            switch gmail.oauthClient {
            case let .builtIn(clientId), let .imported(clientId):
                NoticeRow(text: gmail.oauthClient.label, style: .success)
                    .help(clientId)
            case .notConfigured:
                NoticeRow(text: gmail.oauthClient.label, style: .warning)
            }
        } actions: {
            StateButton("Use Built-in Client", state: gmail.useBuiltInClient) { ui.send(.useBuiltInOAuthClient) }
            StateButton("Import JSON…", state: gmail.importClient) {
                Task { await importOAuthClient() }
            }
        }
        if gmail.importedClientSecretMissing {
            NoticeRow(text: String(localized: "The imported client's secret is missing from Keychain. Import the JSON again."), style: .warning)
        }
        Text("Importing a Desktop app OAuth client JSON from another Google Cloud project replaces the built-in client. Changing the client requires connecting the Google account again.")
            .footnoteStyle()
    }

    @ViewBuilder
    private var gmailTestResult: some View {
        if gmail.test == .running {
            ProgressView().controlSize(.small)
            Text("Checking…").foregroundStyle(.secondary)
        } else if let result = gmail.test.result {
            NoticeRow(text: result.text, style: result.succeeded ? .success : .error)
        }
    }

    /// An open panel in ~/Downloads with the newest `client_secret_*.json` selected; the runtime reads
    /// the file.
    private func importOAuthClient() async {
        let downloads = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
        var newest: URL?
        if let downloads {
            newest = await Self.newestClientFile(in: downloads)
        }
        let panel = NSOpenPanel()
        panel.message = String(localized: "Choose the OAuth client JSON (Desktop app) downloaded from Google Cloud Console.")
        panel.prompt = String(localized: "Import")
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [.json]
        // A file URL opens its folder with the file selected.
        panel.directoryURL = newest ?? downloads
        guard let url = await chooseFile(panel) else { return }
        ui.send(.importOAuthClient(file: url.path))
    }

    /// The most recently modified `client_secret_*.json` in `directory`, if any. Runs off the main
    /// actor: listing ~/Downloads may wait for the macOS privacy prompt.
    @concurrent
    nonisolated private static func newestClientFile(in directory: URL) async -> URL? {
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
}

/// What an import of the VPN client's settings did. The source file is never modified.
private struct ImportReportView: View {
    let report: AppInternal.ClientSettingsImport

    var body: some View {
        switch report {
        case let .imported(report):
            VStack(alignment: .leading, spacing: 6) {
                Text("Imported from \(displayPath(report.source)).")
                Text(verbatim: report.summary)
                    .foregroundStyle(.secondary)
                let password = report.passwordStatus
                NoticeRow(text: password.text, style: password.style)
                if report.password != nil {
                    HStack(alignment: .firstTextBaseline) {
                        NoticeRow(
                            text: String(localized: "The original file contains the password in plain text. The app never changes that file, so delete the password yourself if you don't need it."),
                            style: .warning
                        )
                        Spacer()
                        Button("Show in Finder") { revealInFinder(report.source) }
                    }
                }
                if let message = report.unknownKeysMessage {
                    NoticeRow(text: message, style: .info)
                }
                if let message = report.ignoredKeysMessage {
                    NoticeRow(text: message, style: .info)
                }
                if let message = report.unreadableLinesMessage {
                    NoticeRow(text: message, style: .warning)
                }
            }
            .textSelection(.enabled)
        case let .failed(error):
            NoticeRow(text: error.message, style: .error)
        }
    }
}

/// A labelled row of status text with its buttons, laid out like `AccountRow`: the status starts at
/// the left edge of the field column (a zero ideal width keeps the grouped form from widening the
/// column for long text; it wraps instead), the buttons sit at the right and never truncate.
private struct AccountStatusRow<Status: View, Actions: View>: View {
    let title: LocalizedStringKey
    let status: Status
    let actions: Actions

    init(_ title: LocalizedStringKey, @ViewBuilder status: () -> Status, @ViewBuilder actions: () -> Actions) {
        self.title = title
        self.status = status()
        self.actions = actions()
    }

    var body: some View {
        LabeledContent(title) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                status
                    .multilineTextAlignment(.leading)
                    .frame(minWidth: 0, idealWidth: 0, maxWidth: .infinity, alignment: .leading)
                actions
                    .fixedSize()
            }
        }
    }
}

/// A labelled input row. Credentials read best in ordinary input boxes with the prompt and the typed
/// text both left-aligned; the grouped form style right-aligns fields it lays out itself, so the field
/// is wrapped in `LabeledContent` and styled here.
private struct AccountRow<Field: View>: View {
    let title: LocalizedStringKey
    let field: Field

    init(_ title: LocalizedStringKey, @ViewBuilder field: () -> Field) {
        self.title = title
        self.field = field()
    }

    var body: some View {
        LabeledContent(title) {
            field
                .labelsHidden()
                .textFieldStyle(.roundedBorder)
                .multilineTextAlignment(.leading)
        }
    }
}
