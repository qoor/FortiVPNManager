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

    func unregister() async throws {
        try await service.unregister()
    }

    enum InstallerError: LocalizedError {
        case notInstalledCopy

        var errorDescription: String? {
            String(localized: "The Helper can only be registered from the app installed in /Applications (make install).")
        }
    }
}

extension SMAppService.Status {
    var label: String {
        switch self {
        case .notRegistered: String(localized: "Not registered")
        case .enabled: String(localized: "Allowed to run")
        case .requiresApproval: String(localized: "Needs approval")
        case .notFound: String(localized: "Not found")
        @unknown default: String(localized: "Unknown")
        }
    }
}
