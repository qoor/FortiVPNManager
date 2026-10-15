import Foundation
import os
import FortiCore
import FortiShared

/// "Test Gmail API" in the Account tab: gets an access token (refreshing it with the stored refresh
/// token) and checks that `users.getProfile` reports the configured Gmail address.
struct GmailAPIConnectionTester: GmailAPIConnectionTesting {
    let oauth: GoogleOAuthService
    let client: GmailAPIClient

    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "otp")

    init(oauth: GoogleOAuthService, client: GmailAPIClient = GmailAPIClient()) {
        self.oauth = oauth
        self.client = client
    }

    func test(address rawAddress: String) async -> GmailAPITestResult {
        let address = rawAddress.trimmingCharacters(in: .whitespacesAndNewlines)
        let result = await run(address: address)
        logger.info("Gmail API connection test: \(Self.summary(of: result), privacy: .public)")
        return result
    }

    private func run(address: String) async -> GmailAPITestResult {
        guard !address.isEmpty else { return .notAuthorized(String(localized: "Enter the Gmail address.")) }
        guard !oauth.clientID.isEmpty, oauth.hasClientSecret else {
            return .notAuthorized(String(localized: "Import the OAuth client JSON first."))
        }
        guard oauth.isAuthorized(address: address) else {
            return .notAuthorized(String(localized: "Sign in with Google first."))
        }
        for attempt in 1...2 {
            let token: String
            do {
                token = try await oauth.accessToken(for: address)
            } catch {
                return Self.result(for: error)
            }
            do {
                let profile = try await client.profile(accessToken: token)
                guard profile.emailAddress.caseInsensitiveCompare(address) == .orderedSame else {
                    return .authenticationFailed(String(
                        localized: "The Google authorization belongs to \(profile.emailAddress), not \(address). Sign in with Google again."
                    ))
                }
                return .success(address: profile.emailAddress)
            } catch .unauthorized where attempt == 1 {
                oauth.invalidateAccessToken(for: address)
            } catch {
                return Self.result(for: error)
            }
        }
        return .authenticationFailed(String(localized: "Google rejected the access token. Sign in with Google again."))
    }

    private static func result(for error: GoogleAccessTokenError) -> GmailAPITestResult {
        switch error {
        case .notAuthorized:
            .notAuthorized(String(localized: "Sign in with Google first."))
        case .revoked:
            .authenticationFailed(String(localized: "Google access was revoked or has expired. Sign in with Google again."))
        case .clientRejected:
            .authenticationFailed(String(localized: "Google rejected the OAuth client. Import the client JSON again."))
        case let .keychain(detail):
            .authenticationFailed(String(localized: "Keychain error: \(detail)"))
        case let .network(detail):
            .networkError(String(localized: "Couldn't reach Google: \(detail)"))
        case let .server(detail):
            serverError(detail)
        }
    }

    private static func result(for error: GmailAPIError) -> GmailAPITestResult {
        switch error {
        case .unauthorized:
            .authenticationFailed(String(localized: "Google rejected the access token. Sign in with Google again."))
        case let .forbidden(reason):
            .authenticationFailed(String(
                localized: "The Gmail API refused access (\(reason)). Check that the Gmail API is enabled for the Google Cloud project."
            ))
        case .rateLimited:
            .networkError(String(localized: "The Gmail API rate limit was reached. Try again in a minute."))
        case let .network(detail):
            .networkError(String(localized: "Couldn't reach Google: \(detail)"))
        case .notFound:
            serverError("HTTP 404")
        case let .server(status):
            serverError("HTTP \(status)")
        }
    }

    private static func serverError(_ detail: String) -> GmailAPITestResult {
        .networkError(String(localized: "Google returned an error: \(detail)"))
    }

    private static func summary(of result: GmailAPITestResult) -> String {
        switch result {
        case .success: "success"
        case .notAuthorized: "not authorized"
        case .authenticationFailed: "authentication failed"
        case let .networkError(message): "network error: \(message)"
        }
    }
}
