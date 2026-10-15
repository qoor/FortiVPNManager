import AppKit

/// Owns the app's services and the settings window and forwards the application lifecycle to them.
final class AppDelegate: NSObject, NSApplicationDelegate {
    let environment = AppEnvironment()
    let clock = StatusClock()
    let accountNotice = AccountChangeNotice()
    private(set) lazy var settings = SettingsWindowController(
        environment: environment, clock: clock, accountNotice: accountNotice
    )

    func applicationDidFinishLaunching(_ notification: Notification) {
        environment.start()
        let connection = environment.connection
        Task { [clock] in await clock.run(following: connection) }
        Task { [accountNotice] in await accountNotice.run(following: connection) }
        Task { await openSettingsIfNeeded() }
    }

    /// The controller stops the tunnel first and replies once it exited (at most 12 s).
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        environment.applicationShouldTerminate()
    }

    /// Opening the app again (Finder, Spotlight) shows the settings window.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        settings.show()
        return false
    }

    /// First run (DESIGN §5.6): show the imported openfortivpn settings, or open the tab that needs
    /// attention when the first readiness check finds something missing.
    private func openSettingsIfNeeded() async {
        if let outcome = environment.firstLaunchImport {
            settings.accountEditor.presentImport(outcome)
            settings.show(.account)
            return
        }
        let checker = environment.readinessChecker
        while checker.readiness == .unknown {
            await observationChange { _ = checker.readiness }
        }
        if case let .needsSetup(items) = checker.readiness {
            settings.show(items.first?.settingsTab ?? .account)
        }
    }
}
