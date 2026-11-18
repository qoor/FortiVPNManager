import SwiftUI

/// The Advanced pane: the Helper, the VPN client it runs, and the settings file.
struct AdvancedPane: View {
    let ui: AppUI
    let navigation: SettingsNavigation

    var body: some View {
        SettingsForm(navigation: navigation) {
            if let readiness = ui.readiness, let config = ui.config {
                helperSection(readiness)
                if let found = readiness.client {
                    clientSection(found)
                }
                settingsFileSection(config.file)
            }
        }
    }

    private func helperSection(_ readiness: AppInternal.ReadinessSection) -> some View {
        Section("Helper") {
            LabeledContent("Status") {
                NoticeRow(text: readiness.helper.label, style: readiness.helper.style)
            }
            .searchAnchor("Helper", navigation)
            StateButton("Register Helper", state: readiness.registerHelper) { ui.send(.registerHelper) }
                .searchAnchor("Register Helper", navigation)
            LabeledContent {
                Button("Open System Settings…") { ui.send(.openHelperApprovalSettings) }
            } label: {
                // While approval is needed the location becomes the instruction, instead of a second
                // row repeating it.
                RowTitle("App Background Activity", detail: readiness.helper == .requiresApproval
                    ? Text("Allow background activity for FortiVPNManager in General › Login Items & Extensions.")
                    : Text("General › Login Items & Extensions"))
            }
            .searchAnchor("App Background Activity", navigation)
            if let error = readiness.registerError {
                NoticeRow(text: error.helperMessage, style: .error)
            }
        }
    }

    /// The VPN client the Helper runs.
    private func clientSection(_ found: AppInternal.VpnClientInfo) -> some View {
        Section("VPN client") {
            LabeledContent("Type", value: "openconnect")
                .searchAnchor("Type", navigation)
            LabeledContent("Version", value: found.version ?? String(localized: "Unknown"))
                .searchAnchor("Version", navigation)
            if let untested = found.untestedVersion {
                NoticeRow(text: untested.message, style: .warning)
            }
            if let problem = found.problem {
                NoticeRow(text: problem.message, style: .error)
            }
        }
    }

    private func settingsFileSection(_ file: AppInternal.ConfigFileState) -> some View {
        Section("Settings File") {
            LabeledContent {
                Button("Show in Finder") { revealInFinder(file.path) }
                    .disabled(!file.exists)
            } label: {
                Text(verbatim: displayPath(file.path))
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .searchAnchor("Settings File", navigation)
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
