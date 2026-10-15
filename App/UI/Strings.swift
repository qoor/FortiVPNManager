import Foundation
import FortiConfig
import FortiCore
import FortiShared
import ServiceManagement

// Every user-facing phrase derived from connection state lives here (DESIGN §5.6), so the menu and
// the settings window always describe the same state with the same words. Phrases about Gmail
// credentials depend on `otp.method`: an app password for IMAP, a Google sign-in for the Gmail API.

extension ConnectionFailureReason {
    /// Short phrase for the status line: "Error: <summary>".
    func summary(otpMethod: AppConfig.OTP.Method) -> String {
        switch self {
        case .authenticationRejected: String(localized: "VPN login rejected")
        case let .otpNotReceived(inJunk):
            inJunk ? String(localized: "OTP email is in Spam") : String(localized: "OTP email not received")
        case .otpRejected: String(localized: "OTP verification failed")
        case .tunnelSetupFailed: String(localized: "Tunnel setup failed")
        case .tunnelModeRejected: String(localized: "Server rejected tunnel mode")
        case .sessionDroppedEarly: String(localized: "Dropped right after connecting")
        case .certificateRejectedAfterLogin: String(localized: "Server certificate changed")
        case .gmailAppPassword:
            switch otpMethod {
            case .imap: String(localized: "Check Gmail app password")
            case .gmailAPI: String(localized: "Check Google account connection")
            }
        case .tooManyReconnects: String(localized: "Too many reconnects")
        case .network: String(localized: "Network error")
        case let .helper(code): code.summary
        }
    }

    /// One line on what happened, shown with "Open Settings" (which opens the tab to fix it).
    func guidance(otpMethod: AppConfig.OTP.Method) -> String {
        switch self {
        case .authenticationRejected:
            String(localized: "The server rejected the login. Check the username and password.")
        case .otpNotReceived(inJunk: true):
            String(localized: "The OTP email is in Spam. Check your Gmail filter.")
        case .otpNotReceived(inJunk: false):
            String(localized: "No OTP email was found within 50 seconds.")
        case .otpRejected:
            String(localized: "The server did not accept the OTP.")
        case .tunnelSetupFailed:
            String(localized: "Couldn't open the tunnel after login.")
        case .tunnelModeRejected:
            String(localized: "The gateway doesn't allow SSL VPN tunnel mode.")
        case .sessionDroppedEarly:
            String(localized: "The connection dropped within 2 minutes.")
        case .certificateRejectedAfterLogin:
            String(localized: "The server certificate was rejected after login.")
        case .gmailAppPassword:
            switch otpMethod {
            case .imap:
                String(localized: "Gmail rejected the login. Check the app password and that IMAP is enabled.")
            case .gmailAPI:
                // The router falls back to IMAP when an app password is stored, so either may have failed.
                String(localized: "Couldn't read Gmail with the Google account. Connect the Google account again, or check the fallback app password.")
            }
        case .tooManyReconnects:
            String(localized: "Automatic reconnection stopped after too many sign-in attempts within an hour.")
        case .network:
            String(localized: "Couldn't connect to the VPN server.")
        case .helper(.invalidRequest):
            String(localized: "The connection request was rejected. Check the account details and VPN password. The Log tab shows why.")
        case let .helper(code):
            String(localized: "Helper error: \(code.summary)")
        }
    }
}

extension HelperError.Code {
    var summary: String {
        switch self {
        case .busy: String(localized: "Helper is busy with another connection")
        case .invalidRequest: String(localized: "Invalid connection request")
        case .binaryUnavailable: String(localized: "openfortivpn not found")
        case .spawnFailed: String(localized: "Couldn't start openfortivpn")
        case .notAuthorized: String(localized: "Helper refused the request")
        case .unknownAttempt, .wrongPhase, .internalError: String(localized: "Helper internal error")
        }
    }
}

extension RetryReason {
    /// Why the connection is waiting, shown under the status line.
    func summary(otpMethod: AppConfig.OTP.Method) -> String {
        switch self {
        case .network: String(localized: "Network error")
        case .serverSessionEnded: String(localized: "Server ended the session")
        case let .authFailure(reason):
            String(localized: "Authentication failed (\(reason.summary(otpMethod: otpMethod))), retrying once")
        case .authCooldown: String(localized: "Waiting 90 seconds after authentication failure")
        case .sleep: String(localized: "Resuming after sleep")
        }
    }
}

extension SetupItem {
    /// `.gmailAppPassword` means no usable Gmail credential: no app password and, with the Gmail API
    /// method, no Google sign-in either.
    func summary(otpMethod: AppConfig.OTP.Method) -> String {
        switch self {
        case .invalidConfig: String(localized: "Settings file error")
        case .accountIncomplete: String(localized: "Account details")
        case .vpnPassword: String(localized: "VPN password")
        case .gmailAppPassword:
            switch otpMethod {
            case .imap: String(localized: "Gmail app password")
            case .gmailAPI: String(localized: "Google account connection")
            }
        case .helperNotEnabled: String(localized: "Helper registration and approval")
        case .helperVersionMismatch: String(localized: "Helper version mismatch")
        }
    }

    /// The tab where the item is fixed.
    var settingsTab: SettingsTab {
        switch self {
        case .accountIncomplete, .vpnPassword, .gmailAppPassword: .account
        case .invalidConfig, .helperNotEnabled, .helperVersionMismatch: .connection
        }
    }
}

extension ConnectionAttention {
    /// The one-line reason next to "Open Settings".
    func message(otpMethod: AppConfig.OTP.Method) -> String {
        switch self {
        case let .needsSetup(items):
            String(localized: "Required: \(items.map { $0.summary(otpMethod: otpMethod) }.joined(separator: ", "))")
        case let .failed(reason): reason.guidance(otpMethod: otpMethod)
        }
    }

    /// The tab "Open Settings" opens.
    var settingsTab: SettingsTab {
        switch self {
        case let .needsSetup(items):
            items.first?.settingsTab ?? .account
        case let .failed(reason):
            switch reason {
            // `.invalidRequest`: an account value or the VPN password the helper refused, or a
            // Keychain read failure (the reason is in the Log tab).
            case .authenticationRejected, .otpNotReceived, .gmailAppPassword, .helper(.invalidRequest): .account
            case .helper: .connection
            case .otpRejected, .tunnelSetupFailed, .tunnelModeRejected, .sessionDroppedEarly,
                 .certificateRejectedAfterLogin, .tooManyReconnects, .network: .log
            }
        }
    }
}

extension AttemptContext {
    /// "Connecting" for a fresh connection, "Reconnecting" when replacing or retrying one.
    var verb: String {
        switch reason {
        case .launch, .user, .setupCompleted, .certificateRetry: String(localized: "Connecting")
        case .manualReconnect, .retry, .proactive, .networkChanged, .tunnelLost, .afterSleep: String(localized: "Reconnecting")
        }
    }

    /// Current step of §6.1.
    var progress: String {
        switch stage {
        case .checkingReadiness: return String(localized: "Checking readiness")
        case .preparingOTP: return String(localized: "Connecting to Gmail")
        case .tunnel:
            switch milestone {
            case nil, .spawned?: return String(localized: "Contacting server")
            case .connectedToGateway?: return String(localized: "Logging in")
            case .otpPrompted?: return String(localized: "Waiting for OTP email")
            case .otpSubmitted?: return String(localized: "Verifying OTP")
            case .authenticated?, .tunnelUp?: return String(localized: "Opening tunnel")
            }
        }
    }
}

/// Status texts for the menu and the Connection tab.
enum StatusText {
    /// One line, e.g. "Connected · 1 hr 23 min · Reconnects in about 1 hr 32 min", "Waiting to reconnect (1:12)".
    static func line(_ inputs: ConnectionStatusInputs, now: Date, otpMethod: AppConfig.OTP.Method) -> String {
        switch inputs.state {
        case .idle:
            return String(localized: "Not connected")
        case .needsSetup:
            return String(localized: "Setup required")
        case let .connecting(context):
            return "\(context.verb) · \(context.progress)"
        case let .connected(context):
            var parts = [String(localized: "Connected"), duration(now.timeIntervalSince(context.since))]
            if let planned = context.plannedReconnectAt {
                parts.append(plannedReconnect(planned, now: now))
            }
            return parts.joined(separator: " · ")
        case let .waitingRetry(wait):
            let remaining = wait.until.timeIntervalSince(now)
            return remaining > 0
                ? String(localized: "Waiting to reconnect (\(countdown(remaining)))")
                : String(localized: "Waiting to reconnect")
        case .waitingNetwork:
            return String(localized: "Waiting for network")
        case let .disconnecting(context):
            switch context.after {
            case .idle: return String(localized: "Disconnecting")
            case .reconnect: return String(localized: "Reconnecting · Closing current connection")
            case .resumeAfterSleep: return String(localized: "Disconnecting before sleep")
            case .terminate: return String(localized: "Quitting")
            }
        case let .failed(reason):
            return String(localized: "Error: \(reason.summary(otpMethod: otpMethod))")
        }
    }

    /// Why the connection is waiting, if it is (shown under the status line).
    static func waitReason(_ state: ConnectionState, otpMethod: AppConfig.OTP.Method) -> String? {
        switch state {
        case let .waitingRetry(wait): wait.reason.summary(otpMethod: otpMethod)
        case let .waitingNetwork(wait) where wait.reason != .network: wait.reason.summary(otpMethod: otpMethod)
        default: nil
        }
    }

    /// "Reconnects in about 1 hr 32 min", or "Reconnects soon" within the last minute.
    static func plannedReconnect(_ date: Date, now: Date) -> String {
        let remaining = date.timeIntervalSince(now)
        return remaining < 60
            ? String(localized: "Reconnects soon")
            : String(localized: "Reconnects in about \(duration(remaining))")
    }

    /// Whole minutes in the current locale: "1 hr 23 min", "45 min", "2 hr", or "less than a minute".
    static func duration(_ interval: TimeInterval) -> String {
        let minutes = Int((interval / 60).rounded(.down))
        guard minutes >= 1 else { return String(localized: "less than a minute") }
        return Duration.seconds(minutes * 60)
            .formatted(.units(allowed: [.hours, .minutes], width: .condensedAbbreviated))
    }

    /// "1:12" (minutes:seconds, rounded up).
    static func countdown(_ interval: TimeInterval) -> String {
        let seconds = max(0, Int(interval.rounded(.up)))
        let (minutes, rest) = seconds.quotientAndRemainder(dividingBy: 60)
        return rest < 10 ? "\(minutes):0\(rest)" : "\(minutes):\(rest)"
    }

    /// "2:03 PM", with the date when it is not today ("Oct 14, 2026 at 11:58 PM"), in the current locale.
    static func clockTime(_ date: Date, now: Date) -> String {
        var style = Date.FormatStyle(date: .omitted, time: .shortened)
        if !Calendar.current.isDate(date, inSameDayAs: now) {
            style = Date.FormatStyle(date: .abbreviated, time: .shortened)
        }
        return date.formatted(style)
    }
}

/// What the Connection tab says about the helper (DESIGN §5.6: not registered / needs approval /
/// running / version mismatch).
enum HelperDisplayState: Equatable {
    case checking
    case notRegistered
    case requiresApproval
    case running(BuildID)
    case versionMismatch(helper: BuildID, app: BuildID)
    case unreachable(String)

    init(health: HelperHealth?, appBuild: BuildID) {
        guard let health else {
            self = .checking
            return
        }
        switch health.serviceStatus {
        case .enabled:
            if let build = health.build {
                self = health.buildMatches ? .running(build) : .versionMismatch(helper: build, app: appBuild)
            } else {
                self = .unreachable(health.errorMessage ?? String(localized: "No response"))
            }
        case .requiresApproval:
            self = .requiresApproval
        default:
            self = .notRegistered
        }
    }

    var label: String {
        switch self {
        case .checking: String(localized: "Checking…")
        case .notRegistered: String(localized: "Not registered")
        case .requiresApproval: String(localized: "Needs approval")
        case let .running(build): String(localized: "Running (build \(build.description))")
        case let .versionMismatch(helper, app):
            String(localized: "Version mismatch (Helper \(helper.description), app \(app.description))")
        case let .unreachable(message): String(localized: "No response: \(message)")
        }
    }

}
