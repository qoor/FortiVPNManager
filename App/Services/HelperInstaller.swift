import Foundation
import ServiceManagement
import FortiShared

/// Registers the embedded root helper with SMAppService.
/// Only the copy in /Applications may register, so launchd/BTM never points at a DerivedData build.
struct HelperInstaller {
    static let installedAppURL = URL(fileURLWithPath: "/Applications/FortiVPNManager.app")

    private var service: SMAppService { .daemon(plistName: FortiIdentifiers.helperPlistName) }

    var status: SMAppService.Status { service.status }

    var isInstalledCopy: Bool {
        Bundle.main.bundleURL.resolvingSymlinksInPath().standardizedFileURL == Self.installedAppURL.standardizedFileURL
    }

    func register() throws {
        guard isInstalledCopy else { throw InstallerError.notInstalledCopy }
        try service.register()
    }

    enum InstallerError: LocalizedError {
        case notInstalledCopy

        var errorDescription: String? {
            String(localized: "The Helper can only be registered from the app installed in /Applications (make install).")
        }
    }
}
