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
            if let reason = connection.waitReason {
                Text(reason.summary(time: time))
            }
            if let outlook {
                Text("Schedule: \(outlook.summary(time: time))")
            }
            if let attention = connection.attention(time: time) {
                Text(attention.message)
                Button("Open Settings") { ui.settings.show(attention.tab) }
            }

            Divider()
            StateButton("Connect", state: connection.connect) { ui.send(.connect) }
            StateButton("Disconnect", state: connection.disconnect) { ui.send(.disconnect) }
            StateButton("Reconnect Now", state: connection.reconnectNow) { ui.send(.reconnectNow) }
        }

        Divider()
        Button("Settings…") { ui.settings.show() }
            .keyboardShortcut(",")
        Button("Quit") { NSApp.terminate(nil) }
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
