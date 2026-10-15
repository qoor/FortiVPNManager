import CryptoKit
import Foundation
import Security

/// Google OAuth 2.0 for installed apps (loopback redirect + PKCE, RFC 8252 / RFC 7636): everything
/// that needs no I/O. The app runs the browser flow and the HTTP requests.
public enum GoogleOAuth {
    public static let authorizationEndpoint = URL(string: "https://accounts.google.com/o/oauth2/v2/auth")!
    public static let tokenEndpoint = URL(string: "https://oauth2.googleapis.com/token")!
    public static let revocationEndpoint = URL(string: "https://oauth2.googleapis.com/revoke")!
    /// Read, label and trash messages (no permanent delete, no settings).
    public static let gmailModifyScope = "https://www.googleapis.com/auth/gmail.modify"

    /// `http://127.0.0.1:<port>`: the loopback redirect Google accepts for "Desktop app" clients on any
    /// port. The token request must repeat it byte for byte.
    public static func loopbackRedirectURI(port: UInt16) -> String {
        "http://127.0.0.1:\(port)"
    }

    /// The browser URL that starts the consent flow. `access_type=offline` + `prompt=consent` make
    /// Google return a refresh token every time; `login_hint` preselects the expected account.
    public static func authorizationURL(
        clientID: String,
        redirectURI: String,
        codeChallenge: String,
        state: String,
        loginHint: String?,
        scope: String = gmailModifyScope
    ) -> URL {
        var pairs: [(String, String)] = [
            ("client_id", clientID),
            ("redirect_uri", redirectURI),
            ("response_type", "code"),
            ("scope", scope),
            ("code_challenge", codeChallenge),
            ("code_challenge_method", PKCE.method),
            ("state", state),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ]
        if let loginHint, !loginHint.isEmpty {
            pairs.append(("login_hint", loginHint))
        }
        // Built by hand: `URLComponents.queryItems` leaves "+" unescaped, which Google reads as a space.
        return URL(string: authorizationEndpoint.absoluteString + "?" + formEncoded(pairs))!
    }

    /// `grant_type=authorization_code` body for the token endpoint.
    public static func authorizationCodeRequestBody(
        code: String, clientID: String, clientSecret: String, redirectURI: String, codeVerifier: String
    ) -> Data {
        Data(formEncoded([
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", clientID),
            ("client_secret", clientSecret),
            ("redirect_uri", redirectURI),
            ("code_verifier", codeVerifier),
        ]).utf8)
    }

    /// `grant_type=refresh_token` body for the token endpoint.
    public static func refreshRequestBody(refreshToken: String, clientID: String, clientSecret: String) -> Data {
        Data(formEncoded([
            ("grant_type", "refresh_token"),
            ("refresh_token", refreshToken),
            ("client_id", clientID),
            ("client_secret", clientSecret),
        ]).utf8)
    }

    /// Body for the revocation endpoint (revoking a refresh token also revokes its access tokens).
    public static func revocationRequestBody(token: String) -> Data {
        Data(formEncoded([("token", token)]).utf8)
    }

    /// `application/x-www-form-urlencoded` with every byte outside the RFC 3986 unreserved set escaped.
    public static func formEncoded(_ pairs: [(String, String)]) -> String {
        pairs.map { "\(percentEncoded($0.0))=\(percentEncoded($0.1))" }.joined(separator: "&")
    }

    /// Percent-encodes everything except `A-Z a-z 0-9 - . _ ~` (UTF-8 bytes, uppercase hex).
    public static func percentEncoded(_ text: String) -> String {
        var out = ""
        for byte in text.utf8 {
            switch byte {
            case 0x41...0x5A, 0x61...0x7A, 0x30...0x39, 0x2D, 0x2E, 0x5F, 0x7E:
                out.unicodeScalars.append(Unicode.Scalar(byte))
            default:
                out += "%" + String(byte, radix: 16, uppercase: true).leftPadded(to: 2)
            }
        }
        return out
    }

    /// Unguessable `state` value binding the callback to this flow (24 random bytes, base64url).
    public static func makeState() -> String {
        Base64URL.encode(SecureRandom.bytes(count: 24))
    }
}

/// PKCE (RFC 7636) with the S256 method.
public struct PKCE: Sendable, Equatable {
    public static let method = "S256"
    /// Verifier length limits from RFC 7636 §4.1.
    public static let verifierLengths = 43...128
    /// Random bytes behind a generated verifier (64 base64url characters).
    public static let generatedByteCount = 48

    /// High-entropy secret kept by the app until the token exchange.
    public let verifier: String
    /// `BASE64URL(SHA256(ASCII(verifier)))`, sent in the authorization request.
    public let challenge: String

    /// A verifier from `count` bytes of the system CSPRNG.
    public static func generate() -> PKCE {
        PKCE(verifier: Base64URL.encode(SecureRandom.bytes(count: generatedByteCount)))!
    }

    /// `nil` unless `verifier` is 43–128 characters of `[A-Z] / [a-z] / [0-9] / "-" / "." / "_" / "~"`.
    public init?(verifier: String) {
        guard Self.verifierLengths.contains(verifier.utf8.count),
              verifier.utf8.allSatisfy(Self.isUnreserved)
        else { return nil }
        self.verifier = verifier
        challenge = Self.challenge(for: verifier)
    }

    public static func challenge(for verifier: String) -> String {
        Base64URL.encode(Data(SHA256.hash(data: Data(verifier.utf8))))
    }

    private static func isUnreserved(_ byte: UInt8) -> Bool {
        switch byte {
        case 0x41...0x5A, 0x61...0x7A, 0x30...0x39, 0x2D, 0x2E, 0x5F, 0x7E: true
        default: false
        }
    }
}

/// base64url without padding (RFC 4648 §5).
public enum Base64URL {
    public static func encode(_ bytes: some DataProtocol) -> String {
        Data(bytes).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

/// Bytes from the system CSPRNG (`SecRandomCopyBytes`).
enum SecureRandom {
    static func bytes(count: Int) -> [UInt8] {
        var bytes = [UInt8](repeating: 0, count: count)
        let status = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        if status != errSecSuccess {
            // Not expected to happen; the system generator is CSPRNG-backed on Apple platforms too.
            var generator = SystemRandomNumberGenerator()
            for index in bytes.indices { bytes[index] = generator.next() }
        }
        return bytes
    }
}

// MARK: - Token endpoint

/// A successful token endpoint answer. Its `description` never shows the tokens.
public struct GoogleTokenGrant: Sendable, Equatable, CustomStringConvertible {
    public var accessToken: String
    /// Lifetime of `accessToken` in seconds as reported by Google (typically 3599).
    public var expiresIn: TimeInterval
    /// Present for the authorization-code grant with `access_type=offline`; usually absent on refresh.
    public var refreshToken: String?
    /// Granted scopes (Google may grant fewer than requested when the user unticks one).
    public var scopes: Set<String>

    public init(accessToken: String, expiresIn: TimeInterval, refreshToken: String? = nil, scopes: Set<String> = []) {
        self.accessToken = accessToken
        self.expiresIn = expiresIn
        self.refreshToken = refreshToken
        self.scopes = scopes
    }

    /// Whether `scope` was granted. A refresh answer without a `scope` field keeps the original grant.
    public func grants(_ scope: String) -> Bool {
        scopes.contains(scope)
    }

    public var description: String {
        "GoogleTokenGrant(expiresIn: \(Int(expiresIn)), refreshToken: \(refreshToken == nil ? "no" : "yes"), "
            + "scopes: \(scopes.sorted()))"
    }
}

/// Why the token endpoint refused (RFC 6749 §5.2) or answered something unusable.
public enum GoogleTokenError: Error, Sendable, Equatable {
    /// `invalid_grant`: the refresh token was revoked or expired, or the code is invalid or used.
    case invalidGrant(description: String?)
    /// `invalid_client` / `unauthorized_client`: wrong client ID or secret, or the client was deleted.
    case invalidClient(description: String?)
    /// Any other OAuth error code (`invalid_request`, `invalid_scope`, ...).
    case oauth(code: String, description: String?)
    /// 5xx, or a non-JSON error body.
    case server(status: Int)
    /// A 2xx answer without a usable `access_token`/`expires_in`.
    case malformedResponse
}

public enum GoogleTokenResponse {
    private struct Body: Decodable {
        var accessToken: String?
        var expiresIn: Double?
        var refreshToken: String?
        var scope: String?
        var error: String?
        var errorDescription: String?

        enum CodingKeys: String, CodingKey {
            case accessToken = "access_token"
            case expiresIn = "expires_in"
            case refreshToken = "refresh_token"
            case scope
            case error
            case errorDescription = "error_description"
        }
    }

    /// Decodes a token endpoint answer (authorization-code or refresh grant).
    public static func decode(_ data: Data, status: Int) -> Result<GoogleTokenGrant, GoogleTokenError> {
        let body = try? JSONDecoder().decode(Body.self, from: data)
        if let code = body?.error, !code.isEmpty {
            let description = body?.errorDescription
            switch code {
            case "invalid_grant": return .failure(.invalidGrant(description: description))
            case "invalid_client", "unauthorized_client": return .failure(.invalidClient(description: description))
            default: return .failure(.oauth(code: code, description: description))
            }
        }
        guard (200...299).contains(status) else { return .failure(.server(status: status)) }
        guard let body, let accessToken = body.accessToken, !accessToken.isEmpty,
              let expiresIn = body.expiresIn, expiresIn > 0
        else { return .failure(.malformedResponse) }
        let scopes = Set((body.scope ?? "").split(separator: " ").map(String.init))
        let refreshToken = body.refreshToken.flatMap { $0.isEmpty ? nil : $0 }
        return .success(GoogleTokenGrant(
            accessToken: accessToken, expiresIn: expiresIn, refreshToken: refreshToken, scopes: scopes
        ))
    }
}

// MARK: - Client file

/// The parts of a Google Cloud "Desktop app" OAuth client JSON the app uses.
/// `description` never shows the client secret.
public struct GoogleOAuthClient: Sendable, Equatable, CustomStringConvertible, CustomDebugStringConvertible {
    public var clientID: String
    public var clientSecret: String

    public init(clientID: String, clientSecret: String) {
        self.clientID = clientID
        self.clientSecret = clientSecret
    }

    public var description: String { "GoogleOAuthClient(clientID: \(clientID), clientSecret: <redacted>)" }
    public var debugDescription: String { description }

    /// Upper bound for the client ID and secret lengths (real ones are about 75 and 35 characters).
    public static let maxFieldLength = 256
    public static let clientIDSuffix = ".apps.googleusercontent.com"

    /// `<digits>-<id>.apps.googleusercontent.com` spelled with `[A-Za-z0-9._-]` only.
    public static func isValidClientID(_ value: String) -> Bool {
        guard value.utf8.count <= maxFieldLength, value.hasSuffix(clientIDSuffix),
              value.utf8.count > clientIDSuffix.utf8.count
        else { return false }
        return value.utf8.allSatisfy { byte in
            switch byte {
            case 0x41...0x5A, 0x61...0x7A, 0x30...0x39, 0x2D, 0x2E, 0x5F: true
            default: false
            }
        }
    }

    /// Printable ASCII without spaces.
    static func isValidSecret(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= maxFieldLength && value.utf8.allSatisfy { (0x21...0x7E).contains($0) }
    }
}

/// Why a downloaded client JSON was refused.
public enum GoogleOAuthClientFileError: Error, Sendable, Equatable {
    /// Not a JSON object.
    case notJSON
    /// A "Web application" client (`"web"` section): it cannot use the loopback redirect.
    case webClient
    /// Neither `"installed"` nor `"web"`.
    case missingInstalledSection
    case missingField(String)
    case invalidField(String)
}

public enum GoogleOAuthClientFile {
    /// Hosts the file's `auth_uri`/`token_uri` may point at (the app always uses its own constants).
    static let allowedHostSuffixes = ["google.com", "googleapis.com"]

    /// Parses the `"installed"` section of a client JSON downloaded from Google Cloud Console.
    public static func parse(_ data: Data) throws(GoogleOAuthClientFileError) -> GoogleOAuthClient {
        guard let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw .notJSON
        }
        guard let installed = object["installed"] as? [String: Any] else {
            if object["web"] != nil { throw .webClient }
            throw .missingInstalledSection
        }
        let clientID = try field("client_id", in: installed)
        guard GoogleOAuthClient.isValidClientID(clientID) else { throw .invalidField("client_id") }
        let secret = try field("client_secret", in: installed)
        guard GoogleOAuthClient.isValidSecret(secret) else { throw .invalidField("client_secret") }
        for key in ["auth_uri", "token_uri"] {
            guard let value = installed[key] else { continue }
            guard let text = value as? String, isGoogleHTTPS(text) else { throw .invalidField(key) }
        }
        return GoogleOAuthClient(clientID: clientID, clientSecret: secret)
    }

    private static func field(_ key: String, in section: [String: Any]) throws(GoogleOAuthClientFileError) -> String {
        guard let value = section[key] else { throw .missingField(key) }
        guard let text = value as? String, !text.isEmpty else { throw .invalidField(key) }
        return text
    }

    /// `https://` on a Google host: a tampered file must not be able to point the flow elsewhere.
    static func isGoogleHTTPS(_ text: String) -> Bool {
        guard let components = URLComponents(string: text), components.scheme == "https",
              let host = components.host?.lowercased(), components.user == nil, components.password == nil
        else { return false }
        return allowedHostSuffixes.contains { host == $0 || host.hasSuffix("." + $0) }
    }
}

private extension String {
    func leftPadded(to width: Int) -> String {
        count >= width ? self : String(repeating: "0", count: width - count) + self
    }
}
