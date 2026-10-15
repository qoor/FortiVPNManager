import Foundation

/// Identifiers frozen at the first SMAppService registration. Changing them later trips launchd
/// launch constraints, so treat every value here as permanent.
public enum FortiIdentifiers {
    public static let appBundleID = "com.qoor.FortiVPNManager"
    public static let helperBundleID = "com.qoor.FortiVPNManager.Helper"
    public static let helperMachService = helperBundleID
    public static let helperPlistName = "com.qoor.FortiVPNManager.Helper.plist"
    public static let logSubsystem = "com.qoor.FortiVPNManager"
}

/// Identifies one build of the app + helper pair. The app refuses to talk to a helper whose
/// BuildID differs and asks it to exit so launchd relaunches the embedded binary.
public struct BuildID: Codable, Hashable, Sendable, CustomStringConvertible {
    public let value: String

    public init(_ value: String) {
        self.value = value
    }

    public var description: String { value }
}

/// One connection attempt. Every helper event and output line carries the attempt it belongs to,
/// so late messages from an aborted attempt can be ignored.
public typealias AttemptID = UUID
