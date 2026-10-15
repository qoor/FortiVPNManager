import AppKit
import Foundation
import FortiConfig
import FortiCore
import FortiShared
import Observation
import os

/// Whether the Gmail API can serve an OTP session for an address right now.
enum GmailAPIAvailability: Equatable {
    /// `otp.method` is `imap`.
    case notSelected
    /// `otp.method` is `gmail_api`, but the client secret or the refresh token is missing.
    case notConfigured
    case available
}

/// Why no access token could be obtained (Gmail API OTP session, Gmail API test).
nonisolated enum GoogleAccessTokenError: Error, Sendable, Equatable {
    /// No client ID, client secret or refresh token for the address.
    case notAuthorized
    /// Google refused the refresh token (`invalid_grant`); it was deleted from the Keychain.
    case revoked
    /// Google refused the client ID or secret (`invalid_client` / `unauthorized_client`).
    case clientRejected
    case keychain(String)
    case network(String)
    case server(String)

    /// Retrying cannot help; the user has to sign in again (or fix the client).
    var isAuthenticationFailure: Bool {
        switch self {
        case .notAuthorized, .revoked, .clientRejected, .keychain: true
        case .network, .server: false
        }
    }

    /// Plain English for logs and Log-tab notes. Never contains a token or secret.
    var logDescription: String {
        switch self {
        case .notAuthorized: "no Google authorization stored"
        case .revoked: "Google authorization was revoked or expired"
        case .clientRejected: "Google rejected the OAuth client"
        case let .keychain(detail): "Keychain read failed: \(detail)"
        case let .network(detail): "network error: \(detail)"
        case let .server(detail): "token endpoint error: \(detail)"
        }
    }
}

/// Supplies Gmail API access tokens for an address.
@MainActor
protocol GoogleAccessTokenProviding: AnyObject {
    /// A token valid for at least another minute, refreshed with the stored refresh token when needed.
    func accessToken(for address: String) async throws(GoogleAccessTokenError) -> String
    /// Forget the cached token (after a 401) so the next call refreshes.
    func invalidateAccessToken(for address: String)
}

/// User-facing failures of the Google sign-in and the client JSON import.
nonisolated enum GoogleOAuthError: LocalizedError, Equatable {
    /// The client JSON could not be read; the text says why (localized).
    case clientFileUnreadable(String)
    case clientFile(GoogleOAuthClientFileError)
    /// No OAuth client JSON imported (no client ID in config.yaml, or no client secret in the Keychain).
    case clientNotConfigured
    case addressMissing
    case alreadyInProgress
    case listenerFailed(String)
    case browserUnavailable
    case timedOut
    case cancelled
    /// Google redirected back with `error=<code>`.
    case denied(String)
    /// The token endpoint refused the code (text from Google, safe to show).
    case tokenRejected(String)
    case clientRejected
    case noRefreshToken
    case scopeNotGranted
    case wrongAccount(expected: String, actual: String)
    /// The Gmail API refused the profile check (reason such as `accessNotConfigured`).
    case gmailAPIRefused(String)
    case network(String)
    case keychain(String)
    case config(ConfigStoreError)

    var errorDescription: String? {
        switch self {
        case let .clientFileUnreadable(reason):
            String(localized: "Couldn't read the OAuth client file: \(reason)")
        case .clientFile(.notJSON):
            String(localized: "This file is not a Google OAuth client JSON.")
        case .clientFile(.webClient):
            String(localized: "This is a Web application client. Create an OAuth client of type Desktop app in Google Cloud Console and download its JSON.")
        case .clientFile(.missingInstalledSection):
            String(localized: "This file has no Desktop app client. Download the JSON of an OAuth client of type Desktop app.")
        case let .clientFile(.missingField(name)), let .clientFile(.invalidField(name)):
            String(localized: "The OAuth client JSON has a missing or invalid \(name) field.")
        case .clientNotConfigured:
            String(localized: "Import the OAuth client JSON first.")
        case .addressMissing:
            String(localized: "Enter the Gmail address.")
        case .alreadyInProgress:
            String(localized: "Google sign-in is already in progress.")
        case let .listenerFailed(detail):
            String(localized: "Couldn't start the local sign-in listener: \(detail)")
        case .browserUnavailable:
            String(localized: "Couldn't open the web browser for Google sign-in.")
        case .timedOut:
            String(localized: "Google sign-in timed out after 5 minutes.")
        case .cancelled:
            String(localized: "Google sign-in was cancelled.")
        case .denied("access_denied"):
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
            String(localized: "Gmail access was not granted. Sign in again and allow access to Gmail.")
        case let .wrongAccount(expected, actual):
            String(localized: "You signed in as \(actual), but the Gmail address is \(expected). Sign in with that account.")
        case let .gmailAPIRefused(reason):
            String(localized: "The Gmail API refused access (\(reason)). Check that the Gmail API is enabled for the Google Cloud project.")
        case let .network(detail):
            String(localized: "Couldn't reach Google: \(detail)")
        case let .keychain(detail):
            String(localized: "Keychain error: \(detail)")
        case let .config(error):
            error.message
        }
    }
}

/// Google OAuth for the Gmail API OTP path (`otp.method: gmail_api`).
///
/// - Import: the Desktop-app client JSON's secret goes to the Keychain (`client-secret:<client ID>`),
///   its client ID to config.yaml `otp.google_client_id`.
/// - Sign-in (`connect`): loopback redirect on `127.0.0.1` + PKCE in the default browser, code
///   exchange, then `users.getProfile` must report the configured Gmail address. The refresh token
///   goes to the Keychain (`refresh-token:<lowercased address>`).
/// - Access tokens are kept in memory only and refreshed shortly before they expire. A refresh
///   answered with `invalid_grant` deletes the stored refresh token (authorization revoked).
@Observable
final class GoogleOAuthService: GoogleAccessTokenProviding {
    /// How long `connect` waits for the browser to come back.
    static let flowTimeout: TimeInterval = 5 * 60
    static let maxClientFileBytes = 64 * 1024
    /// Cached access tokens closer than this to expiry are refreshed first.
    static let expiryMargin: TimeInterval = 60

    /// A browser sign-in is running.
    private(set) var isConnecting = false
    /// Bumps whenever the stored client or authorization changes, so views that read `isAuthorized`
    /// or `hasClientSecret` re-evaluate.
    private(set) var revision = 0

    /// Called after the client or an authorization changed (wired to a readiness re-check).
    @ObservationIgnored var onAuthorizationChanged: (() -> Void)?

    @ObservationIgnored private let configStore: ConfigStore
    @ObservationIgnored private let credentials: CredentialStore
    @ObservationIgnored private let http: GoogleHTTP
    @ObservationIgnored private let gmail: GmailAPIClient
    @ObservationIgnored private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "oauth")
    @ObservationIgnored private var cachedTokens: [String: CachedToken] = [:]
    @ObservationIgnored private var refreshes: [String: Task<Result<String, GoogleAccessTokenError>, Never>] = [:]
    @ObservationIgnored private var flowServer: LoopbackCallbackServer?

    private struct CachedToken {
        var token: String
        var expiresAt: Date
    }

    init(configStore: ConfigStore, credentials: CredentialStore, http: GoogleHTTP = .shared) {
        self.configStore = configStore
        self.credentials = credentials
        self.http = http
        gmail = GmailAPIClient(http: http)
    }

    // MARK: - Client

    /// `otp.google_client_id` from config.yaml (empty when no client JSON was imported).
    var clientID: String { configStore.config.otp.googleClientID }

    /// Whether the client secret for `clientID` is in the Keychain.
    var hasClientSecret: Bool {
        _ = revision
        let clientID = clientID
        guard !clientID.isEmpty else { return false }
        return keychainHas { try credentials.hasGoogleClientSecret(clientID: clientID) }
    }

    /// Imports a Google Cloud "Desktop app" OAuth client JSON: the secret goes to the Keychain, the
    /// client ID to config.yaml. Switching to a different client deletes the refresh token of the
    /// configured Gmail address (it was issued to the old client). Returns the client ID.
    @discardableResult
    func importClientFile(at url: URL) throws(GoogleOAuthError) -> String {
        let data: Data
        do {
            data = try FileIO.readRegularFile(url, limit: Self.maxClientFileBytes)
        } catch {
            throw .clientFileUnreadable(Self.describe(error))
        }
        let client: GoogleOAuthClient
        do {
            client = try GoogleOAuthClientFile.parse(data)
        } catch {
            throw .clientFile(error)
        }
        let previous = clientID
        do {
            try credentials.setGoogleClientSecret(client.clientSecret, clientID: client.clientID)
        } catch {
            throw .keychain(error.localizedDescription)
        }
        do {
            try configStore.update { $0.otp.googleClientID = client.clientID }
        } catch {
            throw .config(error)
        }
        if previous != client.clientID {
            let address = configStore.config.otp.gmailAddress
            if !address.isEmpty {
                logIfFails("deleting the refresh token of the previous client") {
                    try credentials.deleteGoogleRefreshToken(address: address)
                }
            }
            if !previous.isEmpty {
                logIfFails("deleting the previous client secret") {
                    try credentials.deleteGoogleClientSecret(clientID: previous)
                }
            }
            cachedTokens.removeAll()
        }
        logger.notice("Imported Google OAuth client \(client.clientID, privacy: .public)")
        authorizationChanged()
        return client.clientID
    }

    // MARK: - Authorization state

    /// The client secret and a refresh token for `address` are stored (the Gmail API can be used).
    func isAuthorized(address: String) -> Bool {
        _ = revision
        let address = Self.normalized(address)
        let clientID = clientID
        return keychainHas { try credentials.hasGmailAPIAuthorization(address: address, clientID: clientID) }
    }

    /// What `OTPSessionRouter` should use for `address`.
    func availability(address: String) -> GmailAPIAvailability {
        guard configStore.config.otp.method == .gmailAPI else { return .notSelected }
        return isAuthorized(address: address) ? .available : .notConfigured
    }

    /// Runs the browser sign-in for `address` and stores the refresh token. Requires an imported
    /// client. Fails with `.wrongAccount` when the user signs in with another Google account.
    func connect(address rawAddress: String) async throws(GoogleOAuthError) {
        let address = Self.normalized(rawAddress)
        guard !address.isEmpty else { throw .addressMissing }
        guard !isConnecting else { throw .alreadyInProgress }
        let clientID = clientID
        guard !clientID.isEmpty else { throw .clientNotConfigured }
        let storedSecret: String?
        do {
            storedSecret = try credentials.googleClientSecret(clientID: clientID)
        } catch {
            throw .keychain(error.localizedDescription)
        }
        guard let secret = storedSecret, !secret.isEmpty else { throw .clientNotConfigured }

        isConnecting = true
        defer {
            isConnecting = false
            flowServer = nil
        }
        let state = GoogleOAuth.makeState()
        let pkce = PKCE.generate()
        let server = LoopbackCallbackServer(expectedState: state)
        flowServer = server
        let port: UInt16
        do {
            port = try await server.start()
        } catch {
            throw Self.map(error)
        }
        let redirectURI = GoogleOAuth.loopbackRedirectURI(port: port)
        let url = GoogleOAuth.authorizationURL(
            clientID: clientID, redirectURI: redirectURI, codeChallenge: pkce.challenge, state: state, loginHint: address
        )
        logger.notice("Google sign-in: opening the browser")
        guard NSWorkspace.shared.open(url) else {
            server.cancel()
            throw .browserUnavailable
        }
        let code: String
        do {
            code = try await server.waitForCallback(timeout: Self.flowTimeout)
        } catch {
            logger.notice("Google sign-in ended without a code: \(String(describing: error), privacy: .public)")
            throw Self.map(error)
        }

        let grant = try await exchange(
            code: code, clientID: clientID, secret: secret, redirectURI: redirectURI, verifier: pkce.verifier
        )
        guard let refreshToken = grant.refreshToken else {
            await revoke(grant.accessToken)
            throw .noRefreshToken
        }
        guard grant.grants(GoogleOAuth.gmailModifyScope) else {
            await revoke(refreshToken)
            throw .scopeNotGranted
        }
        let profile: GmailProfile
        do {
            profile = try await gmail.profile(accessToken: grant.accessToken)
        } catch {
            await revoke(refreshToken)
            throw Self.map(error)
        }
        guard profile.emailAddress.caseInsensitiveCompare(address) == .orderedSame else {
            logger.error("Google sign-in used a different account than the configured Gmail address")
            await revoke(refreshToken)
            throw .wrongAccount(expected: address, actual: profile.emailAddress)
        }
        do {
            try credentials.setGoogleRefreshToken(refreshToken, address: address)
        } catch {
            throw .keychain(error.localizedDescription)
        }
        cachedTokens[Self.cacheKey(address: address, clientID: clientID)] = CachedToken(
            token: grant.accessToken, expiresAt: Date().addingTimeInterval(grant.expiresIn)
        )
        logger.notice("Google sign-in completed; refresh token stored")
        authorizationChanged()
    }

    /// Cancels a running `connect` (it then fails with `.cancelled`).
    func cancelConnect() {
        flowServer?.cancel()
    }

    /// Deletes the stored refresh token of `address` and, with `revokeAtGoogle`, revokes it at Google
    /// (best effort; this removes the app's Gmail access for that account).
    func disconnect(address rawAddress: String, revokeAtGoogle: Bool = true) async throws(GoogleOAuthError) {
        let address = Self.normalized(rawAddress)
        guard !address.isEmpty else { return }
        let token = try? credentials.googleRefreshToken(address: address)
        do {
            try credentials.deleteGoogleRefreshToken(address: address)
        } catch {
            throw .keychain(error.localizedDescription)
        }
        cachedTokens.removeAll()
        logger.notice("Google authorization removed")
        authorizationChanged()
        if revokeAtGoogle, let token {
            await revoke(token)
        }
    }

    // MARK: - Access tokens

    func accessToken(for rawAddress: String) async throws(GoogleAccessTokenError) -> String {
        let address = Self.normalized(rawAddress)
        let clientID = clientID
        guard !address.isEmpty, !clientID.isEmpty else { throw .notAuthorized }
        let key = Self.cacheKey(address: address, clientID: clientID)
        if let cached = cachedTokens[key], cached.expiresAt.timeIntervalSinceNow > Self.expiryMargin {
            return cached.token
        }
        if let running = refreshes[key] {
            return try await running.value.get()
        }
        let task = Task { await self.refresh(address: address, clientID: clientID) }
        refreshes[key] = task
        let result = await task.value
        if refreshes[key] == task { refreshes[key] = nil }
        return try result.get()
    }

    func invalidateAccessToken(for address: String) {
        cachedTokens[Self.cacheKey(address: Self.normalized(address), clientID: clientID)] = nil
    }

    /// `grant_type=refresh_token`. `invalid_grant` deletes the refresh token that was used.
    private func refresh(address: String, clientID: String) async -> Result<String, GoogleAccessTokenError> {
        let secret: String?
        let refreshToken: String?
        do {
            secret = try credentials.googleClientSecret(clientID: clientID)
            refreshToken = try credentials.googleRefreshToken(address: address)
        } catch {
            return .failure(.keychain(error.localizedDescription))
        }
        guard let secret, !secret.isEmpty, let refreshToken, !refreshToken.isEmpty else {
            return .failure(.notAuthorized)
        }
        let response: GoogleHTTP.Response
        do {
            response = try await http.postForm(
                GoogleOAuth.tokenEndpoint,
                body: GoogleOAuth.refreshRequestBody(refreshToken: refreshToken, clientID: clientID, clientSecret: secret)
            )
        } catch {
            return .failure(.network(error.detail))
        }
        switch GoogleTokenResponse.decode(response.body, status: response.status) {
        case let .success(grant):
            // A disconnect or a new client during the request makes this token stale.
            if self.clientID == clientID, (try? credentials.googleRefreshToken(address: address)) == refreshToken {
                cachedTokens[Self.cacheKey(address: address, clientID: clientID)] = CachedToken(
                    token: grant.accessToken, expiresAt: Date().addingTimeInterval(grant.expiresIn)
                )
            }
            return .success(grant.accessToken)
        case .failure(.invalidGrant):
            logger.error("Google refused the refresh token (invalid_grant); deleting it")
            if (try? credentials.googleRefreshToken(address: address)) == refreshToken {
                logIfFails("deleting the revoked refresh token") {
                    try credentials.deleteGoogleRefreshToken(address: address)
                }
                cachedTokens.removeAll()
                authorizationChanged()
            }
            return .failure(.revoked)
        case .failure(.invalidClient):
            logger.error("Google refused the OAuth client during a token refresh")
            return .failure(.clientRejected)
        case let .failure(.server(status)):
            return .failure(.server("HTTP \(status)"))
        case let .failure(.oauth(code, _)):
            return .failure(.server(code))
        case .failure(.malformedResponse):
            return .failure(.server("malformed token response"))
        }
    }

    // MARK: - Token endpoint

    private func exchange(
        code: String, clientID: String, secret: String, redirectURI: String, verifier: String
    ) async throws(GoogleOAuthError) -> GoogleTokenGrant {
        let body = GoogleOAuth.authorizationCodeRequestBody(
            code: code, clientID: clientID, clientSecret: secret, redirectURI: redirectURI, codeVerifier: verifier
        )
        let response: GoogleHTTP.Response
        do {
            response = try await http.postForm(GoogleOAuth.tokenEndpoint, body: body)
        } catch {
            throw .network(error.detail)
        }
        switch GoogleTokenResponse.decode(response.body, status: response.status) {
        case let .success(grant):
            return grant
        case .failure(.invalidClient):
            logger.error("Google refused the OAuth client during the code exchange")
            throw .clientRejected
        case let .failure(error):
            logger.error("Google code exchange failed: \(Self.describe(error), privacy: .public)")
            throw .tokenRejected(Self.describe(error))
        }
    }

    /// Best-effort revocation (also removes the app's grant for that account).
    private func revoke(_ token: String) async {
        do {
            let response = try await http.postForm(
                GoogleOAuth.revocationEndpoint, body: GoogleOAuth.revocationRequestBody(token: token)
            )
            logger.notice("Google token revocation answered HTTP \(response.status, privacy: .public)")
        } catch {
            logger.error("Google token revocation failed: \(error.detail, privacy: .public)")
        }
    }

    // MARK: - Helpers

    private func authorizationChanged() {
        revision &+= 1
        onAuthorizationChanged?()
    }

    private func keychainHas(_ lookup: () throws -> Bool) -> Bool {
        do {
            return try lookup()
        } catch {
            logger.error("keychain lookup failed: \(error.localizedDescription, privacy: .public)")
            return false
        }
    }

    private func logIfFails(_ what: String, _ action: () throws -> Void) {
        do {
            try action()
        } catch {
            logger.error("\(what, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    private static func normalized(_ address: String) -> String {
        address.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func cacheKey(address: String, clientID: String) -> String {
        "\(clientID) \(address.lowercased())"
    }

    private static func describe(_ error: FileIO.ReadError) -> String {
        switch error {
        case .notFound: String(localized: "The file does not exist.")
        case .notRegularFile: String(localized: "This is not a regular file.")
        case .tooLarge: String(localized: "The file is too large to be an OAuth client JSON.")
        case let .io(error): error.localizedDescription
        }
    }

    /// `invalid_request: Missing code verifier.` style text from Google (no secrets).
    private static func describe(_ error: GoogleTokenError) -> String {
        switch error {
        case let .invalidGrant(description): "invalid_grant" + (description.map { ": \($0)" } ?? "")
        case let .invalidClient(description): "invalid_client" + (description.map { ": \($0)" } ?? "")
        case let .oauth(code, description): code + (description.map { ": \($0)" } ?? "")
        case let .server(status): "HTTP \(status)"
        case .malformedResponse: "malformed token response"
        }
    }

    private static func map(_ error: LoopbackCallbackError) -> GoogleOAuthError {
        switch error {
        case let .listenerFailed(detail): .listenerFailed(detail)
        case .timedOut: .timedOut
        case .cancelled: .cancelled
        case let .denied(code): .denied(code)
        }
    }

    private static func map(_ error: GmailAPIError) -> GoogleOAuthError {
        switch error {
        case let .network(detail): .network(detail)
        case .unauthorized: .gmailAPIRefused("unauthorized")
        case let .forbidden(reason): .gmailAPIRefused(reason)
        case .notFound: .gmailAPIRefused("not found")
        case .rateLimited: .gmailAPIRefused("rate limited")
        case let .server(status): .gmailAPIRefused("HTTP \(status)")
        }
    }
}
