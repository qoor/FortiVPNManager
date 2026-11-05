import Foundation
import FortiShared

/// What the user wants, tracked separately from what is currently happening.
public enum DesiredState: Sendable, Equatable {
    case connected
    case disconnected
}

/// The `connection.*` options of `config.yaml` that the reducer acts on.
public struct ConnectionSettings: Sendable, Equatable {
    public var sessionMaxMinutes: Int
    public var proactiveReconnectEnabled: Bool
    public var proactiveMarginMinutes: Int
    public var reconnectAfterSleepEnabled: Bool
    public var minSleepMinutes: Int
    /// `connection.schedule`, or `nil` when the schedule is off.
    public var schedule: ConnectionSchedule?

    public init(
        sessionMaxMinutes: Int,
        proactiveReconnectEnabled: Bool,
        proactiveMarginMinutes: Int,
        reconnectAfterSleepEnabled: Bool,
        minSleepMinutes: Int,
        schedule: ConnectionSchedule?
    ) {
        self.sessionMaxMinutes = sessionMaxMinutes
        self.proactiveReconnectEnabled = proactiveReconnectEnabled
        self.proactiveMarginMinutes = proactiveMarginMinutes
        self.reconnectAfterSleepEnabled = reconnectAfterSleepEnabled
        self.minSleepMinutes = minSleepMinutes
        self.schedule = schedule
    }

    /// `true` when a change from `old` requires recomputing `plannedReconnectAt`.
    func affectsPlannedReconnect(comparedTo old: ConnectionSettings) -> Bool {
        sessionMaxMinutes != old.sessionMaxMinutes
            || proactiveReconnectEnabled != old.proactiveReconnectEnabled
            || proactiveMarginMinutes != old.proactiveMarginMinutes
    }
}

/// The `vpn.*` section of `config.yaml`: the gateway account plus the auto-managed certificate pins.
///
/// The reducer owns the in-memory copy of `trustedCerts` so that a digest learned from a rejection is
/// carried by the very next connect request even before `config.yaml` has been rewritten.
public struct VPNProfile: Sendable, Equatable {
    public var host: String
    public var port: Int
    public var username: String
    public var realm: String?
    /// Lowercase hex SHA-256 digests, newest first (`vpn.trusted_certs`).
    public var trustedCerts: [String]

    public init(host: String, port: Int, username: String, realm: String? = nil, trustedCerts: [String] = []) {
        self.host = host
        self.port = port
        self.username = username
        self.realm = realm
        self.trustedCerts = trustedCerts
    }

    /// Key for the failure ledger (`username@host`).
    public var profileKey: String { "\(username)@\(host)" }

    /// Builds the XPC request for one attempt.
    public func connectRequest(attemptID: AttemptID, password: String) -> ConnectRequest {
        ConnectRequest(
            attemptID: attemptID,
            host: host,
            port: port,
            username: username,
            realm: realm,
            trustedCerts: trustedCerts,
            password: password
        )
    }

    /// `true` when `other` points at a different gateway (host or port), which invalidates the pins.
    func isDifferentGateway(from other: VPNProfile) -> Bool {
        host != other.host || port != other.port
    }

    /// `true` when the account identity changed (host, port or username).
    func isDifferentAccount(from other: VPNProfile) -> Bool {
        isDifferentGateway(from: other) || username != other.username
    }
}

/// Something that must be fixed in the settings before a connection can be attempted.
public enum SetupItem: Sendable, Hashable {
    /// `config.yaml` failed to load or validate.
    case invalidConfig
    /// Host, username or Gmail address is empty, or host/username/realm break the helper's rules
    /// (`ConfigRenderer.validateHost` etc.), e.g. a host typed as `https://vpn.example.com`.
    case accountIncomplete
    /// No Keychain item for `username@host`.
    case vpnPassword
    /// No usable Gmail credential: no app password for the Gmail address and, with
    /// `otp.method: gmail_api`, no Google authorization (refresh token + client secret) either.
    case gmailAppPassword
    /// The helper daemon is not `.enabled` (not registered or awaiting approval).
    case helperNotEnabled
    /// The running helper reports a different `BuildID`.
    case helperVersionMismatch
}

/// Result of the app's readiness check.
public enum Readiness: Sendable, Equatable {
    /// Not checked yet (launch, or the account just changed).
    case unknown
    case ready
    case needsSetup([SetupItem])

    /// `.ready` when nothing is missing, otherwise `.needsSetup` with the items as given.
    public init(missing: [SetupItem]) {
        self = missing.isEmpty ? .ready : .needsSetup(missing)
    }
}

/// Raw facts the app gathers for a readiness check. Pure mapping to `Readiness`.
public struct ReadinessInputs: Sendable, Equatable {
    public var configValid: Bool
    public var accountComplete: Bool
    public var hasVPNPassword: Bool
    /// An IMAP app password exists, or `otp.method` is `gmail_api` and its Google authorization is stored.
    public var hasGmailAppPassword: Bool
    public var helperEnabled: Bool
    public var helperBuildMatches: Bool

    public init(
        configValid: Bool,
        accountComplete: Bool,
        hasVPNPassword: Bool,
        hasGmailAppPassword: Bool,
        helperEnabled: Bool,
        helperBuildMatches: Bool
    ) {
        self.configValid = configValid
        self.accountComplete = accountComplete
        self.hasVPNPassword = hasVPNPassword
        self.hasGmailAppPassword = hasGmailAppPassword
        self.helperEnabled = helperEnabled
        self.helperBuildMatches = helperBuildMatches
    }

    public var readiness: Readiness {
        var missing: [SetupItem] = []
        if !configValid { missing.append(.invalidConfig) }
        if !accountComplete { missing.append(.accountIncomplete) }
        if !hasVPNPassword { missing.append(.vpnPassword) }
        if !hasGmailAppPassword { missing.append(.gmailAppPassword) }
        if !helperEnabled {
            missing.append(.helperNotEnabled)
        } else if !helperBuildMatches {
            missing.append(.helperVersionMismatch)
        }
        return Readiness(missing: missing)
    }
}

/// The physical route to the gateway: result of `route -n get <gatewayIP>`, excluding ppp*/utun*/other.
public struct Underlay: Sendable, Equatable {
    public var interfaceName: String
    public var sourceAddress: String?

    public init(interfaceName: String, sourceAddress: String? = nil) {
        self.interfaceName = interfaceName
        self.sourceAddress = sourceAddress
    }
}

/// Latest `NWPathMonitor` state plus the underlay the app derived from it.
public struct PathSnapshot: Sendable, Equatable {
    public var isSatisfied: Bool
    public var underlay: Underlay?

    public init(isSatisfied: Bool, underlay: Underlay? = nil) {
        self.isSatisfied = isSatisfied
        self.underlay = underlay
    }
}

/// What the app observed once the network settled after a wake (or after `RetryPolicy.wakeSettleTimeout`
/// without settling).
public struct WakeObservation: Sendable, Equatable {
    public var processAlive: Bool
    public var pppUp: Bool
    public var underlay: Underlay?
    /// `false` when the path and Router did not come up within `RetryPolicy.wakeSettleTimeout`.
    public var pathSatisfied: Bool

    public init(processAlive: Bool, pppUp: Bool, underlay: Underlay?, pathSatisfied: Bool) {
        self.processAlive = processAlive
        self.pppUp = pppUp
        self.underlay = underlay
        self.pathSatisfied = pathSatisfied
    }
}

/// How a Gmail OTP session (IMAP or Gmail API) failed.
public enum OTPSessionFailure: String, Sendable, Equatable {
    /// Login rejected: the Gmail app password is wrong or revoked, or the Google authorization was
    /// revoked or refused (Gmail API).
    case authentication
    /// Connection, TLS or protocol trouble.
    case network
}

/// Why the connection gave up or is waiting.
public enum ConnectionFailureReason: Sendable, Equatable, Codable {
    /// Rejected after reaching the gateway and before the OTP prompt (wrong password, locked account).
    case authenticationRejected
    /// No OTP message matched in time. `inJunk` is set when the message was found in the spam folder.
    case otpNotReceived(inJunk: Bool)
    /// The OTP was submitted but authentication did not complete.
    case otpRejected
    /// Authenticated, but the tunnel never came up.
    case tunnelSetupFailed
    /// The gateway refused tunnel mode after login (openfortivpn: "make sure tunnel mode is allowed").
    case tunnelModeRejected
    /// The tunnel came up but dropped before `RetryPolicy.stableSessionDuration`.
    case sessionDroppedEarly
    /// The gateway certificate was rejected after the OTP had been used.
    case certificateRejectedAfterLogin
    /// Gmail rejected the app password or the Google authorization.
    case gmailAppPassword
    /// Too many authentication attempts within `RetryPolicy.authAttemptWindow`.
    case tooManyReconnects
    /// The gateway could not be reached (not counted as an authentication failure).
    case network
    /// The helper refused to start openfortivpn.
    case helper(HelperError.Code)
}

/// Why the connection is waiting before the next attempt.
public enum RetryReason: Sendable, Equatable {
    /// Network error backoff.
    case network
    /// The server ended a stable session.
    case serverSessionEnded
    /// One authentication failure was counted; one more try after the cooldown.
    case authFailure(ConnectionFailureReason)
    /// A manual or automatic attempt waits out the cooldown after the last authentication failure.
    case authCooldown
    /// An attempt was interrupted by sleep and resumes after wake.
    case sleep
    /// Outside the scheduled hours with the user away: the reconnect waits until the user returns
    /// (`until` is when it is given up).
    case userAway
}

/// Why an attempt was started.
public enum AttemptReason: Sendable, Equatable {
    /// Auto-connect at launch.
    case launch
    /// The user chose "Connect".
    case user
    /// The user chose "Reconnect Now".
    case manualReconnect
    /// Setup became complete while a connection was wanted.
    case setupCompleted
    /// Immediate retry after the gateway certificate was learned (once per chain).
    case certificateRetry
    /// Scheduled retry after a wait.
    case retry(RetryReason)
    /// Planned break-before-make reconnect before the server's session limit.
    case proactive
    /// The underlay interface or address changed while connected.
    case networkChanged
    /// openfortivpn or its ppp interface was gone after a wake.
    case tunnelLost
    /// Resuming after sleep, or `reconnect_after_sleep` applied.
    case afterSleep
    /// The scheduled hours started.
    case scheduled

    /// Manual attempts bypass the rolling attempt cap (the user's action resets the ledger).
    var isManual: Bool {
        switch self {
        case .user, .manualReconnect: true
        default: false
        }
    }

    /// Reconnects the app starts on its own after a connection was wanted; outside the scheduled
    /// hours they wait for, or give up on, an away user. Launch and the schedule start are gated
    /// by the schedule itself, and a certificate retry continues an attempt already running.
    var isAutomaticReconnect: Bool {
        switch self {
        case .retry, .proactive, .networkChanged, .tunnelLost, .afterSleep, .setupCompleted: true
        case .launch, .user, .manualReconnect, .certificateRetry, .scheduled: false
        }
    }
}

/// Timers owned by the reducer. The app keeps at most one pending timer per ID; scheduling an ID again
/// replaces the previous one. A fire is only honoured when its generation is the current one.
public enum ConnectionTimer: Sendable, Equatable, Hashable, CaseIterable {
    case attemptDeadline
    case otpDeadline
    /// End of `waitingRetry`.
    case retry
    /// `plannedReconnectAt` while connected.
    case proactiveReconnect
    case quitDeadline
    /// Next start or end of the scheduled hours.
    case schedule
    /// When the user will have been away for `schedule.awayMinutes`.
    case awayLimit
}
