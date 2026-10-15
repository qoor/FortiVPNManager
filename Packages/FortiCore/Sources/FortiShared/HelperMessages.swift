import Foundation

/// Progress of one openfortivpn run, in the order it normally happens.
public enum Milestone: Int, Codable, Sendable, Comparable, CaseIterable {
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
        realm: String? = nil,
        trustedCerts: [String] = [],
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

/// Phase whose watchdog fired.
public enum WatchdogPhase: String, Codable, Sendable {
    case spawnToGateway
    case gatewayToPromptOrAuth
    case otpPromptToSubmit
    case otpSubmitToAuth
    case authToTunnel
    case terminate
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
    public var watchdogFired: WatchdogPhase?
    /// Digests printed by openfortivpn in its "Gateway certificate validation failed" block, deduplicated.
    public var certRejectedDigests: [String]
    /// The milestone reached when the first certificate rejection was printed, if any.
    public var certRejectedAt: Milestone?
    public var authFailLineSeen: Bool
    /// openfortivpn reported that the gateway refused tunnel mode after authentication
    /// (e.g. SSL VPN tunnel mode removed by a FortiOS upgrade).
    public var tunnelModeRejected: Bool
    /// Seconds between `tunnelUp` and exit, if the tunnel came up.
    public var upDuration: TimeInterval?
    public var exitStatus: ProcessExitStatus
    public var routeRepairPerformed: Bool
    /// Sequence number of the last output line sent before this event.
    public var lastOutputSeq: UInt64

    public init(
        furthestMilestone: Milestone?,
        stopInitiator: StopInitiator,
        watchdogFired: WatchdogPhase? = nil,
        certRejectedDigests: [String] = [],
        certRejectedAt: Milestone? = nil,
        authFailLineSeen: Bool = false,
        tunnelModeRejected: Bool = false,
        upDuration: TimeInterval? = nil,
        exitStatus: ProcessExitStatus,
        routeRepairPerformed: Bool = false,
        lastOutputSeq: UInt64 = 0
    ) {
        self.furthestMilestone = furthestMilestone
        self.stopInitiator = stopInitiator
        self.watchdogFired = watchdogFired
        self.certRejectedDigests = certRejectedDigests
        self.certRejectedAt = certRejectedAt
        self.authFailLineSeen = authFailLineSeen
        self.tunnelModeRejected = tunnelModeRejected
        self.upDuration = upDuration
        self.exitStatus = exitStatus
        self.routeRepairPerformed = routeRepairPerformed
        self.lastOutputSeq = lastOutputSeq
    }
}

/// Event pushed from the helper to the app.
public struct HelperEvent: Codable, Sendable, Equatable {
    public enum Kind: Codable, Sendable, Equatable {
        case milestone(Milestone, tunnelIP: String?)
        case exited(ExitFacts)
    }

    public var attemptID: AttemptID
    public var date: Date
    public var kind: Kind

    public init(attemptID: AttemptID, date: Date, kind: Kind) {
        self.attemptID = attemptID
        self.date = date
        self.kind = kind
    }
}

public enum OutputSource: String, Codable, Sendable {
    case stdout
    case stderr
    /// Meta lines written by the helper itself (attempt start, "password written", signals, exit).
    case helper
}

/// One raw line of openfortivpn/pppd output (or a helper meta line), shown verbatim in the Log tab.
public struct OutputLine: Codable, Sendable, Equatable {
    public var seq: UInt64
    public var date: Date
    public var attemptID: AttemptID?
    public var source: OutputSource
    public var text: String
    /// True when the line was flushed before its terminating newline arrived (e.g. a prompt).
    public var isPartial: Bool

    public init(seq: UInt64, date: Date, attemptID: AttemptID?, source: OutputSource, text: String, isPartial: Bool = false) {
        self.seq = seq
        self.date = date
        self.attemptID = attemptID
        self.source = source
        self.text = text
        self.isPartial = isPartial
    }
}

/// Snapshot returned by `status()`.
public struct HelperStatus: Codable, Sendable, Equatable {
    public var buildID: BuildID
    public var activeAttemptID: AttemptID?
    public var childAlive: Bool
    public var openfortivpnPath: String?
    public var openfortivpnVersion: String?
    /// Human-readable problem with the openfortivpn binary, if any.
    public var binaryProblem: String?

    public init(
        buildID: BuildID,
        activeAttemptID: AttemptID?,
        childAlive: Bool,
        openfortivpnPath: String?,
        openfortivpnVersion: String?,
        binaryProblem: String?
    ) {
        self.buildID = buildID
        self.activeAttemptID = activeAttemptID
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
        case spawnFailed
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
