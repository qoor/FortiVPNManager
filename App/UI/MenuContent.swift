import FortiCore
import SwiftUI

/// The menu bar icon (DESIGN §5.6): connected / disconnected / in progress / error.
struct MenuBarIcon: View {
    let connection: ConnectionController

    var body: some View {
        let icon = connection.icon
        Image(systemName: icon.symbolName)
            .accessibilityLabel("FortiVPNManager: \(icon.accessibilityText)")
    }
}

extension ConnectionIcon {
    var symbolName: String {
        switch self {
        case .connected: "lock.shield.fill"
        case .disconnected: "shield.slash"
        case .inProgress: "shield.lefthalf.filled"
        case .error: "exclamationmark.shield.fill"
        }
    }

    var accessibilityText: String {
        switch self {
        case .connected: String(localized: "Connected")
        case .disconnected: String(localized: "Not connected")
        case .inProgress: String(localized: "Connecting")
        case .error: String(localized: "Error")
        }
    }
}

/// The menu (DESIGN §5.6): status, the schedule's next step, the reason and "Open Settings" when
/// attention is needed, the connection actions, "Settings…" and "Quit".
struct MenuContent: View {
    let environment: AppEnvironment
    let clock: StatusClock
    let settings: SettingsWindowController

    private var connection: ConnectionController { environment.connection }

    var body: some View {
        let otpMethod = environment.configStore.otpMethod
        Text(StatusText.line(connection.statusInputs, now: clock.now, otpMethod: otpMethod))
        if let reason = StatusText.waitReason(connection.model.state, otpMethod: otpMethod) {
            Text(reason)
        }
        if let outlook = connection.scheduleOutlook {
            Text("Schedule: \(outlook.summary(now: clock.now))")
        }
        if let attention = connection.attention {
            Text(attention.message(otpMethod: otpMethod))
            Button("Open Settings") { settings.show(attention.settingsTab) }
        }

        Divider()
        let actions = connection.menuActions
        if actions.showsConnect {
            Button("Connect") { connection.connect() }
        }
        if actions.showsDisconnect {
            Button("Disconnect") { connection.disconnect() }
        }
        Button("Reconnect Now") { connection.reconnectNow() }
            .disabled(!actions.reconnectNowEnabled)

        Divider()
        Button("Settings…") { settings.show() }
            .keyboardShortcut(",")
        Button("Quit") { connection.quit() }
            .keyboardShortcut("q")
    }
}
