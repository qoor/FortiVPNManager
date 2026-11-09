import Foundation
import FortiShared
import ServiceManagement

/// The Helper's registration as a system service (SMAppService daemon) and the app's as a login item
/// (SMAppService main app). Only the installed copy registers, so the system never points at a build
/// somewhere else.
@MainActor
enum ServiceRegistration {
    static let installedAppURL = URL(filePath: "/Applications/FortiVPNManager.app")

    static var isInstalledCopy: Bool {
        Bundle.main.bundleURL.resolvingSymlinksInPath().standardizedFileURL == installedAppURL.standardizedFileURL
    }

    static func helperStatus() -> AppInternal.PortResponse {
        .registration(status: status(of: helper))
    }

    static func registerHelper() -> AppInternal.PortResponse {
        change { try helper.register() }
    }

    static func loginItemStatus() -> AppInternal.PortResponse {
        .registration(status: status(of: .mainApp))
    }

    static func setLoginItem(_ enabled: Bool) -> AppInternal.PortResponse {
        change {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        }
    }

    /// Login Items & Extensions, where the Helper's background activity is approved.
    static func openApprovalSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }

    private static var helper: SMAppService { .daemon(plistName: FortiIdentifiers.helperPlistName) }

    private static func change(_ body: () throws -> Void) -> AppInternal.PortResponse {
        guard isInstalledCopy else { return .registrationFailed(error: .notInstalledCopy) }
        do {
            try body()
            return .registered
        } catch {
            return .registrationFailed(error: .failed(description: error.localizedDescription))
        }
    }

    private static func status(of service: SMAppService) -> AppInternal.RegistrationStatus {
        switch service.status {
        case .notRegistered: .notRegistered
        case .enabled: .enabled
        case .requiresApproval: .requiresApproval
        case .notFound: .notFound
        @unknown default: .notFound
        }
    }
}
