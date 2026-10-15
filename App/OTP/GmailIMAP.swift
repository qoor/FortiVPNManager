import Foundation
import FortiCore

/// Gmail refused the login: wrong address or app password, Google demands a browser sign-in, or IMAP
/// is disabled for the account or domain. Retrying with the same credentials cannot help.
nonisolated struct GmailLoginRejected: LocalizedError, Sendable {
    /// The server's explanation, e.g. "Invalid credentials (Failure)". Never contains credentials.
    let serverText: String

    var errorDescription: String? {
        "Gmail rejected the login" + (serverText.isEmpty ? "" : ": \(serverText)")
    }
}

/// Gmail login and mailbox discovery shared by `GmailOTPSession` and `GmailConnectionTester`.
extension IMAPTransport {
    /// Budget for TCP + TLS + greeting + AUTHENTICATE.
    nonisolated static let loginTimeout: Duration = .seconds(15)

    /// Connects and logs in with `AUTHENTICATE PLAIN` (SASL-IR) within `loginTimeout`.
    /// Throws `GmailLoginRejected` for refused credentials and `IMAPTransportError` for everything else.
    func logIn(_ account: GmailAccount) async throws {
        let deadline = ContinuousClock.now + Self.loginTimeout
        try await connect(timeout: Self.loginTimeout)
        do {
            let command = IMAPCommand.authenticatePlain(username: account.address, password: account.appPassword)
            _ = try await execute(command, timeout: deadline - ContinuousClock.now)
        } catch IMAPTransportError.rejected(_, .no, let code, let text)
            where IMAPLoginRejection.isCredentialRejection(code: code, text: text) {
            throw GmailLoginRejected(serverText: text)
        } catch IMAPTransportError.invalidCommand(.invalidCredentials) {
            throw GmailLoginRejected(serverText: "")
        }
    }

    /// `LIST "" "*"` resolved to the `\All`/`\Trash`/`\Junk` mailboxes. Names stay in wire form
    /// (modified UTF-7, e.g. a Korean Gmail UI); use `displayName` for the UI.
    func specialMailboxes() async throws -> IMAPSpecialMailboxes {
        IMAPSpecialMailboxes(responses: try await execute(.list))
    }
}
