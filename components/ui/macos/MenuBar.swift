import AppKit
import SwiftUI

/// The menu bar item, the app's only scene; the settings window is AppKit
/// (`SettingsWindowController`).
struct MenuBarScene: Scene {
    let ui: AppUI

    var body: some Scene {
        MenuBarExtra {
            MenuContent(ui: ui)
        } label: {
            MenuBarIcon(ui: ui)
        }
        .menuBarExtraStyle(.menu)
    }
}

struct MenuBarIcon: View {
    let ui: AppUI

    var body: some View {
        let icon = ui.connection?.icon ?? .disconnected
        Image(systemName: icon.symbolName)
            .accessibilityLabel("FortiVPNManager: \(icon.accessibilityText)")
    }
}

struct MenuContent: View {
    let ui: AppUI
    @Environment(\.self) private var environment

    var body: some View {
        if let connection = ui.connection {
            let time = TimeStyle(now: ui.clock.now, environment: environment)
            let outlook = ui.schedule?.outlook
            Text(connection.statusLine(outlook: outlook, time: time))
            if case let .connected(since, tunnelIp, _) = connection.status {
                Text("Tunnel IP \(tunnelIp ?? String(localized: "Unknown")) · since \(time.clockTime(since))")
            }
            if let reason = connection.waitReason {
                Text(reason.summary(time: time))
            }
            if let outlook {
                Text("Schedule: \(outlook.summary(time: time))")
            }
            switch connection.status {
            case let .setupRequired(items):
                ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                    Button(item.actionTitle) { ui.settings.show(item.settingsPane) }
                }
            case let .failed(reason):
                Text(reason.guidance(time: time))
                Button(reason.actionTitle) { ui.settings.show(reason.settingsPane) }
            default:
                EmptyView()
            }

            // Always listed; the ones that don't apply now are dimmed.
            Divider()
            Button("Connect") { ui.send(.connect) }
                .disabled(connection.connect != .enabled)
            Button("Disconnect") { ui.send(.disconnect) }
                .disabled(connection.disconnect != .enabled)
            Button("Reconnect Now") { ui.send(.reconnectNow) }
                .disabled(connection.reconnectNow != .enabled)
        }

        Divider()
        Button("Show Log") { ui.settings.show(.log) }
        Button("Settings…") { ui.settings.show() }
            .keyboardShortcut(",")
        Divider()
        Button("Quit FortiVPNManager") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }
}

/// A button drawn as its section says: hidden, enabled or disabled.
struct StateButton: View {
    let title: LocalizedStringKey
    let state: AppInternal.ButtonState
    let action: () -> Void

    init(_ title: LocalizedStringKey, state: AppInternal.ButtonState, action: @escaping () -> Void) {
        self.title = title
        self.state = state
        self.action = action
    }

    var body: some View {
        if state != .hidden {
            Button(title, action: action)
                .disabled(state == .disabled)
        }
    }
}
