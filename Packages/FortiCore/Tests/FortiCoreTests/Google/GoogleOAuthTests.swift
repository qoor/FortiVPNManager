import Foundation
import Testing
@testable import FortiCore

/// Splits an `application/x-www-form-urlencoded` body into decoded pairs (test helper).
func formPairs(_ data: Data) -> [String: String] {
    let request = Data("GET /?\(String(decoding: data, as: UTF8.self)) HTTP/1.1\r\n\r\n".utf8)
    guard case let .request(parsed) = LoopbackHTTPRequest.parse(request) else { return [:] }
    return Dictionary(parsed.queryItems.map { ($0.name, $0.value) }, uniquingKeysWith: { first, _ in first })
}

@Suite("Google OAuth: PKCE and URLs")
struct GoogleOAuthPKCETests {
    /// RFC 7636 Appendix B.
    @Test func rfc7636Vector() throws {
        let pkce = try #require(PKCE(verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"))
        #expect(pkce.challenge == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        #expect(PKCE.method == "S256")
    }

    @Test func generatedVerifiersAreLongRandomAndUnreserved() {
        let first = PKCE.generate()
        let second = PKCE.generate()
        #expect(first.verifier.count == 64)
        #expect(PKCE.verifierLengths.contains(first.verifier.count))
        #expect(first.verifier != second.verifier)
        #expect(first.verifier.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") })
        #expect(first.challenge.count == 43)
        #expect(!first.challenge.contains("=") && !first.challenge.contains("+") && !first.challenge.contains("/"))
        #expect(first.challenge == PKCE.challenge(for: first.verifier))
    }

    @Test("Verifier length and alphabet", arguments: [
        (String(repeating: "a", count: 42), false),
        (String(repeating: "a", count: 43), true),
        (String(repeating: "~", count: 128), true),
        (String(repeating: "a", count: 129), false),
        (String(repeating: "a", count: 42) + "=", false),
        (String(repeating: "a", count: 42) + " ", false),
        (String(repeating: "a", count: 42) + "\u{E9}", false),
        ("A-._~" + String(repeating: "0", count: 40), true),
    ])
    func verifierRules(verifier: String, valid: Bool) {
        #expect((PKCE(verifier: verifier) != nil) == valid)
    }

    @Test func base64URLHasNoPaddingOrStandardSymbols() {
        #expect(Base64URL.encode([0xFB, 0xFF]) == "-_8")
        #expect(Base64URL.encode([UInt8]()) == "")
        #expect(Base64URL.encode(Data("any carnal pleas".utf8)) == "YW55IGNhcm5hbCBwbGVhcw")
    }

    @Test func statesAreRandomBase64URL() {
        let state = GoogleOAuth.makeState()
        #expect(state.count == 32)
        #expect(state != GoogleOAuth.makeState())
        #expect(state.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") })
    }

    @Test(arguments: [
        ("abcXYZ019-._~", "abcXYZ019-._~"),
        ("a b+c/d@e:f?g&h=i", "a%20b%2Bc%2Fd%40e%3Af%3Fg%26h%3Di"),
        ("\u{D55C}", "%ED%95%9C"),
        ("\n", "%0A"),
    ])
    func percentEncoding(raw: String, encoded: String) {
        #expect(GoogleOAuth.percentEncoded(raw) == encoded)
    }

    @Test func loopbackRedirectURI() {
        #expect(GoogleOAuth.loopbackRedirectURI(port: 53122) == "http://127.0.0.1:53122")
    }

    @Test func authorizationURLCarriesEveryParameter() throws {
        let url = GoogleOAuth.authorizationURL(
            clientID: "123-abc.apps.googleusercontent.com",
            redirectURI: "http://127.0.0.1:53122",
            codeChallenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            state: "st-ate_1",
            loginHint: "me+otp@example.com"
        )
        #expect(url.absoluteString.hasPrefix("https://accounts.google.com/o/oauth2/v2/auth?"))
        // "+" must be escaped or Google reads a space.
        #expect(url.absoluteString.contains("login_hint=me%2Botp%40example.com"))
        let components = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false))
        let items = Dictionary(uniqueKeysWithValues: (components.queryItems ?? []).map { ($0.name, $0.value ?? "") })
        #expect(items == [
            "client_id": "123-abc.apps.googleusercontent.com",
            "redirect_uri": "http://127.0.0.1:53122",
            "response_type": "code",
            "scope": "https://www.googleapis.com/auth/gmail.modify",
            "code_challenge": "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            "code_challenge_method": "S256",
            "state": "st-ate_1",
            "access_type": "offline",
            "prompt": "consent",
            "login_hint": "me+otp@example.com",
        ])
    }

    @Test(arguments: [nil, ""] as [String?])
    func loginHintIsOptional(hint: String?) throws {
        let url = GoogleOAuth.authorizationURL(
            clientID: "c", redirectURI: "http://127.0.0.1:1", codeChallenge: "x", state: "s", loginHint: hint
        )
        let names = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems).map(\.name)
        #expect(!names.contains("login_hint"))
        #expect(names.count == 9)
    }

    @Test func tokenRequestBodies() {
        let exchange = formPairs(GoogleOAuth.authorizationCodeRequestBody(
            code: "4/0Ab_cd+ef", clientID: "cid", clientSecret: "s3cr&t", redirectURI: "http://127.0.0.1:9",
            codeVerifier: "verifier"
        ))
        #expect(exchange == [
            "grant_type": "authorization_code", "code": "4/0Ab_cd+ef", "client_id": "cid", "client_secret": "s3cr&t",
            "redirect_uri": "http://127.0.0.1:9", "code_verifier": "verifier",
        ])
        let refresh = formPairs(GoogleOAuth.refreshRequestBody(refreshToken: "1//rt", clientID: "cid", clientSecret: "sec"))
        #expect(refresh == ["grant_type": "refresh_token", "refresh_token": "1//rt", "client_id": "cid", "client_secret": "sec"])
        #expect(String(decoding: GoogleOAuth.revocationRequestBody(token: "1//rt"), as: UTF8.self) == "token=1%2F%2Frt")
    }
}

@Suite("Google OAuth: token responses")
struct GoogleTokenResponseTests {
    private func decode(_ json: String, status: Int = 200) -> Result<GoogleTokenGrant, GoogleTokenError> {
        GoogleTokenResponse.decode(Data(json.utf8), status: status)
    }

    @Test func authorizationCodeGrant() throws {
        let json = """
            {"access_token": "ya29.a0Af", "expires_in": 3599, "refresh_token": "1//0g-rt",
             "scope": "https://www.googleapis.com/auth/gmail.modify openid", "token_type": "Bearer"}
            """
        let grant = try decode(json).get()
        #expect(grant == GoogleTokenGrant(
            accessToken: "ya29.a0Af", expiresIn: 3599, refreshToken: "1//0g-rt",
            scopes: ["https://www.googleapis.com/auth/gmail.modify", "openid"]
        ))
        #expect(grant.grants(GoogleOAuth.gmailModifyScope))
        #expect(!grant.description.contains("ya29") && !grant.description.contains("1//0g"))
    }

    @Test func refreshGrantHasNoRefreshToken() throws {
        let grant = try decode(#"{"access_token": "ya29.b", "expires_in": 3599, "token_type": "Bearer"}"#).get()
        #expect(grant.refreshToken == nil)
        #expect(grant.scopes.isEmpty)
        #expect(try decode(#"{"access_token": "a", "expires_in": 10, "refresh_token": ""}"#).get().refreshToken == nil)
    }

    @Test("Error answers", arguments: [
        (#"{"error": "invalid_grant", "error_description": "Token has been expired or revoked."}"#, 400,
         GoogleTokenError.invalidGrant(description: "Token has been expired or revoked.")),
        (#"{"error": "invalid_grant"}"#, 400, .invalidGrant(description: nil)),
        (#"{"error": "invalid_client", "error_description": "Unauthorized"}"#, 401, .invalidClient(description: "Unauthorized")),
        (#"{"error": "unauthorized_client"}"#, 400, .invalidClient(description: nil)),
        (#"{"error": "invalid_request", "error_description": "Missing code verifier."}"#, 400,
         .oauth(code: "invalid_request", description: "Missing code verifier.")),
        ("<html>Bad Gateway</html>", 502, .server(status: 502)),
        ("{}", 500, .server(status: 500)),
        ("{}", 200, .malformedResponse),
        (#"{"access_token": "", "expires_in": 3599}"#, 200, .malformedResponse),
        (#"{"access_token": "a", "expires_in": 0}"#, 200, .malformedResponse),
        (#"{"access_token": "a"}"#, 200, .malformedResponse),
        ("not json", 200, .malformedResponse),
    ])
    func errors(json: String, status: Int, expected: GoogleTokenError) {
        #expect(decode(json, status: status) == .failure(expected))
    }
}

@Suite("Google OAuth: client JSON")
struct GoogleOAuthClientFileTests {
    static let clientID = "123456789012-abcdefghijklmnopqrstuvwxyz012345.apps.googleusercontent.com"
    static let secret = "GOCSPX-Test_Only-NotARealSecret12345"

    static func installed(_ fields: [String: Any]) -> Data {
        try! JSONSerialization.data(withJSONObject: ["installed": fields])
    }

    static var complete: [String: Any] { [
        "client_id": clientID,
        "project_id": "example-123",
        "auth_uri": "https://accounts.google.com/o/oauth2/auth",
        "token_uri": "https://oauth2.googleapis.com/token",
        "auth_provider_x509_cert_url": "https://www.googleapis.com/oauth2/v1/certs",
        "client_secret": secret,
        "redirect_uris": ["http://localhost"],
    ] }

    @Test func desktopClient() throws {
        let client = try GoogleOAuthClientFile.parse(Self.installed(Self.complete))
        #expect(client == GoogleOAuthClient(clientID: Self.clientID, clientSecret: Self.secret))
        #expect(!client.description.contains(Self.secret))
        #expect(!String(describing: client).contains(Self.secret))
        #expect(!String(reflecting: client).contains(Self.secret))
        #expect(client.description.contains(Self.clientID))
    }

    @Test func minimalDesktopClient() throws {
        let client = try GoogleOAuthClientFile.parse(Self.installed(["client_id": Self.clientID, "client_secret": Self.secret]))
        #expect(client.clientID == Self.clientID)
    }

    @Test func webClientIsRefused() throws {
        let data = try JSONSerialization.data(withJSONObject: ["web": Self.complete])
        #expect(throws: GoogleOAuthClientFileError.webClient) { try GoogleOAuthClientFile.parse(data) }
    }

    @Test("Not a client file", arguments: ["", "[]", "\"installed\"", "{", "null"])
    func notJSON(text: String) {
        #expect(throws: GoogleOAuthClientFileError.notJSON) { try GoogleOAuthClientFile.parse(Data(text.utf8)) }
    }

    @Test func missingSection() {
        #expect(throws: GoogleOAuthClientFileError.missingInstalledSection) {
            try GoogleOAuthClientFile.parse(Data(#"{"other": {}}"#.utf8))
        }
        #expect(throws: GoogleOAuthClientFileError.missingInstalledSection) {
            try GoogleOAuthClientFile.parse(Data(#"{"installed": "x"}"#.utf8))
        }
    }

    @Test("Field problems", arguments: [
        ("client_id", nil, GoogleOAuthClientFileError.missingField("client_id")),
        ("client_secret", nil, .missingField("client_secret")),
        ("client_id", "", .invalidField("client_id")),
        ("client_id", "123-abc", .invalidField("client_id")),
        ("client_id", ".apps.googleusercontent.com", .invalidField("client_id")),
        ("client_id", "123 abc.apps.googleusercontent.com", .invalidField("client_id")),
        ("client_id", "123-abc.apps.googleusercontent.com\n", .invalidField("client_id")),
        ("client_secret", "", .invalidField("client_secret")),
        ("client_secret", "has space", .invalidField("client_secret")),
        ("client_secret", "tab\t", .invalidField("client_secret")),
        ("token_uri", "http://oauth2.googleapis.com/token", .invalidField("token_uri")),
        ("token_uri", "https://oauth2.googleapis.com.evil.example/token", .invalidField("token_uri")),
        ("token_uri", "https://user@oauth2.googleapis.com/token", .invalidField("token_uri")),
        ("auth_uri", "https://evilgoogle.com/auth", .invalidField("auth_uri")),
        ("auth_uri", "", .invalidField("auth_uri")),
    ] as [(String, String?, GoogleOAuthClientFileError)])
    func fieldProblems(key: String, value: String?, expected: GoogleOAuthClientFileError) {
        var fields = Self.complete
        fields[key] = value
        #expect(throws: expected) { try GoogleOAuthClientFile.parse(Self.installed(fields)) }
    }

    @Test func nonStringFieldIsInvalid() {
        var fields = Self.complete
        fields["client_secret"] = 42
        #expect(throws: GoogleOAuthClientFileError.invalidField("client_secret")) {
            try GoogleOAuthClientFile.parse(Self.installed(fields))
        }
    }

    @Test func clientIDRules() {
        #expect(GoogleOAuthClient.isValidClientID(Self.clientID))
        #expect(GoogleOAuthClient.isValidClientID("1-a.apps.googleusercontent.com"))
        #expect(!GoogleOAuthClient.isValidClientID(""))
        #expect(!GoogleOAuthClient.isValidClientID("1-a.apps.googleusercontent.com.example"))
        #expect(!GoogleOAuthClient.isValidClientID(String(repeating: "a", count: 250) + ".apps.googleusercontent.com"))
    }
}
