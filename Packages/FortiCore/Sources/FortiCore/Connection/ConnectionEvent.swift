import Foundation
import FortiShared

/// Inputs to `ConnectionReducer`. Events tagged with an `AttemptID` (or a timer generation) that is not
/// the current one are ignored.
public enum ConnectionEvent: Sendable, Equatable {
    /// Once at launch. `restoredLedger` comes from `state.json`; it is discarded if it belongs to another profile.
    case appLaunched(autoConnect: Bool, restoredLedger: FailureLedger?)
    /// Answer to `.checkReadiness`, or an unsolicited re-check (app activation, settings window, config change).
    case readinessChanged(Readiness)
    /// `vpn.*` in `config.yaml` changed (settings window or hand edit).
    case profileChanged(VPNProfile)
    /// `connection.*` in `config.yaml` changed.
    case settingsChanged(ConnectionSettings)
    case userConnect
    case userDisconnect
    case userReconnectNow
    case quitRequested
    /// The Gmail session logged in, took its snapshot and is idling.
    case otpSessionReady(AttemptID)
    case otpSessionFailed(AttemptID, OTPSessionFailure)
    case otpCodeFound(AttemptID, code: String)
    /// After an OTP deadline: the message was found in the spam folder.
    case otpFoundInJunk(AttemptID)
    case helperEvent(HelperEvent)
    /// The helper's `connect` reply was an error; no `.exited` will follow for this attempt.
    case helperConnectRejected(AttemptID, HelperError)
    /// The XPC connection to the helper was interrupted or invalidated; no `.exited` will follow.
    case helperConnectionLost
    case timerFired(ConnectionTimer, generation: UInt64)
    case willSleep
    case didWake(sleptFor: TimeInterval)
    /// Sent once after every `didWake`, when the path and Router settled or after 30 s.
    case wakeSettled(WakeObservation)
    case pathChanged(PathSnapshot)
    /// The user left (screen locked, display or system asleep) or returned. Sent on changes only;
    /// `away: true` comes before `.willSleep`.
    case presenceChanged(away: Bool)
    /// The time zone (or another calendar setting) changed; schedule times are re-read in it.
    case calendarChanged(Calendar)
}

/// Outputs of `ConnectionReducer`, executed by the app in order.
public enum ConnectionEffect: Sendable, Equatable {
    /// Run the readiness check (§5.4) and answer with `.readinessChanged`.
    case checkReadiness
    /// Start a Gmail session for this attempt (login, snapshot, sweep, IDLE); answer with
    /// `.otpSessionReady` or `.otpSessionFailed`.
    case startOTPSession(AttemptID)
    /// The OTP prompt appeared; look for codes in messages received since `promptedAt − 5 s`.
    case beginOTPSearch(AttemptID, promptedAt: Date)
    /// Close the Gmail session. With `checkJunk`, search the spam folder once first and report
    /// `.otpFoundInJunk` if the OTP message is there.
    case closeOTPSession(AttemptID, checkJunk: Bool)
    /// `connect` with `profile.connectRequest(attemptID:password:)`.
    case startTunnel(AttemptID, VPNProfile)
    /// `submitOTP`, then move the matched messages to Trash and log out.
    case submitOTP(AttemptID, code: String)
    case stopTunnel(AttemptID, StopReason)
    /// Write `vpn.trusted_certs` with exactly this list.
    case persistTrustedCerts([String])
    /// Write the ledger to `state.json`.
    case persistLedger(FailureLedger)
    /// Replace any pending timer with this ID; fire `.timerFired(id, generation:)` at the wall-clock date.
    case scheduleTimer(ConnectionTimer, at: Date, generation: UInt64)
    case cancelTimer(ConnectionTimer)
    /// Hold or release `beginActivity(.userInitiatedAllowingIdleSystemSleep)`.
    case setActivity(Bool)
    /// Reply `.terminateNow` to a pending `.terminateLater`. Always the last effect.
    case terminateApp
}
