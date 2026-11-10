import Foundation

// The connection's sentences, shared by the menu and the General pane so both describe the same
// state with the same words.

extension AppInternal.ConnectionSection {
    /// One line: "Connected · 1 hr 23 min · Reconnects in about 1 hr 32 min", "Waiting to reconnect (1:12)".
    /// `outlook` is the schedule's, which ends the line of a session without a planned reconnect.
    func statusLine(outlook: AppInternal.ScheduleOutlook?, time: TimeStyle) -> String {
        switch status {
        case .notConnected:
            return String(localized: "Not connected")
        case .setupRequired:
            return String(localized: "Setup required")
        case let .connecting(reconnecting, progress):
            let verb = reconnecting ? String(localized: "Reconnecting") : String(localized: "Connecting")
            return "\(verb) · \(progress.text)"
        case let .connected(since, _, proactiveReconnect):
            var parts = [String(localized: "Connected"), time.duration(time.now.timeIntervalSince(since))]
            if case let .at(at) = proactiveReconnect {
                parts.append(time.plannedReconnect(at))
            } else if outlook == .suspended {
                parts.append(String(localized: "Won't reconnect"))
            } else if case .holdingWhileAway = outlook {
                parts.append(String(localized: "Reconnect on hold"))
            }
            return parts.joined(separator: " · ")
        case .reconnectOnHold:
            return String(localized: "Reconnect on hold")
        case let .waitingToReconnect(until):
            let remaining = until.timeIntervalSince(time.now)
            return remaining > 0
                ? String(localized: "Waiting to reconnect (\(time.countdown(remaining)))")
                : String(localized: "Waiting to reconnect")
        case .waitingForNetwork:
            return String(localized: "Waiting for network")
        case let .disconnecting(then):
            switch then {
            case .idle: return String(localized: "Disconnecting")
            case .reconnect: return String(localized: "Reconnecting · Closing current connection")
            case .sleep: return String(localized: "Disconnecting before sleep")
            case .quit: return String(localized: "Quitting")
            }
        case let .failed(reason):
            return String(localized: "Error: \(reason.summary)")
        }
    }

    /// When the next attempt starts, while one is awaited.
    var nextAttempt: Date? {
        switch status {
        case let .waitingToReconnect(until), let .reconnectOnHold(until): until
        default: nil
        }
    }
}

extension AppInternal.ConnectProgress {
    var text: String {
        switch self {
        case .checkingReadiness: String(localized: "Checking readiness")
        case .connectingToGmail: String(localized: "Connecting to Gmail")
        case .contactingServer: String(localized: "Contacting server")
        case .loggingIn: String(localized: "Logging in")
        case .waitingForOtp: String(localized: "Waiting for OTP email")
        case .verifyingOtp: String(localized: "Verifying OTP")
        case .openingTunnel: String(localized: "Opening tunnel")
        }
    }
}

extension AppInternal.FailureReason {
    /// Short phrase for the status line: "Error: <summary>".
    var summary: String {
        switch self {
        case .vpnLoginRejected: String(localized: "VPN login rejected")
        case let .otpNotReceived(inSpam, _):
            inSpam ? String(localized: "OTP email is in Spam") : String(localized: "OTP email not received")
        case .otpRejected: String(localized: "OTP verification failed")
        case .tunnelSetupFailed: String(localized: "Tunnel setup failed")
        case .tunnelModeRejected: String(localized: "Server rejected tunnel mode")
        case .droppedEarly: String(localized: "Dropped right after connecting")
        case .certificateChangedAfterLogin: String(localized: "Server certificate changed")
        case let .gmailRejected(method):
            switch method {
            case .imap: String(localized: "Check Gmail app password")
            case .gmailApi: String(localized: "Check Google account connection")
            }
        case .tooManyReconnects: String(localized: "Too many reconnects")
        case .network: String(localized: "Network error")
        case let .helper(code): code.summary
        }
    }

    /// One line on what happened, shown with "Open Settings".
    func guidance(time: TimeStyle) -> String {
        switch self {
        case .vpnLoginRejected:
            String(localized: "The server rejected the login. Check the username and password.")
        case .otpNotReceived(inSpam: true, _):
            String(localized: "The OTP email is in Spam. Check your Gmail filter.")
        case let .otpNotReceived(inSpam: false, waitedSeconds):
            String(localized: "No OTP email was found within \(time.spelledOut(seconds: waitedSeconds)).")
        case .otpRejected:
            String(localized: "The server did not accept the OTP.")
        case .tunnelSetupFailed:
            String(localized: "Couldn't open the tunnel after login.")
        case .tunnelModeRejected:
            String(localized: "The gateway doesn't allow SSL VPN tunnel mode.")
        case let .droppedEarly(withinSeconds):
            String(localized: "The connection dropped within \(time.spelledOut(seconds: withinSeconds)).")
        case .certificateChangedAfterLogin:
            String(localized: "The server certificate was rejected after login.")
        case let .gmailRejected(method):
            switch method {
            case .imap:
                String(localized: "Gmail rejected the login. Check the app password and that IMAP is enabled.")
            case .gmailApi:
                String(localized: "Couldn't read Gmail with the Google account. Connect the Google account again, or check the fallback app password.")
            }
        case let .tooManyReconnects(windowSeconds):
            String(localized: "Automatic reconnection stopped after too many sign-in attempts within \(time.spelledOut(seconds: windowSeconds)).")
        case .network:
            String(localized: "Couldn't connect to the VPN server.")
        case .helper(.invalidRequest):
            String(localized: "The connection request was rejected. Check the account details and VPN password. The log shows why.")
        case let .helper(code):
            String(localized: "Helper error: \(code.summary)")
        }
    }

    /// The pane that helps with this failure, opened by the button shown with the guidance.
    var settingsPane: SettingsPane {
        switch self {
        // `.invalidRequest`: an account value or the VPN password the Helper refused (the log shows
        // which).
        case .vpnLoginRejected, .otpNotReceived, .gmailRejected, .helper(.invalidRequest): .account
        case .helper: .advanced
        case .otpRejected, .tunnelSetupFailed, .tunnelModeRejected, .droppedEarly, .certificateChangedAfterLogin,
             .tooManyReconnects, .network: .log
        }
    }

    /// The title of that button, naming where it goes.
    var actionTitle: String {
        switch settingsPane {
        case .account: String(localized: "Open Account Settings…")
        case .advanced: String(localized: "Open Advanced Settings…")
        default: String(localized: "Show Log")
        }
    }
}

extension AppInternal.HelperErrorCode {
    var summary: String {
        switch self {
        case .busy: String(localized: "Helper is busy with another connection")
        case .invalidRequest: String(localized: "Invalid connection request")
        case .clientUnavailable: String(localized: "openfortivpn not found")
        case .notAuthorized: String(localized: "Helper refused the request")
        case .unknownAttempt, .wrongPhase, .internalError: String(localized: "Helper internal error")
        }
    }
}

extension AppInternal.WaitReason {
    /// Why the connection is waiting, shown under the status line.
    func summary(time: TimeStyle) -> String {
        switch self {
        case .network: String(localized: "Network error")
        case .serverEndedSession: String(localized: "Server ended the session")
        case .forcedTermination: String(localized: "Connection was forcibly ended")
        case let .authFailure(reason):
            String(localized: "Authentication failed (\(reason.summary)), retrying once")
        case let .authCooldown(delaySeconds):
            String(localized: "Waiting \(time.spelledOut(seconds: delaySeconds)) after authentication failure")
        case .sleep: String(localized: "Resuming after sleep")
        case .userAway: String(localized: "Outside the scheduled hours, reconnects when you're back")
        }
    }
}

extension AppInternal.SetupItem {
    /// The action offered for the item, one per item so it is clear what to fix and where; it opens
    /// `settingsPane`.
    var actionTitle: String {
        switch self {
        case .accountDetails: String(localized: "Enter Account Details…")
        case .vpnPassword: String(localized: "Enter VPN Password…")
        case let .gmailCredential(method):
            switch method {
            case .imap: String(localized: "Enter Gmail App Password…")
            case .gmailApi: String(localized: "Connect Google Account…")
            }
        case .configFile: String(localized: "Check Settings File Error…")
        case .helperApproval: String(localized: "Register and Approve Helper…")
        case .helperVersion: String(localized: "Check Helper Version…")
        }
    }

    /// The pane where the item is set up.
    var settingsPane: SettingsPane {
        switch self {
        case .accountDetails, .vpnPassword, .gmailCredential: .account
        case .configFile, .helperApproval, .helperVersion: .advanced
        }
    }
}

extension AppInternal.ConnectionIcon {
    var symbolName: String {
        switch self {
        case .connected: "lock.shield.fill"
        case .disconnected: "shield.slash"
        case .inProgress: "shield.lefthalf.filled"
        case .error: "exclamationmark.shield.fill"
        }
    }

    var accessibilityText: String {
        switch self {
        case .connected: String(localized: "Connected")
        case .disconnected: String(localized: "Not connected")
        case .inProgress: String(localized: "Connecting")
        case .error: String(localized: "Error")
        }
    }
}
