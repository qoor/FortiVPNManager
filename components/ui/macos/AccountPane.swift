import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The Account pane: the VPN account and the Gmail account that receives the OTP, saved together
/// with Save in the toolbar (and File > Save). No certificate fields: gateway certificates are trusted
/// automatically. The OTP method and the Google account controls act at once instead of waiting for
/// Save.
struct AccountPane: View {
    let ui: AppUI
    @Bindable var navigation: SettingsNavigation

    var body: some View {
        if let config = ui.config, let gmail = ui.gmail {
            SettingsForm(navigation: navigation) {
                if config.account.feedback != nil || config.account.savedAccountProblem != nil {
                    feedbackSection(config.account)
                }
                vpnSection(config.account)
                GmailSection(ui: ui, navigation: navigation, account: config.account, gmail: gmail)
            }
            .toolbar {
                ToolbarItemGroup(placement: .primaryAction) {
                    StateButton("Revert", state: config.account.revert) { ui.send(.revertAccount) }
                    StateButton("Save", state: config.account.save) { ui.send(.saveAccount) }
                        .buttonStyle(.borderedProminent)
                        .keyboardShortcut(.defaultAction)
                        .help("Save account settings (⌘S)")
                }
            }
        }
    }

    /// What the last Save did, and a saved account the runtime can't connect with.
    private func feedbackSection(_ account: AppInternal.AccountFormState) -> some View {
        Section {
            if let problem = account.savedAccountProblem {
                NoticeRow(text: String(localized: "Saved account error: \(problem.message)"), style: .error)
            }
            switch account.feedback {
            case .saved?:
                NoticeRow(text: String(localized: "Saved."), style: .success)
            case let .failed(error)?:
                NoticeRow(text: error.message, style: .error)
            case nil:
                EmptyView()
            }
        }
    }

    private func vpnSection(_ account: AppInternal.AccountFormState) -> some View {
        Section {
            AccountRow("Server address") {
                TextField("Server address", text: accountField(ui, \.host), prompt: Text(verbatim: "vpn.example.com"))
            }
            .searchAnchor("Server address", navigation)
            AccountRow("Port") {
                // Sized for a port number, in the column the other fields start at (HIG Text fields).
                HStack {
                    TextField("Port", text: accountField(ui, \.port), prompt: Text(verbatim: "443"))
                        .frame(width: 80)
                    Spacer(minLength: 0)
                }
            }
            .searchAnchor("Port", navigation)
            AccountRow("Username") {
                TextField("Username", text: accountField(ui, \.username))
            }
            .searchAnchor("Username", navigation)
            AccountRow("Password") {
                SecureField("Password", text: accountField(ui, \.vpnPassword), prompt: storedPrompt(account.vpnPasswordStored))
            }
            .searchAnchor("Password", navigation)
            // A disclosed setting is a row of its own, separated like the others, with its label
            // indented under the disclosure and its field in the shared column (HIG Layout).
            DisclosureGroup("Realm (advanced)", isExpanded: $navigation.showsRealm) {}
            if navigation.showsRealm {
                AccountRow("Realm", indented: true) {
                    TextField("Realm", text: accountField(ui, \.realm), prompt: Text("Optional"))
                }
                .searchAnchor("Realm", navigation)
            }
        } header: {
            Text("VPN")
        } footer: {
            if ui.connection?.accountAppliesNextConnection == true {
                Text("Account changes apply from the next connection.")
                    .footnoteStyle()
            }
        }
    }
}

/// A field of the Account pane: each edit is sent with all the fields as typed.
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
    stored ? Text("Saved in Keychain · type to change") : Text("Required")
}

/// The Gmail account that receives the OTP, and how it is read.
private struct GmailSection: View {
    let ui: AppUI
    @Bindable var navigation: SettingsNavigation
    let account: AppInternal.AccountFormState
    let gmail: AppInternal.GmailSection
    @State private var confirmsGoogleDisconnect = false
    @Environment(\.self) private var environment

    var body: some View {
        Section {
            AccountRow("Gmail address") {
                TextField("Gmail address", text: accountField(ui, \.gmailAddress), prompt: Text(verbatim: "name@example.com"))
            }
            .searchAnchor("Gmail address", navigation)
            Picker("Connection method", selection: method) {
                Text("Google account (Gmail API)").tag(AppInternal.OtpMethod.gmailApi)
                Text("App password (IMAP)").tag(AppInternal.OtpMethod.imap)
            }
            .help("Google account is recommended: it sees the OTP email at once. IMAP can be several seconds late.")
            .searchAnchor("Connection method", navigation)
            if let error = gmail.methodError {
                NoticeRow(text: error.message, style: .error)
            }
            switch gmail.method {
            case .gmailApi:
                googleAccountRows
                DisclosureGroup("IMAP fallback (optional)", isExpanded: $navigation.showsIMAPFallback) {}
                    .searchAnchor("IMAP fallback (optional)", navigation)
                if navigation.showsIMAPFallback {
                    // The explanation runs under the field, so it doesn't widen the label column.
                    VStack(alignment: .leading, spacing: 4) {
                        appPasswordRow(required: false, indented: true)
                        Text("When the Gmail API is unavailable, this app password reads the OTP email.")
                            .footnoteStyle()
                            .subordinate()
                    }
                    .searchAnchor("App password", navigation)
                }
                DisclosureGroup("OAuth client (advanced)", isExpanded: oauthClientExpanded) {}
                if oauthClientExpanded.wrappedValue {
                    googleClientRows
                }
            case .imap:
                appPasswordRow(required: true, indented: false)
                    .searchAnchor("App password", navigation)
            }
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                StateButton("Test Gmail Connection", state: gmail.testButton) { ui.send(.testGmail) }
                gmailTestResult
            }
            .searchAnchor("Test Gmail Connection", navigation)
        } header: {
            Text("2-Step Verification (Gmail)")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                switch gmail.method {
                case .gmailApi:
                    Text("Connecting opens Google sign-in in your web browser. Sign in with the Gmail address above.")
                case .imap:
                    Text("Uses a 16-character app password created in your Google Account. Spaces are ignored.")
                }
                Text("Recommended: create a Gmail filter for subject:AuthCode that never sends it to Spam.")
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
    private func appPasswordRow(required: Bool, indented: Bool) -> some View {
        AccountRow("App password", indented: indented) {
            SecureField("App password", text: accountField(ui, \.appPassword), prompt: appPasswordPrompt(required: required))
        }
    }

    private func appPasswordPrompt(required: Bool) -> Text {
        if account.appPasswordStored { return storedPrompt(true) }
        return required ? Text("16-character app password") : Text("Optional")
    }

    // MARK: - Google account

    @ViewBuilder
    private var googleAccountRows: some View {
        AccountStatusRow("Google account") {
            googleAccountStatus
        } actions: {
            StateButton("Cancel", state: gmail.cancelSignIn) { ui.send(.cancelGoogleSignIn) }
            StateButton("Disconnect", state: gmail.disconnectGoogle) { confirmsGoogleDisconnect = true }
            StateButton("Connect Google Account…", state: gmail.connectGoogle) { ui.send(.connectGoogleAccount) }
        }
        .searchAnchor("Google account", navigation)
        // Undoing it takes a new sign-in in the browser. The person chose to disconnect, so the
        // confirming button isn't styled destructive (HIG Alerts).
        .alert("Disconnect the Google account?", isPresented: $confirmsGoogleDisconnect) {
            Button("Disconnect") { ui.send(.disconnectGoogleAccount) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("To connect again, you'll sign in to Google in your web browser.")
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
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                ProgressView().controlSize(.small)
                // Wraps in the narrow field column rather than being cut.
                Text("Signing in via browser…")
                    .foregroundStyle(.secondary)
                    .lineLimit(nil)
                    .fixedSize(horizontal: false, vertical: true)
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
            get: { navigation.showsOAuthClient || gmail.oauthClient == .notConfigured },
            set: { navigation.showsOAuthClient = $0 }
        )
    }

    /// The client in use, and under it its buttons, each a row of its own.
    @ViewBuilder
    private var googleClientRows: some View {
        LabeledContent {
            switch gmail.oauthClient {
            case let .builtIn(clientId), let .imported(clientId):
                Text(verbatim: gmail.oauthClient.label)
                    .help(clientId)
            case .notConfigured:
                NoticeRow(text: gmail.oauthClient.label, style: .warning)
            }
        } label: {
            RowTitle("OAuth client", detail: Text("Changing the client requires connecting the Google account again."))
                .subordinate()
        }
        .searchAnchor("OAuth client", navigation)
        HStack {
            Spacer()
            StateButton("Use Built-in Client", state: gmail.useBuiltInClient) { ui.send(.useBuiltInOAuthClient) }
            StateButton("Import JSON…", state: gmail.importClient) {
                Task { await importOAuthClient() }
            }
            .help("Use a Desktop app OAuth client JSON instead of the built-in client.")
        }
        .searchAnchor(SettingsSearch.oauthClientButtons, navigation)
        if gmail.importedClientSecretMissing {
            NoticeRow(text: String(localized: "The imported client's secret is missing from Keychain. Import the JSON again."), style: .warning)
        }
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

/// A labelled row of status text with its buttons, laid out like `AccountRow`: the status starts at
/// the left edge of the field column and the buttons sit at the right. When both don't fit on one
/// line, the buttons move under the status, so neither the text nor a button title is cut or split.
/// A zero ideal width keeps the grouped form from widening the column for long text.
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
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    status
                        .fixedSize()
                    Spacer(minLength: 0)
                    actions
                        .fixedSize()
                }
                VStack(alignment: .leading, spacing: 6) {
                    status
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 8) { actions }
                        .fixedSize()
                }
            }
            .frame(minWidth: 0, idealWidth: 0, maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// A labelled input row. Credentials read best in ordinary input boxes with the prompt and the typed
/// text both left-aligned; the grouped form style right-aligns fields it lays out itself, so the field
/// is wrapped in `LabeledContent` and styled here. An indented row (a disclosed setting) indents only
/// its label, so its field stays in the column of the others.
private struct AccountRow<Field: View>: View {
    let title: LocalizedStringKey
    let indented: Bool
    let field: Field

    init(_ title: LocalizedStringKey, indented: Bool = false, @ViewBuilder field: () -> Field) {
        self.title = title
        self.indented = indented
        self.field = field()
    }

    var body: some View {
        LabeledContent {
            field
                .labelsHidden()
                .textFieldStyle(.roundedBorder)
                .multilineTextAlignment(.leading)
        } label: {
            if indented {
                Text(title).subordinate()
            } else {
                Text(title)
            }
        }
    }
}
