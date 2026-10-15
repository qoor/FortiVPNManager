import Foundation
import FortiShared
import Observation
import os
import ServiceManagement

/// "Launch at Login" (DESIGN §5.6, §5.7): reads and writes `SMAppService.mainApp` directly, because the
/// user can change it in System Settings. Only the /Applications copy registers, so the login item
/// never points at a DerivedData build.
@Observable
final class LoginItemService {
    enum LoginItemError: LocalizedError {
        case notInstalledCopy

        var errorDescription: String? {
            String(localized: "Launch at Login can only be set from the app installed in /Applications.")
        }
    }

    private(set) var status: SMAppService.Status
    let isInstalledCopy: Bool

    @ObservationIgnored private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "login-item")

    init(installer: HelperInstaller = HelperInstaller()) {
        isInstalledCopy = installer.isInstalledCopy
        status = SMAppService.mainApp.status
    }

    var isEnabled: Bool { status == .enabled }

    func refresh() {
        let current = SMAppService.mainApp.status
        if current != status { status = current }
    }

    func setEnabled(_ enabled: Bool) throws {
        defer { refresh() }
        guard isInstalledCopy else { throw LoginItemError.notInstalledCopy }
        if enabled {
            try SMAppService.mainApp.register()
        } else {
            try SMAppService.mainApp.unregister()
        }
    }

    /// Registers once by default, on the first launch of the /Applications copy. Later changes
    /// (including turning it off in System Settings) are the user's.
    func registerOnFirstInstalledLaunch(state: StateStore) {
        guard isInstalledCopy, !state.loginItemRegistered else { return }
        do {
            if SMAppService.mainApp.status != .enabled {
                try SMAppService.mainApp.register()
            }
            state.setLoginItemRegistered(true)
            logger.notice("registered login item on first launch")
        } catch {
            logger.error("login item registration failed: \(error.localizedDescription, privacy: .public)")
        }
        refresh()
    }
}
