import FortiConfig
import SwiftUI

/// Tab 1, Account (DESIGN §5.6): the VPN account and the Gmail account that receives the OTP, saved
/// together. No certificate fields: gateway certificates are trusted automatically. The OTP method
/// (Gmail API with a Google account, or IMAP with an app password) applies as soon as it is picked.
struct AccountTab: View {
    let environment: AppEnvironment
    @Bindable var editor: AccountEditor
    let accountNotice: AccountChangeNotice

    @State private var showsAdvanced = false
    @State private var showsIMAPFallback = false

    private var oauth: GoogleOAuthService { environment.googleOAuth }
    private var otpMethod: AppConfig.OTP.Method { environment.configStore.otpMethod }

    var body: some View {
        VStack(spacing: 0) {
            Form {
                if accountNotice.isPending {
                    Section {
                        NoticeRow(text: String(localized: "Account changes apply from the next connection."), style: .info)
                    }
                }
                if let outcome = editor.importOutcome {
                    Section("Imported openfortivpn Settings") {
                        ImportReportView(outcome: outcome)
                        Button("Close") { editor.dismissImportReport() }
                    }
                }
                vpnSection
                gmailSection
            }
            .formStyle(.grouped)

            Divider()
            footer
                .padding(12)
        }
        .onChange(of: environment.configStore.config) { editor.reloadIfUnchanged() }
        .onChange(of: editor.hasChanges) { editor.editingChanged() }
        .onChange(of: editor.fields.gmailAddress) { editor.resetGmailTest() }
        .onChange(of: editor.appPassword) { editor.resetGmailTest() }
        .onChange(of: otpMethod) { editor.resetGmailTest() }
        .onChange(of: googleStateKey, initial: true) { editor.refreshGoogleState() }
        .task(id: editor.secretsKey) { editor.refreshStoredSecrets() }
    }

    /// Changes whenever the Google client or authorization may have changed.
    private var googleStateKey: [String] {
        [String(oauth.revision), oauth.clientID, editor.savedGmailAddress]
    }

    private var vpnSection: some View {
        Section("VPN") {
            if let problem = editor.storedAccountProblem {
                NoticeRow(text: String(localized: "Can't connect with the saved account details. \(problem)"), style: .error)
            }
            AccountRow("Server address") {
                TextField("Server address", text: $editor.fields.host, prompt: Text(verbatim: "vpn.example.com"))
            }
            AccountRow("Port") {
                TextField("Port", text: $editor.fields.port, prompt: Text(verbatim: "443"))
            }
            AccountRow("Username") {
                TextField("Username", text: $editor.fields.username)
            }
            AccountRow("Password") {
                SecureField("Password", text: $editor.vpnPassword, prompt: storedPrompt(editor.vpnPasswordStored))
            }
            DisclosureGroup("Advanced", isExpanded: $showsAdvanced) {
                AccountRow("Realm") {
                    TextField("Realm", text: $editor.fields.realm, prompt: Text("None"))
                }
            }
        }
    }

    private var gmailSection: some View {
        Section {
            AccountRow("Gmail address") {
                TextField("Gmail address", text: $editor.fields.gmailAddress, prompt: Text(verbatim: "name@example.com"))
            }
            AccountRow("Connection method") {
                Picker("Connection method", selection: otpMethodSelection) {
                    Text("Google account (Gmail API)").tag(AppConfig.OTP.Method.gmailAPI)
                    Text("App password (IMAP)").tag(AppConfig.OTP.Method.imap)
                }
                .pickerStyle(.menu)
                .fixedSize()
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            Text("Google account is recommended: the Gmail API detects the OTP email as soon as it arrives. Over IMAP, Gmail can show it several seconds later.")
                .footnoteStyle()
            if let error = editor.methodError {
                NoticeRow(text: error, style: .error)
            }
            switch otpMethod {
            case .gmailAPI:
                googleAccountRows
                DisclosureGroup("IMAP fallback (optional)", isExpanded: $showsIMAPFallback) {
                    appPasswordRow(required: false)
                    Text("If the Gmail API can't be used during a connection, the OTP email is read over IMAP with this app password instead.")
                        .footnoteStyle()
                }
            case .imap:
                appPasswordRow(required: true)
            }
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Button("Test Gmail Connection") {
                    Task { await editor.testGmail() }
                }
                .disabled(editor.gmailTest == .running || oauth.isConnecting)
                gmailTestResult
            }
        } header: {
            Text("2-Step Verification (Gmail)")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                switch otpMethod {
                case .gmailAPI:
                    Text("Import the JSON of a Desktop app OAuth client from a Google Cloud project with the Gmail API enabled. Connecting opens Google sign-in in your web browser.")
                case .imap:
                    Text("The app password is the 16-character code created under Google Account > Security > 2-Step Verification > App passwords. Spaces are ignored.")
                }
                Text("Recommended: create a Gmail filter (search `subject:AuthCode` → Never send it to Spam). If the OTP email lands in Spam, automatic entry fails.")
            }
            .footnoteStyle()
        }
    }

    /// The picker writes `otp.method` to config.yaml as soon as it changes.
    private var otpMethodSelection: Binding<AppConfig.OTP.Method> {
        Binding(
            get: { otpMethod },
            set: { editor.setOTPMethod($0) }
        )
    }

    /// The IMAP app password: required for the IMAP method, an optional fallback for the Gmail API.
    private func appPasswordRow(required: Bool) -> some View {
        AccountRow("App password") {
            SecureField(
                "App password",
                text: $editor.appPassword,
                prompt: editor.appPasswordStored || required ? storedPrompt(editor.appPasswordStored) : Text("Optional")
            )
        }
    }

    // MARK: - Google account

    @ViewBuilder
    private var googleAccountRows: some View {
        let busy = oauth.isConnecting || editor.isDisconnectingGoogle
        AccountStatusRow("OAuth client") {
            googleClientStatus
        } actions: {
            Button("Import JSON…") {
                Task { await editor.importGoogleClient() }
            }
            .disabled(busy || editor.isChoosingGoogleClient)
        }
        AccountStatusRow("Google account") {
            googleAccountStatus
        } actions: {
            if oauth.isConnecting {
                Button("Cancel") { editor.cancelGoogleConnect() }
            } else if editor.googleAuthorized {
                Button("Disconnect") {
                    Task { await editor.disconnectGoogle() }
                }
                .disabled(busy)
            } else {
                Button("Connect Google Account") {
                    Task { await editor.connectGoogle() }
                }
                .disabled(busy || editor.googleConnectBlocker != nil)
            }
        }
        if !busy, !editor.googleAuthorized, let blocker = editor.googleConnectBlocker {
            Text(blocker)
                .footnoteStyle()
        }
        if let error = editor.googleError {
            NoticeRow(text: error, style: .error)
                .textSelection(.enabled)
        }
    }

    @ViewBuilder
    private var googleClientStatus: some View {
        let clientID = oauth.clientID
        if clientID.isEmpty {
            Text("Not imported")
                .foregroundStyle(.secondary)
        } else if editor.googleClientStored {
            NoticeRow(text: String(localized: "Configured (\(Self.clientIDTail(clientID)))"), style: .success)
                .help(clientID)
        } else {
            NoticeRow(text: String(localized: "The client secret is missing from Keychain. Import the JSON again."), style: .warning)
        }
    }

    @ViewBuilder
    private var googleAccountStatus: some View {
        if oauth.isConnecting {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Waiting for Google sign-in in the browser…")
                    .foregroundStyle(.secondary)
            }
        } else if editor.isDisconnectingGoogle {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Disconnecting")
                    .foregroundStyle(.secondary)
            }
        } else if editor.googleAuthorized {
            NoticeRow(text: String(localized: "Connected (\(editor.savedGmailAddress))"), style: .success)
        } else {
            Text("Not connected")
                .foregroundStyle(.secondary)
        }
    }

    /// The end of a client ID without the `.apps.googleusercontent.com` suffix, e.g. `…pe2vcd6960`.
    private static func clientIDTail(_ clientID: String) -> String {
        let suffix = ".apps.googleusercontent.com"
        let stem = clientID.hasSuffix(suffix) ? String(clientID.dropLast(suffix.count)) : clientID
        return stem.count > 10 ? "…" + stem.suffix(10) : stem
    }

    @ViewBuilder
    private var gmailTestResult: some View {
        switch editor.gmailTest {
        case .idle:
            EmptyView()
        case .running:
            ProgressView().controlSize(.small)
            Text("Checking…").foregroundStyle(.secondary)
        case let .succeeded(message):
            NoticeRow(text: message, style: .success)
        case let .failed(message):
            NoticeRow(text: message, style: .error)
        }
    }

    private var footer: some View {
        HStack(spacing: 10) {
            Button("Import openfortivpn Settings…") { editor.importOpenFortiVPN() }
            Spacer()
            switch editor.feedback {
            case .saved?:
                NoticeRow(text: String(localized: "Saved."), style: .success)
            case let .error(message)?:
                NoticeRow(text: message, style: .error)
                    .lineLimit(3)
            case nil:
                EmptyView()
            }
            Button("Revert") { editor.revert() }
                .disabled(!editor.hasChanges)
            Button("Save") { editor.save() }
                .keyboardShortcut(.defaultAction)
                .disabled(!editor.hasChanges)
        }
    }

    private func storedPrompt(_ stored: Bool) -> Text {
        stored ? Text("Saved in Keychain · enter only to change") : Text("Required")
    }
}

/// What an openfortivpn import did. The source file is never modified.
private struct ImportReportView: View {
    let outcome: Result<OpenFortiVPNImportReport, OpenFortiVPNImportError>

    var body: some View {
        switch outcome {
        case let .success(report):
            VStack(alignment: .leading, spacing: 6) {
                Text("Imported from \(displayPath(report.sourceURL)).")
                Text(summary(report))
                    .foregroundStyle(.secondary)
                passwordStatus(report)
                if !report.unknownKeys.isEmpty {
                    NoticeRow(
                        text: String(localized: """
                            Ignored unknown keys: \(report.unknownKeys.joined(separator: ", ")). \
                            openfortivpn doesn't read the settings after such keys.
                            """),
                        style: .info
                    )
                }
                if !report.ignoredKeys.isEmpty {
                    NoticeRow(
                        text: String(localized: "Options not imported: \(report.ignoredKeys.joined(separator: ", "))"),
                        style: .info
                    )
                }
                if !report.invalidLines.isEmpty {
                    NoticeRow(
                        text: String(localized: "Unreadable lines: \(report.invalidLines.map(String.init).joined(separator: ", "))"),
                        style: .warning
                    )
                }
            }
            .textSelection(.enabled)
        case let .failure(error):
            NoticeRow(text: error.localizedDescription, style: .error)
        }
    }

    private func summary(_ report: OpenFortiVPNImportReport) -> String {
        var parts = [String(localized: "Server: \(report.host):\(report.port)")]
        if !report.username.isEmpty { parts.append(String(localized: "Username: \(report.username)")) }
        if !report.realm.isEmpty { parts.append(String(localized: "Realm: \(report.realm)")) }
        parts.append(String(localized: "Server certificates: \(report.trustedCertCount)"))
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private func passwordStatus(_ report: OpenFortiVPNImportReport) -> some View {
        if report.passwordStored {
            NoticeRow(text: String(localized: "Saved the password to Keychain."), style: .success)
        } else if let error = report.passwordError {
            NoticeRow(text: error, style: .warning)
        } else if !report.containsPlaintextPassword {
            NoticeRow(text: String(localized: "The file has no password. Enter the VPN password above."), style: .info)
        }
        if report.containsPlaintextPassword {
            HStack(alignment: .firstTextBaseline) {
                NoticeRow(
                    text: String(localized: "The original file contains the password in plain text. The app never changes that file, so delete the password yourself if you don't need it."),
                    style: .warning
                )
                Spacer()
                Button("Show in Finder") { revealInFinder(report.sourceURL) }
            }
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
