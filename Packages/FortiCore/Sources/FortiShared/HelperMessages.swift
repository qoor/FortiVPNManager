import Foundation

/// Progress of one openfortivpn run, in the order it normally happens.
public enum Milestone: Int, Codable, Sendable, Comparable {
    case spawned
    case connectedToGateway
    case otpPrompted
    case otpSubmitted
    case authenticated
    case tunnelUp

    public static func < (lhs: Milestone, rhs: Milestone) -> Bool {
        lhs.rawValue < rhs.rawValue
    }
}

/// Everything the helper needs to run openfortivpn once. The helper re-validates every field
/// (see `ConfigRenderer`) and never trusts the app's input.
public struct ConnectRequest: Codable, Sendable, Equatable {
    public var attemptID: AttemptID
    public var host: String
    public var port: Int
    public var username: String
    public var realm: String?
    /// Lowercase hex SHA-256 digests of trusted gateway certificates (`trusted-cert`).
    public var trustedCerts: [String]
    public var password: String

    public init(
        attemptID: AttemptID,
        host: String,
        port: Int,
        username: String,
        realm: String?,
        trustedCerts: [String],
        password: String
    ) {
        self.attemptID = attemptID
        self.host = host
        self.port = port
        self.username = username
        self.realm = realm
        self.trustedCerts = trustedCerts
        self.password = password
    }
}

/// Why the helper stopped (or was asked to stop) a run.
public enum StopReason: String, Codable, Sendable {
    case userDisconnect
    case proactiveReconnect
    /// App-initiated restart other than the proactive timer (manual reconnect, network change, wake rules).
    case reconnect
    case otpTimeout
    /// The Gmail session failed while openfortivpn was waiting for the OTP.
    case otpUnavailable
    case attemptTimeout
    case sleep
    case appQuit
    case clientInvalidated
    case watchdog
    case helperShutdown
}

/// Who ended the run. `.none` means openfortivpn exited on its own (server cut, auth failure, ...).
public enum StopInitiator: Codable, Sendable, Equatable {
    case none
    case helper(StopReason)
}

public enum ProcessExitStatus: Codable, Sendable, Equatable {
    case exited(code: Int32)
    case signaled(signal: Int32)
    case spawnFailed(errno: Int32)
    /// The status could not be collected (e.g. the child was reaped elsewhere).
    case unknown
}

/// Facts the helper recorded about a finished run. The app's reducer classifies the outcome from
/// these facts; exit codes and log text alone are not reliable for openfortivpn 1.24.1.
public struct ExitFacts: Codable, Sendable, Equatable {
    /// `nil` when the process could not be spawned at all.
    public var furthestMilestone: Milestone?
    public var stopInitiator: StopInitiator
    /// Digests printed by openfortivpn in its "Gateway certificate validation failed" block, deduplicated.
    public var certRejectedDigests: [String]
    /// The milestone reached when the first certificate rejection was printed, if any.
    public var certRejectedAt: Milestone?
    /// openfortivpn reported that the gateway refused tunnel mode after authentication
    /// (e.g. SSL VPN tunnel mode removed by a FortiOS upgrade).
    public var tunnelModeRejected: Bool
    /// Seconds between `tunnelUp` and exit, if the tunnel came up.
    public var upDuration: TimeInterval?
    public var exitStatus: ProcessExitStatus

    public init(
        furthestMilestone: Milestone?,
        stopInitiator: StopInitiator,
        certRejectedDigests: [String] = [],
        certRejectedAt: Milestone? = nil,
        tunnelModeRejected: Bool = false,
        upDuration: TimeInterval? = nil,
        exitStatus: ProcessExitStatus
    ) {
        self.furthestMilestone = furthestMilestone
        self.stopInitiator = stopInitiator
        self.certRejectedDigests = certRejectedDigests
        self.certRejectedAt = certRejectedAt
        self.tunnelModeRejected = tunnelModeRejected
        self.upDuration = upDuration
        self.exitStatus = exitStatus
    }
}

/// Event pushed from the helper to the app.
public struct HelperEvent: Codable, Sendable, Equatable {
    public enum Kind: Codable, Sendable, Equatable {
        case milestone(Milestone, tunnelIP: String?)
        case exited(ExitFacts)
    }

    public var attemptID: AttemptID
    public var kind: Kind

    public init(attemptID: AttemptID, kind: Kind) {
        self.attemptID = attemptID
        self.kind = kind
    }
}

public enum OutputSource: String, Codable, Sendable {
    case stdout
    case stderr
    /// Meta lines written by the helper itself (attempt start, "password sent", signals, exit).
    case helper
}

/// One raw line of openfortivpn/pppd output (or a helper meta line), shown verbatim in the Log tab.
public struct OutputLine: Codable, Sendable, Equatable {
    public var date: Date
    public var source: OutputSource
    public var text: String

    public init(date: Date, source: OutputSource, text: String) {
        self.date = date
        self.source = source
        self.text = text
    }
}

/// Snapshot returned by `status()`.
public struct HelperStatus: Codable, Sendable, Equatable {
    public var childAlive: Bool
    public var openfortivpnPath: String?
    public var openfortivpnVersion: String?
    /// Human-readable problem with the openfortivpn binary, if any.
    public var binaryProblem: String?

    public init(
        childAlive: Bool,
        openfortivpnPath: String?,
        openfortivpnVersion: String?,
        binaryProblem: String?
    ) {
        self.childAlive = childAlive
        self.openfortivpnPath = openfortivpnPath
        self.openfortivpnVersion = openfortivpnVersion
        self.binaryProblem = binaryProblem
    }
}

/// Error returned in XPC replies.
public struct HelperError: Error, Codable, Sendable, Equatable {
    public enum Code: String, Codable, Sendable {
        case busy
        case invalidRequest
        case unknownAttempt
        case wrongPhase
        case binaryUnavailable
        case notAuthorized
        case internalError
    }

    public var code: Code
    public var message: String

    public init(_ code: Code, _ message: String) {
        self.code = code
        self.message = message
    }
}
