import Foundation

extension AppInternal.GoogleError {
    func message(time: TimeStyle) -> String {
        switch self {
        case let .clientFileUnreadable(problem):
            String(localized: "Couldn't read the OAuth client file: \(problem.clientFileReason)")
        case let .clientFileInvalid(problem):
            problem.message
        case let .listenerFailed(detail):
            String(localized: "Couldn't start the local sign-in listener: \(detail)")
        case .browserUnavailable:
            String(localized: "Couldn't open the web browser for Google sign-in.")
        case let .timedOut(afterSeconds):
            String(localized: "Google sign-in timed out after \(time.spelledOut(seconds: afterSeconds)).")
        case .cancelledInBrowser:
            String(localized: "Google sign-in was cancelled in the browser.")
        case let .denied(code):
            String(localized: "Google sign-in failed (\(code)).")
        case let .tokenRejected(detail):
            String(localized: "Google rejected the sign-in: \(detail)")
        case .clientRejected:
            String(localized: "Google rejected the OAuth client. Import the client JSON again.")
        case .noRefreshToken:
            String(localized: "Google did not return a refresh token. Sign in again.")
        case .scopeNotGranted:
            String(localized: "Gmail access wasn't allowed. Sign in again and allow it.")
        case let .wrongAccount(expected, actual):
            String(localized: "You signed in as \(actual), but the Gmail address is \(expected). Sign in with that account.")
        case let .gmailApiRefused(reason):
            String(localized: "The Gmail API refused access (\(reason)). Check that it's enabled in Google Cloud.")
        case let .network(detail):
            String(localized: "Couldn't reach Google: \(detail)")
        case let .secretStore(failure):
            String(localized: "Keychain error: \(failure.description)")
        case let .config(error):
            error.message
        }
    }
}

extension AppInternal.OAuthClientFileProblem {
    var message: String {
        switch self {
        case .notJson:
            String(localized: "This file is not a Google OAuth client JSON.")
        case .webClient:
            String(localized: "This is a Web application client. Import a Desktop app client JSON.")
        case .noDesktopClient:
            String(localized: "This file has no Desktop app client.")
        case let .missingField(name), let .invalidField(name):
            String(localized: "The OAuth client JSON has a missing or invalid \(name) field.")
        }
    }
}

extension AppInternal.FileProblem {
    /// Why the OAuth client file the user picked could not be read.
    var clientFileReason: String {
        switch self {
        case .notFound: String(localized: "The file does not exist.")
        case .notRegularFile: String(localized: "This is not a regular file.")
        case .tooLarge: String(localized: "The file is too large to be an OAuth client JSON.")
        case let .io(operation, path, errno): fileOperationFailure(operation, path, errno)
        }
    }

    /// Why the VPN client's settings file could not be read.
    var importReason: String {
        switch self {
        case .notFound: String(localized: "The file does not exist.")
        case .notRegularFile: String(localized: "Not a regular file.")
        case let .tooLarge(limitBytes): String(localized: "Larger than \(limitBytes / 1024) KiB.")
        case let .io(operation, path, errno): fileOperationFailure(operation, path, errno)
        }
    }
}

/// "open failed: /path: Permission denied".
private func fileOperationFailure(_ operation: String, _ path: String, _ errno: Int32) -> String {
    String(localized: "\(operation) failed: \(path): \(errorText(errno))")
}

extension AppInternal.GoogleConnectBlocker {
    var message: String {
        switch self {
        case .noOAuthClient: String(localized: "No built-in OAuth client. Import a client JSON.")
        case .enterAddress: String(localized: "Enter the Gmail address.")
        case .saveAddressFirst: String(localized: "Save the Gmail address first.")
        }
    }
}

extension AppInternal.OAuthClientSource {
    /// The status line of the OAuth client row.
    var label: String {
        switch self {
        case .builtIn: String(localized: "Built into the app")
        case let .imported(clientId): String(localized: "Imported (\(Self.tail(of: clientId)))")
        case .notConfigured: String(localized: "Not configured")
        }
    }

    /// The end of a client ID without the `.apps.googleusercontent.com` suffix, such as `…a1b2c3d4e5`.
    private static func tail(of clientID: String) -> String {
        let suffix = ".apps.googleusercontent.com"
        let stem = clientID.hasSuffix(suffix) ? String(clientID.dropLast(suffix.count)) : clientID
        return stem.count > 10 ? "…" + stem.suffix(10) : stem
    }
}

extension AppInternal.GmailTest {
    /// The result next to "Test Gmail Connection": its text and whether it succeeded.
    var result: (text: String, succeeded: Bool)? {
        switch self {
        case .idle, .running:
            nil
        case .imapSucceeded:
            (String(localized: "Connection succeeded (IMAP)"), true)
        case .apiSucceeded:
            (String(localized: "Connection succeeded (Gmail API)"), true)
        case let .failed(failure):
            (failure.message, false)
        }
    }
}

extension AppInternal.GmailTestFailure {
    var message: String {
        switch self {
        case .enterAddress:
            String(localized: "Enter the Gmail address.")
        case .enterAppPassword:
            String(localized: "Enter the app password.")
        case let .appPasswordUnreadable(failure):
            String(localized: "Couldn't read the app password from Keychain: \(failure.description)")
        case let .imapLoginRejected(serverText):
            if let serverText, !serverText.isEmpty {
                String(localized: "Gmail rejected the sign-in. Check the address, app password and IMAP setting. (\(serverText))")
            } else {
                String(localized: "Gmail rejected the sign-in. Check the address, app password and IMAP setting.")
            }
        case let .mailboxesMissing(allMail, trash):
            Self.missingMailboxes(allMail: allMail, trash: trash)
        case let .imapUnreachable(failure):
            String(localized: "Could not reach the Gmail server: \(failure.message)")
        case .noOAuthClient:
            String(localized: "No built-in OAuth client. Import a client JSON.")
        case .notSignedIn:
            String(localized: "Sign in with Google first.")
        case .accessRevoked:
            String(localized: "Google access was revoked or has expired. Sign in with Google again.")
        case .clientRejected:
            String(localized: "Google rejected the OAuth client. Import the client JSON again.")
        case let .secretStore(failure):
            String(localized: "Keychain error: \(failure.description)")
        case let .otherAccount(authorized, expected):
            String(localized: "Connected as \(authorized). Connect again with \(expected).")
        case .accessTokenRejected:
            String(localized: "Google rejected the access token. Sign in with Google again.")
        case let .gmailApiRefused(reason):
            String(localized: "The Gmail API refused access (\(reason)). Check that it's enabled in Google Cloud.")
        case .rateLimited:
            String(localized: "The Gmail API rate limit was reached. Try again in a minute.")
        case let .googleUnreachable(detail):
            String(localized: "Couldn't reach Google: \(detail)")
        case let .googleReturnedError(detail):
            String(localized: "Google returned an error: \(detail)")
        }
    }

    private static func missingMailboxes(allMail: Bool, trash: Bool) -> String {
        let names = [
            allMail ? String(localized: "All Mail (\\All)") : nil,
            trash ? String(localized: "Trash (\\Trash)") : nil,
        ].compactMap(\.self).joined(separator: ", ")
        return String(localized: "Mailboxes not visible over IMAP: \(names). In Gmail Settings > Labels, turn on 'Show in IMAP'.")
    }
}

extension AppInternal.ImapFailure {
    var message: String {
        switch self {
        case let .network(detail): String(localized: "Network error (\(detail))")
        case let .timedOut(step): String(localized: "\(step.name) timed out")
        case let .serverClosed(text):
            if let text {
                String(localized: "The server closed the connection: \(text)")
            } else {
                String(localized: "The server closed the connection")
            }
        case let .protocol(detail): String(localized: "IMAP protocol error: \(detail)")
        case let .rejected(command, status, text): String(localized: "\(command) rejected (\(status)): \(text)")
        }
    }
}

extension AppInternal.ImapStep {
    fileprivate var name: String {
        switch self {
        case .connecting: String(localized: "Connecting to the server")
        case let .command(name): name
        }
    }
}

/// The pages the Google sign-in listener shows in the browser.
extension AppInternal.SignInPages {
    static var localized: Self {
        Self(
            completed: String(localized: "FortiVPNManager: you can close this window."),
            notCompleted: String(localized: "FortiVPNManager: Google sign-in was not completed. You can close this window."),
            invalid: String(localized: "FortiVPNManager: this sign-in response is not valid. Start the sign-in again in Settings.")
        )
    }
}
