import Foundation
import os
import FortiCore
import FortiShared

/// "Test Gmail Connection" in the Account tab: logs in, resolves `\All`/`\Trash`/`\Junk` and logs out.
struct GmailConnectionTester: GmailConnectionTesting {
    var endpoint: IMAPEndpoint = .gmail

    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "otp")

    func test(account: GmailAccount) async -> GmailTestResult {
        let transport = IMAPTransport(endpoint: endpoint)
        let result: GmailTestResult
        do {
            try await transport.logIn(account)
            result = Self.result(for: try await transport.specialMailboxes())
        } catch let rejection as GmailLoginRejected {
            let serverText = rejection.serverText
            result = .authenticationFailed(serverText.isEmpty
                ? String(localized: "Gmail rejected the sign-in. Check the Gmail address, the app password and that IMAP is enabled.")
                : String(localized: "Gmail rejected the sign-in. Check the Gmail address, the app password and that IMAP is enabled. (Server response: \(serverText))"))
        } catch {
            result = .networkError(String(localized: "Could not reach the Gmail server: \(error.localizedDescription)"))
        }
        await transport.logout()
        logger.info("Gmail connection test: \(Self.summary(of: result), privacy: .public)")
        return result
    }

    private static func result(for mailboxes: IMAPSpecialMailboxes) -> GmailTestResult {
        guard let all = mailboxes.all, let trash = mailboxes.trash else {
            let missing = [
                mailboxes.all == nil ? String(localized: "All Mail (\\All)") : nil,
                mailboxes.trash == nil ? String(localized: "Trash (\\Trash)") : nil,
            ].compactMap(\.self)
            let names = missing.joined(separator: ", ")
            return .mailboxesMissing(
                String(localized: "Mailboxes not visible over IMAP: \(names). In Gmail Settings > Labels, turn on 'Show in IMAP'.")
            )
        }
        return .success(allMail: all.displayName, trash: trash.displayName, junk: mailboxes.junk?.displayName)
    }

    private static func summary(of result: GmailTestResult) -> String {
        switch result {
        case .success: "success"
        case .authenticationFailed: "authentication failed"
        case .mailboxesMissing: "mailboxes missing"
        case .networkError(let message): "network error: \(message)"
        }
    }
}
