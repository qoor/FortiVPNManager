import Foundation
import FortiCore

/// The secrets of DESIGN §5.7 in the login Keychain.
///
/// - VPN password: service `com.qoor.FortiVPNManager.vpn`, account `username@host` (`VPNProfile.profileKey`).
///   A changed host or username therefore needs a new item, so an old password is never tried
///   against another account (lockout protection).
/// - Gmail app password: service `com.qoor.FortiVPNManager.gmail`, account = Gmail address.
/// - Gmail API (`otp.method: gmail_api`): service `com.qoor.FortiVPNManager.google`, account
///   `refresh-token:<lowercased Gmail address>` for the Google refresh token and
///   `client-secret:<client ID>` for the OAuth client secret (the client ID is in config.yaml).
struct CredentialStore {
    static let vpnService = "com.qoor.FortiVPNManager.vpn"
    static let gmailService = "com.qoor.FortiVPNManager.gmail"
    static let googleService = "com.qoor.FortiVPNManager.google"

    private let vpn = KeychainStore(service: Self.vpnService)
    private let gmail = KeychainStore(service: Self.gmailService)
    private let google = KeychainStore(service: Self.googleService)

    /// Keychain account of a VPN password; identical to `VPNProfile.profileKey`.
    static func vpnAccount(username: String, host: String) -> String {
        "\(username)@\(host)"
    }

    /// Google shows app passwords as four groups of four letters; spaces (or any other whitespace)
    /// copied along are not part of the password.
    static func normalizedAppPassword(_ raw: String) -> String {
        String(raw.unicodeScalars.filter { !CharacterSet.whitespacesAndNewlines.contains($0) })
    }

    // MARK: VPN password

    func vpnPassword(for profile: VPNProfile) throws -> String? {
        try vpn.read(account: profile.profileKey)
    }

    func hasVPNPassword(username: String, host: String) throws -> Bool {
        try vpn.contains(account: Self.vpnAccount(username: username, host: host))
    }

    /// Stores the password for `username@host`. An empty password deletes the item instead.
    func setVPNPassword(_ password: String, username: String, host: String) throws {
        let account = Self.vpnAccount(username: username, host: host)
        if password.isEmpty {
            try vpn.delete(account: account)
        } else {
            try vpn.write(password, account: account)
        }
    }

    // MARK: Gmail app password

    func gmailAppPassword(address: String) throws -> String? {
        try gmail.read(account: address)
    }

    func hasGmailAppPassword(address: String) throws -> Bool {
        try gmail.contains(account: address)
    }

    /// Stores the app password with all whitespace removed. An empty result deletes the item instead.
    func setGmailAppPassword(_ password: String, address: String) throws {
        let normalized = Self.normalizedAppPassword(password)
        if normalized.isEmpty {
            try gmail.delete(account: address)
        } else {
            try gmail.write(normalized, account: address)
        }
    }

    /// Credentials for one OTP session, or `nil` when the address or the app password is missing.
    func gmailAccount(address: String) throws -> GmailAccount? {
        guard !address.isEmpty, let password = try gmailAppPassword(address: address), !password.isEmpty else {
            return nil
        }
        return GmailAccount(address: address, appPassword: Self.normalizedAppPassword(password))
    }

    // MARK: Gmail API (Google OAuth)

    /// Keychain account of the Google refresh token for a Gmail address (case-insensitive).
    static func googleRefreshTokenAccount(address: String) -> String {
        "refresh-token:\(address.lowercased())"
    }

    /// Keychain account of an OAuth client secret.
    static func googleClientSecretAccount(clientID: String) -> String {
        "client-secret:\(clientID)"
    }

    func googleRefreshToken(address: String) throws -> String? {
        try google.read(account: Self.googleRefreshTokenAccount(address: address))
    }

    func hasGoogleRefreshToken(address: String) throws -> Bool {
        try google.contains(account: Self.googleRefreshTokenAccount(address: address))
    }

    func setGoogleRefreshToken(_ token: String, address: String) throws {
        try google.write(token, account: Self.googleRefreshTokenAccount(address: address))
    }

    func deleteGoogleRefreshToken(address: String) throws {
        try google.delete(account: Self.googleRefreshTokenAccount(address: address))
    }

    func googleClientSecret(clientID: String) throws -> String? {
        try google.read(account: Self.googleClientSecretAccount(clientID: clientID))
    }

    func hasGoogleClientSecret(clientID: String) throws -> Bool {
        try google.contains(account: Self.googleClientSecretAccount(clientID: clientID))
    }

    func setGoogleClientSecret(_ secret: String, clientID: String) throws {
        try google.write(secret, account: Self.googleClientSecretAccount(clientID: clientID))
    }

    func deleteGoogleClientSecret(clientID: String) throws {
        try google.delete(account: Self.googleClientSecretAccount(clientID: clientID))
    }

    /// Whether the Gmail API can be used for `address` with `clientID`: the client secret and a
    /// refresh token are stored. Reads no secret (no access prompt).
    func hasGmailAPIAuthorization(address: String, clientID: String) throws -> Bool {
        guard !address.isEmpty, !clientID.isEmpty else { return false }
        return try hasGoogleClientSecret(clientID: clientID) && hasGoogleRefreshToken(address: address)
    }
}
