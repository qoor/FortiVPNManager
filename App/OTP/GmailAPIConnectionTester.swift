import Foundation
import os
import FortiCore
import FortiShared

/// "Test Gmail Connection" in the Account tab with the Gmail API method: gets an access token
/// (refreshing it with the stored refresh token) and checks that `users.getProfile` reports the
/// configured Gmail address.
struct GmailAPIConnectionTester {
    let oauth: GoogleOAuthService

    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "otp")

    init(oauth: GoogleOAuthService) {
        self.oauth = oauth
    }

    func test(address: String) async -> GmailAPITestResult {
        let result = await run(address: address)
        logger.info("Gmail API connection test: \(Self.summary(of: result), privacy: .public)")
        return result
    }

    private func run(address: String) async -> GmailAPITestResult {
        guard oauth.hasClientSecret else {
            return .notAuthorized(GoogleOAuthError.clientNotConfigured.localizedDescription)
        }
        guard oauth.isAuthorized(address: address) else {
            return Self.result(for: GoogleAccessTokenError.notAuthorized)
        }
        for attempt in 1...2 {
            let token: String
            do {
                token = try await oauth.accessToken(for: address)
            } catch {
                return Self.result(for: error)
            }
            do {
                let profile = try await GmailAPIClient().profile(accessToken: token)
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
        return Self.result(for: GmailAPIError.unauthorized)
    }

    private static func result(for error: GoogleAccessTokenError) -> GmailAPITestResult {
        switch error {
        case .notAuthorized:
            .notAuthorized(String(localized: "Sign in with Google first."))
        case .revoked:
            .authenticationFailed(String(localized: "Google access was revoked or has expired. Sign in with Google again."))
        case .clientRejected:
            .authenticationFailed(GoogleOAuthError.clientRejected.localizedDescription)
        case let .keychain(detail):
            .authenticationFailed(GoogleOAuthError.keychain(detail).localizedDescription)
        case let .network(detail):
            .networkError(GoogleOAuthError.network(detail).localizedDescription)
        case let .server(detail):
            serverError(detail)
        }
    }

    private static func result(for error: GmailAPIError) -> GmailAPITestResult {
        switch error {
        case .unauthorized:
            .authenticationFailed(String(localized: "Google rejected the access token. Sign in with Google again."))
        case let .forbidden(reason):
            .authenticationFailed(GoogleOAuthError.gmailAPIRefused(reason).localizedDescription)
        case .rateLimited:
            .networkError(String(localized: "The Gmail API rate limit was reached. Try again in a minute."))
        case let .network(detail):
            .networkError(GoogleOAuthError.network(detail).localizedDescription)
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
