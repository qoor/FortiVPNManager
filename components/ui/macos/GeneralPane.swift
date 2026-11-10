import SwiftUI

/// The General pane: the connection's status with its buttons (as in the menu), and how the app
/// starts.
struct GeneralPane: View {
    let ui: AppUI
    @Environment(\.self) private var environment

    var body: some View {
        Form {
            if let connection = ui.connection {
                statusSection(connection, time: TimeStyle(now: ui.clock.now, environment: environment))
            }
            if let config = ui.config {
                startupSection(config)
            }
        }
        .formStyle(.grouped)
    }

    // MARK: - Status

    private func statusSection(_ connection: AppInternal.ConnectionSection, time: TimeStyle) -> some View {
        Section("Status") {
            LabeledContent("Current status", value: connection.statusLine(outlook: ui.schedule?.outlook, time: time))
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
                LabeledContent("Tunnel IP", value: tunnelIp ?? String(localized: "Unknown"))
                LabeledContent("Next proactive reconnect", value: Self.text(of: proactiveReconnect, time: time))
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
            .controlSize(.mini)
            .disabled(!openAtLogin.available)
            if !openAtLogin.available {
                Text("Open at login can only be set for the app installed in /Applications.")
                    .footnoteStyle()
            } else if openAtLogin.requiresApproval {
                Text("Needs approval in System Settings > General > Login Items & Extensions.")
                    .footnoteStyle()
            }
            if let error = openAtLogin.error {
                NoticeRow(text: error.loginItemMessage, style: .error)
            }
            Toggle("Connect automatically at launch", isOn: Binding(
                get: { config.options.autoConnectOnLaunch },
                set: { ui.send(.setConnectionOption(option: .autoConnectOnLaunch(enabled: $0))) }
            ))
            .controlSize(.mini)
            if ui.schedule?.enabled == true {
                Text("With the schedule on, this connects only during the scheduled hours.")
                    .footnoteStyle()
            }
        }
    }
}
