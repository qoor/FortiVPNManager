import AppKit
import SwiftUI

/// Menu bar app (LSUIElement): the status item is the only scene. The settings window is AppKit
/// (`SettingsWindowController`), and `AppDelegate` starts the runtime.
@main
struct FortiVPNManagerApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        MenuBarScene(ui: appDelegate.router.ui)
    }
}

/// Starts the runtime and hands the application lifecycle to the router.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let router: AppRouter
    private let facts = AppInternal.AppFacts.current()

    override init() {
        let inbox = RuntimeInbox()
        // The one hop to the main actor for the runtime's messages; the main queue keeps their order.
        let runtime = Runtime.start { message in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { inbox.router?.receive(message) }
            }
        }
        router = AppRouter(appBuild: facts.appBuild, send: { runtime.send($0) })
        // The runtime sends nothing before the launch message, which `start` sends.
        inbox.router = router
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        router.start(facts)
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        router.lifetime.applicationShouldTerminate()
    }

    /// Opening the app again (Finder, Spotlight) shows the settings window.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        router.ui.settings.show()
        return false
    }
}

/// Where the runtime's messages go once the router exists.
@MainActor
private final class RuntimeInbox {
    var router: AppRouter?
}
