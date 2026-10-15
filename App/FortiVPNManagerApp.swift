import SwiftUI

/// Menu bar app (LSUIElement): the status item is the only scene. The settings window is AppKit
/// (`SettingsWindowController`), and `AppDelegate` owns the services.
@main
struct FortiVPNManagerApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        MenuBarExtra {
            MenuContent(environment: appDelegate.environment, clock: appDelegate.clock, settings: appDelegate.settings)
        } label: {
            MenuBarIcon(connection: appDelegate.environment.connection)
        }
        .menuBarExtraStyle(.menu)
    }
}
