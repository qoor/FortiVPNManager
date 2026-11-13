import SwiftUI

/// The General pane: the connection's status with its buttons (as in the menu), how the app starts,
/// and importing the VPN client's settings.
struct GeneralPane: View {
    let ui: AppUI
    let navigation: SettingsNavigation
    @Environment(\.self) private var environment

    var body: some View {
        SettingsForm(navigation: navigation) {
            if let connection = ui.connection {
                statusSection(connection, time: TimeStyle(now: ui.clock.now, environment: environment))
            }
            if let config = ui.config {
                startupSection(config)
                importSection(config.account)
            }
        }
        .toggleStyle(.switch)
    }

    // MARK: - Status

    private func statusSection(_ connection: AppInternal.ConnectionSection, time: TimeStyle) -> some View {
        Section("Status") {
            // The note runs under the whole row: beside a long status it would push the status to a
            // line of its own. The planned reconnect is left to the "Next proactive reconnect" row,
            // as it would wrap the status line.
            VStack(alignment: .leading, spacing: 3) {
                LabeledContent("Current status") {
                    Label(
                        connection.statusLine(outlook: ui.schedule?.outlook, time: time, includesPlannedReconnect: false),
                        systemImage: connection.icon.statusSymbol
                    )
                        .foregroundStyle(connection.icon.statusColor)
                }
                if connection.accountAppliesNextConnection {
                    Text("Account changes apply from the next connection.")
                        .footnoteStyle()
                }
            }
            .searchAnchor("Current status", navigation)
            if let reason = connection.waitReason {
                LabeledContent("Reason", value: reason.summary(time: time))
            }
            switch connection.status {
            case let .setupRequired(items):
                ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                    Button(item.actionTitle) { ui.settings.show(item.settingsPane) }
                }
            case let .failed(reason):
                NoticeRow(text: reason.guidance(time: time), style: .error)
                Button(reason.actionTitle) { ui.settings.show(reason.settingsPane) }
            case let .connected(since, tunnelIp, proactiveReconnect):
                LabeledContent(
                    "Connected at",
                    value: String(localized: "\(time.clockTime(since)) (\(time.duration(time.now.timeIntervalSince(since))) ago)")
                )
                .searchAnchor("Connected at", navigation)
                LabeledContent("Tunnel IP") {
                    Text(verbatim: tunnelIp ?? String(localized: "Unknown"))
                        .textSelection(.enabled)
                }
                .searchAnchor("Tunnel IP", navigation)
                LabeledContent("Next proactive reconnect", value: Self.text(of: proactiveReconnect, time: time))
                    .searchAnchor("Next proactive reconnect", navigation)
            default:
                EmptyView()
            }
            if let nextAttempt = connection.nextAttempt {
                LabeledContent("Next attempt", value: time.clockTime(nextAttempt))
            }
            // Always shown; the ones that don't apply now are dimmed, as in the menu.
            HStack {
                Button("Connect") { ui.send(.connect) }
                    .disabled(connection.connect != .enabled)
                Button("Disconnect") { ui.send(.disconnect) }
                    .disabled(connection.disconnect != .enabled)
                Button("Reconnect Now") { ui.send(.reconnectNow) }
                    .disabled(connection.reconnectNow != .enabled)
            }
            .searchAnchor(SettingsSearch.connectionButtons, navigation)
        }
    }

    private static func text(of proactiveReconnect: AppInternal.ProactiveReconnect, time: TimeStyle) -> String {
        switch proactiveReconnect {
        case let .at(at): "\(time.clockTime(at)) (\(time.plannedReconnect(at)))"
        case .pausedBySchedule: String(localized: "Paused by the schedule")
        case .off: String(localized: "Off")
        }
    }

    // MARK: - Startup

    private func startupSection(_ config: AppInternal.ConfigSection) -> some View {
        let openAtLogin = config.openAtLogin
        return Section("Startup") {
            Toggle("Open at login", isOn: Binding(
                get: { openAtLogin.enabled },
                set: { ui.send(.setOpenAtLogin(enabled: $0)) }
            ))
            .disabled(!openAtLogin.available)
            .searchAnchor("Open at login", navigation)
            if !openAtLogin.available {
                Text("Only available for the app installed in /Applications.")
                    .footnoteStyle()
            } else if openAtLogin.requiresApproval {
                Text("Needs approval in System Settings.")
                    .footnoteStyle()
            }
            // The footnote above already gives the reason a copy outside /Applications can't set it.
            if let error = openAtLogin.error, openAtLogin.available || error != .notInstalledCopy {
                NoticeRow(text: error.loginItemMessage, style: .error)
            }
            Toggle("Connect automatically at launch", isOn: Binding(
                get: { config.options.autoConnectOnLaunch },
                set: { ui.send(.setConnectionOption(option: .autoConnectOnLaunch(enabled: $0))) }
            ))
            .searchAnchor("Connect automatically at launch", navigation)
            if ui.schedule?.enabled == true {
                Text("With the schedule on, this connects only during the scheduled hours.")
                    .footnoteStyle()
            }
        }
    }

    // MARK: - Import

    /// The VPN client's settings, imported here (not in the Account pane, which they fill) so the
    /// Account pane holds only the account. The report of the last import stays until closed.
    private func importSection(_ account: AppInternal.AccountFormState) -> some View {
        Section("Import Settings") {
            LabeledContent {
                Button("Import…") { Task { await ui.importClientSettings() } }
            } label: {
                RowTitle("openfortivpn settings", detail: Text("Brings in the account and certificate values from your openfortivpn settings. The original file isn't changed."))
            }
            .searchAnchor(SettingsSearch.importRow, navigation)
            if let report = account.importReport {
                ImportReportView(report: report)
                Button("Close") { ui.send(.dismissImportReport) }
            }
        }
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
                            text: String(localized: "The original file has the password in plain text. Delete it there if you don't need it."),
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

extension AppInternal.ConnectionIcon {
    /// The mark of the General pane's status line: green when connected, as in the settings mockup.
    var statusSymbol: String {
        switch self {
        case .connected: "checkmark.circle.fill"
        case .disconnected: "minus.circle"
        case .inProgress: "arrow.triangle.2.circlepath"
        case .error: "exclamationmark.triangle.fill"
        }
    }

    var statusColor: Color {
        switch self {
        case .connected: .green
        case .disconnected, .inProgress: .secondary
        case .error: .orange
        }
    }
}
