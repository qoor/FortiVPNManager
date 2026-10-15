import Foundation
import FortiShared
import FortiCore

/// Gmail credentials for one OTP session. The app password comes from the Keychain.
struct GmailAccount: Sendable, Equatable {
    var address: String
    /// 16-character Google app password with spaces already removed. Empty when none is stored:
    /// the Gmail API session does not need one, and `OTPSessionRouter` then has no IMAP fallback.
    var appPassword: String
}

/// Reports from an OTP session, mapped 1:1 to `ConnectionEvent`s by the ConnectionController.
enum OTPSessionEvent: Sendable, Equatable {
    /// Signed in and snapshot taken (IMAP also swept stale AuthCode mail), waiting (→ `.otpSessionReady`).
    case ready(AttemptID)
    /// Login or Google authorization rejected (`.authentication`) or connection/protocol failure
    /// (`.network`) (→ `.otpSessionFailed`).
    case failed(AttemptID, OTPSessionFailure)
    /// A fresh code matched after `beginSearch` (→ `.otpCodeFound`). Reported at most once per attempt.
    case codeFound(AttemptID, code: String)
    /// `close(checkJunk: true)` found a fresh AuthCode message in the spam folder (→ `.otpFoundInJunk`).
    case foundInJunk(AttemptID)
    /// Diagnostic text for the Log tab (e.g. how long the OTP mail took); not a reducer event.
    case note(AttemptID, String)

    var attemptID: AttemptID {
        switch self {
        case let .ready(id), let .failed(id, _), let .codeFound(id, _), let .foundInJunk(id), let .note(id, _): id
        }
    }
}

/// One Gmail session per connection attempt (DESIGN.md §5.3): IMAP (`GmailOTPSession`), the Gmail
/// REST API (`GmailAPIOTPSession`), or `OTPSessionRouter` choosing between them per attempt.
/// All methods are idempotent and ignore attempt IDs other than the current session's.
@MainActor
protocol OTPSessionService: AnyObject {
    /// Receives every `OTPSessionEvent`, on the main actor.
    var onEvent: (@MainActor (OTPSessionEvent) -> Void)? { get set }

    /// Effect `.startOTPSession`: closes any previous session, then signs in and snapshots the mailbox.
    /// IMAP: logs in, finds `\All`/`\Trash`/`\Junk`, SELECTs `\All`, snapshots UIDVALIDITY/UIDNEXT,
    /// trashes stale AuthCode mail and enters IDLE. Gmail API: access token, `users.getProfile`
    /// (address check) and its history ID.
    func start(attemptID: AttemptID, account: GmailAccount)

    /// Effect `.beginOTPSearch`: start matching mail received at or after `promptedAt − 5 s` (IMAP: IDLE
    /// plus a periodic UID SEARCH; Gmail API: `history.list` every 500 ms) and report `.codeFound` once.
    func beginSearch(attemptID: AttemptID, promptedAt: Date)

    /// Effect `.submitOTP` (after the code was handed to the helper): move every candidate message
    /// to Trash and end the session.
    func finish(attemptID: AttemptID)

    /// Effect `.closeOTPSession`: end the session. With `checkJunk`, first search the spam folder once
    /// and report `.foundInJunk` if a fresh AuthCode message is there.
    func close(attemptID: AttemptID, checkJunk: Bool)
}

/// Result of the settings window's "Test Gmail Connection" button.
enum GmailTestResult: Sendable, Equatable {
    /// Login succeeded and the special-use mailboxes were found.
    case success(allMail: String, trash: String, junk: String?)
    case authenticationFailed(String)
    case mailboxesMissing(String)
    case networkError(String)
}

/// Stand-alone connectivity check used by the Account tab.
@MainActor
protocol GmailConnectionTesting {
    func test(account: GmailAccount) async -> GmailTestResult
}

/// Result of the Account tab's Gmail API check (`GmailAPIConnectionTester`).
enum GmailAPITestResult: Sendable, Equatable {
    /// The stored Google authorization works and belongs to the Gmail address.
    case success(address: String)
    /// No OAuth client imported, or not signed in with Google for this address.
    case notAuthorized(String)
    /// Google refused the authorization (revoked, client rejected, other account, Gmail API refused).
    case authenticationFailed(String)
    case networkError(String)
}

/// Stand-alone Gmail API check used by the Account tab.
@MainActor
protocol GmailAPIConnectionTesting {
    func test(address: String) async -> GmailAPITestResult
}
