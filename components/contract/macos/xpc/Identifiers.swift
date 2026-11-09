import Foundation

/// Identifiers frozen at the first SMAppService registration. The registration, its approval in
/// System Settings and the XPC code-signing requirements are tied to them, so treat every value
/// here as permanent.
enum FortiIdentifiers {
    static let appBundleID = "com.qoor.FortiVPNManager"
    static let helperBundleID = "com.qoor.FortiVPNManager.Helper"
    static let helperMachService = helperBundleID
    static let helperPlistName = "com.qoor.FortiVPNManager.Helper.plist"
    static let logSubsystem = "com.qoor.FortiVPNManager"
}

/// Identifies one build of the app + helper pair. The app refuses to talk to a helper whose
/// BuildID differs and asks it to exit so launchd relaunches the embedded binary.
struct BuildID: Codable, Hashable, Sendable, CustomStringConvertible {
    let value: String

    init(_ value: String) {
        self.value = value
    }

    var description: String { value }
}
