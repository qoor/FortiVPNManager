import SwiftUI

/// The Advanced pane: the Helper, the VPN client it runs, and the settings file.
struct AdvancedPane: View {
    let ui: AppUI

    var body: some View {
        Form {
            if let readiness = ui.readiness, let config = ui.config {
                helperSection(readiness)
                if let client = readiness.client {
                    clientSection(client)
                }
                settingsFileSection(config.file)
            }
        }
        .formStyle(.grouped)
    }

    private func helperSection(_ readiness: AppInternal.ReadinessSection) -> some View {
        Section("Helper") {
            LabeledContent("Status") {
                NoticeRow(text: readiness.helper.label, style: readiness.helper.style)
            }
            StateButton("Register Helper", state: readiness.registerHelper) { ui.send(.registerHelper) }
            if readiness.helper == .requiresApproval {
                Text("Allow background activity for FortiVPNManager in System Settings.")
                    .footnoteStyle()
            }
            LabeledContent("General > Login Items & Extensions > App Background Activity") {
                Button("Open System Settings…") { ui.send(.openHelperApprovalSettings) }
            }
            if let error = readiness.registerError {
                NoticeRow(text: error.helperMessage, style: .error)
            }
        }
    }

    /// The VPN client the Helper runs.
    private func clientSection(_ client: AppInternal.VpnClientInfo) -> some View {
        Section("VPN Client") {
            LabeledContent("openfortivpn", value: client.path ?? String(localized: "Not found"))
            LabeledContent("Version", value: client.version ?? String(localized: "Unknown"))
            if let untested = client.untestedVersion {
                NoticeRow(text: untested.message, style: .warning)
            }
            if let problem = client.problem {
                NoticeRow(text: problem.message, style: .error)
            }
        }
    }

    private func settingsFileSection(_ file: AppInternal.ConfigFileState) -> some View {
        Section("Settings File") {
            LabeledContent(displayPath(file.path)) {
                Button("Show in Finder") { revealInFinder(file.path) }
                    .disabled(!file.exists)
            }
            if !file.exists {
                Text("Not created yet. Saving the account creates it.")
                    .footnoteStyle()
            }
            if let error = file.loadError {
                NoticeRow(text: String(localized: "\(error.message) Using the last valid settings."), style: .error)
                    .textSelection(.enabled)
            }
            ForEach(Array(file.warnings.enumerated()), id: \.offset) { _, warning in
                NoticeRow(text: warning.displayMessage, style: .warning)
            }
        }
    }
}
