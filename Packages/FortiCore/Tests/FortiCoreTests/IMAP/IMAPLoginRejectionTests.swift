import Foundation
import Testing
@testable import FortiCore

@Suite("IMAP login rejection")
struct IMAPLoginRejectionTests {
    /// Decodes a tagged reply line the way IMAPTransport sees it and classifies it.
    private func isCredentialRejection(_ line: String) throws -> Bool {
        let responses = try IMAPFixture.parseAll(IMAPFixture.lines(line))
        guard case .tagged(_, .no, let code, let text)? = responses.first, responses.count == 1 else {
            Issue.record("expected one tagged NO, got \(responses)")
            return false
        }
        return IMAPLoginRejection.isCredentialRejection(code: code, text: text)
    }

    @Test(arguments: [
        "A0001 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)",
        "A0001 NO [ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)",
        "A0001 NO [ALERT] Please log in via your web browser: https://support.google.com/mail/accounts/answer/78754 (Failure)",
        // Google flagged the sign-in: retrying from here only repeats a suspicious login.
        "A0001 NO [WEBALERT https://accounts.google.com/ContinueSignIn?sarp=1&scc=1&plt=AKgnsbs] Web login required.",
        // IMAP disabled by the user or a Workspace admin.
        "A0001 NO [ALERT] Your account is not enabled for IMAP use. Please visit your Gmail settings page and enable your account for IMAP access. (Failure)",
        "A0001 NO [ALERT] IMAP access is disabled for your domain. Please contact your domain administrator for questions about this feature. (Failure)",
        "A0001 NO [AUTHORIZATIONFAILED] Not allowed",
        "A0001 NO [EXPIRED] Password expired",
        "A0001 NO Invalid credentials",
    ])
    func permanentRefusalsAreCredentialRejections(_ line: String) throws {
        #expect(try isCredentialRejection(line))
    }

    @Test(arguments: [
        "A0001 NO [UNAVAILABLE] Temporary System Problem. Try again later.",
        "A0001 NO [LIMIT] Too many connections",
        "A0001 NO [INUSE] Mailbox in use",
        "A0001 NO [SERVERBUG] Internal error",
        "A0001 NO [ALERT] Too many simultaneous connections. (Failure)",
        "A0001 NO [ALERT] Account exceeded command or bandwidth limits. (Failure)",
        "A0001 NO [ALERT] Temporary problem, please try again later",
    ])
    func transientRefusalsAreNotCredentialRejections(_ line: String) throws {
        #expect(try !isCredentialRejection(line))
    }
}
