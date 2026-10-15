import Foundation
import Testing
import FortiShared
@testable import FortiCore

@Suite struct ConfigRendererTests {
    typealias Problem = ConfigRenderer.ValidationError.Problem
    typealias Field = ConfigRenderer.ValidationError.Field

    static let digestA = "4d1b72371c42a69bd949048883897aa673059f2e7b1cc992a650584132aae8f3"
    static let digestB = "e0ef56d6eb603d9ffe87e74c28b8210b0ea93839e2ea4fb64f1fc2c66e95e391"
    static let digestC = "39610bca9443fdb6ae772d781954447c0bb23c05bcb2f08e72d6cc5f70209726"
    static let attemptID = UUID(uuidString: "6F9619FF-8B86-D011-B42D-00C04FC964FF")!

    static func request(
        host: String = "vpn.example.com",
        port: Int = 8443,
        username: String = "alice",
        realm: String? = nil,
        trustedCerts: [String] = [digestA],
        password: String = "s3cr3t"
    ) -> ConnectRequest {
        ConnectRequest(
            attemptID: attemptID, host: host, port: port, username: username,
            realm: realm, trustedCerts: trustedCerts, password: password
        )
    }

    private func expectRejected(_ request: ConnectRequest, _ field: Field, _ problem: Problem, sourceLocation: SourceLocation = #_sourceLocation) {
        let expected = ConfigRenderer.ValidationError(field: field, problem: problem)
        #expect(throws: expected, sourceLocation: sourceLocation) { try ConfigRenderer.render(request) }
        #expect(throws: expected, sourceLocation: sourceLocation) { try ConfigRenderer.validate(request) }
    }

    // MARK: - Output

    @Test func rendersMinimalRequestExactly() throws {
        let text = try ConfigRenderer.render(Self.request())
        #expect(text == """
            host = vpn.example.com
            port = 8443
            username = alice
            trusted-cert = \(Self.digestA)

            """)
    }

    @Test func rendersRealmAndOneLinePerDigestInOrder() throws {
        let text = try ConfigRenderer.render(Self.request(
            host: "203.0.113.10", port: 443, realm: "staff", trustedCerts: [Self.digestB, Self.digestA, Self.digestC]
        ))
        #expect(text == """
            host = 203.0.113.10
            port = 443
            username = alice
            realm = staff
            trusted-cert = \(Self.digestB)
            trusted-cert = \(Self.digestA)
            trusted-cert = \(Self.digestC)

            """)
    }

    @Test func rendersWithoutDigestsForTheFirstConnection() throws {
        let text = try ConfigRenderer.render(Self.request(trustedCerts: []))
        #expect(text == "host = vpn.example.com\nport = 8443\nusername = alice\n")
    }

    @Test(arguments: [nil, ""] as [String?])
    func omitsMissingOrEmptyRealm(_ realm: String?) throws {
        let text = try ConfigRenderer.render(Self.request(realm: realm))
        #expect(!text.contains("realm"))
    }

    @Test func neverWritesThePasswordOrAnyOtherKey() throws {
        let password = "Pa55word-that-must-not-leak"
        let text = try ConfigRenderer.render(Self.request(
            realm: "staff", trustedCerts: [Self.digestA, Self.digestB], password: password
        ))
        #expect(!text.contains(password))
        #expect(!text.contains("password"))
        let keys = text.split(separator: "\n").map { $0.split(separator: "=", maxSplits: 1)[0].trimmingCharacters(in: .whitespaces) }
        #expect(Set(keys).isSubset(of: ["host", "port", "username", "realm", "trusted-cert"]))
        #expect(text.hasSuffix("\n"))
        #expect(!text.contains("\r"))
    }

    static let roundTripRequests: [ConnectRequest] = [
        request(),
        request(host: "203.0.113.10", port: 1, realm: "staff", trustedCerts: [digestA, digestB]),
        request(host: "VPN-1.Corp.Example.COM", port: 65535, username: "bob=x#y", realm: "r=1#2", trustedCerts: []),
        request(username: "#alice smith", realm: "\u{C9C1}\u{C6D0}", trustedCerts: [digestC, digestB, digestA]),
        request(username: String(repeating: "a", count: 64), realm: String(repeating: "r", count: 63)),
        request(host: String(repeating: "a.", count: 126) + "a", username: String(repeating: "\u{AC00}", count: 21)),
    ]

    @Test(arguments: roundTripRequests)
    func renderedFileReadsBackUnchanged(_ request: ConnectRequest) throws {
        let text = try ConfigRenderer.render(request)

        let imported = OpenFortiVPNConfigReader.parse(text)
        #expect(imported == ImportedOpenFortiVPNConfig(
            host: request.host,
            port: request.port,
            username: request.username,
            realm: request.realm,
            trustedCerts: request.trustedCerts
        ))

        // Same check against the exact load_config() model.
        let entries = OpenFortiVPNConfigSyntax.entriesAsReadByOpenFortiVPN(Array(text.utf8)).map {
            [String(decoding: $0.key, as: UTF8.self), String(decoding: $0.value, as: UTF8.self)]
        }
        var expected = [["host", request.host], ["port", String(request.port)], ["username", request.username]]
        if let realm = request.realm { expected.append(["realm", realm]) }
        expected += request.trustedCerts.map { ["trusted-cert", $0] }
        #expect(entries == expected)
    }

    @Test func eightDigestsAreAllowed() throws {
        let digests = (0..<8).map { String(repeating: String($0), count: 64) }
        let text = try ConfigRenderer.render(Self.request(trustedCerts: digests))
        #expect(text.components(separatedBy: "trusted-cert = ").count == 9)
    }

    // MARK: - Host

    @Test(arguments: [
        "vpn.example.com", "VPN.Example.COM", "vpn", "localhost", "vpn-1.example.com", "10.example.com",
        "203.0.113.10", "0.0.0.0", "255.255.255.255", "1.2.3.4",
        String(repeating: "a", count: 63) + ".com",
        String(repeating: "a.", count: 126) + "a",  // 253 bytes
    ])
    func acceptsHost(_ host: String) throws {
        try ConfigRenderer.validate(Self.request(host: host))
        try ConfigRenderer.validateHost(host)
    }

    @Test(arguments: [
        ("", .empty),
        (String(repeating: "a.", count: 126) + "ab", .tooLong(maxBytes: 253)),
        (" vpn.example.com", .leadingOrTrailingWhitespace),
        ("vpn.example.com ", .leadingOrTrailingWhitespace),
        ("vpn.example.com\u{A0}", .leadingOrTrailingWhitespace),
        ("vpn.example.com\npppd-plugin = /tmp/x.so", .forbiddenCharacter(codePoint: 0x0A)),
        ("vpn.example.com\r", .forbiddenCharacter(codePoint: 0x0D)),
        ("vpn.example.com\t", .forbiddenCharacter(codePoint: 0x09)),
        ("vpn\0.example.com", .forbiddenCharacter(codePoint: 0x00)),
        ("vpn.example.com\u{85}", .forbiddenCharacter(codePoint: 0x85)),
        ("vpn.example.com\u{2028}x", .forbiddenCharacter(codePoint: 0x2028)),
        ("vpn.example.com\u{2029}x", .forbiddenCharacter(codePoint: 0x2029)),
        ("\u{FEFF}vpn.example.com", .forbiddenCharacter(codePoint: 0xFEFF)),
        ("vpn example.com", .invalidHost),
        ("-vpn.example.com", .invalidHost),
        ("vpn-.example.com", .invalidHost),
        ("vpn..example.com", .invalidHost),
        (".vpn.example.com", .invalidHost),
        ("vpn.example.com.", .invalidHost),
        ("vpn_1.example.com", .invalidHost),
        (String(repeating: "a", count: 64) + ".com", .invalidHost),
        ("vpn.example.com:443", .invalidHost),
        ("vpn.ex#ample.com", .invalidHost),
        ("host=evil.example.com", .invalidHost),
        ("vpn.exämple.com", .invalidHost),
        ("ｖｐｎ.example.com", .invalidHost),
        ("::1", .invalidHost),
        ("[::1]", .invalidHost),
        ("1.2.3", .invalidHost),
        ("127.1", .invalidHost),
        ("1.2.3.4.5", .invalidHost),
        ("256.1.1.1", .invalidHost),
        ("999.1.1.1", .invalidHost),
        ("010.0.0.1", .invalidHost),
        ("1.2.3.04", .invalidHost),
        ("0x7f000001", .invalidHost),
        ("0x7f.1", .invalidHost),
        ("10.0.0.0x1", .invalidHost),
    ] as [(String, Problem)])
    func rejectsHost(_ host: String, _ problem: Problem) {
        expectRejected(Self.request(host: host), .host, problem)
        #expect(throws: ConfigRenderer.ValidationError(field: .host, problem: problem)) { try ConfigRenderer.validateHost(host) }
    }

    /// A gateway pasted from the browser must be caught by the app (Account tab, readiness) with
    /// the same rule the helper applies, not after a Gmail login.
    @Test(arguments: [
        "https://vpn.example.com", "https://vpn.example.com/", "http://vpn.example.com:10443",
        "vpn.example.com:10443", "vpn.example.com/remote/login", "vpn.example.com.",
    ])
    func validateHostRejectsURLsAndPorts(_ host: String) {
        #expect(throws: ConfigRenderer.ValidationError(field: .host, problem: .invalidHost)) { try ConfigRenderer.validateHost(host) }
        expectRejected(Self.request(host: host), .host, .invalidHost)
    }

    // MARK: - Port

    @Test(arguments: [1, 443, 8443, 65535])
    func acceptsPort(_ port: Int) throws {
        try ConfigRenderer.validate(Self.request(port: port))
    }

    @Test(arguments: [0, -1, 65536, Int.max, Int.min])
    func rejectsPort(_ port: Int) {
        expectRejected(Self.request(port: port), .port, .outOfRange)
    }

    // MARK: - Username

    @Test(arguments: [
        "a", String(repeating: "a", count: 64), String(repeating: "\u{AC00}", count: 21), "alice smith", "bob=x",
        "#bob", "bob#1", "alice@corp.example", "名前", "DOMAIN\\alice",
    ])
    func acceptsUsername(_ username: String) throws {
        try ConfigRenderer.validate(Self.request(username: username))
        try ConfigRenderer.validateUsername(username)
    }

    @Test(arguments: [
        ("", .empty),
        (String(repeating: "a", count: 65), .tooLong(maxBytes: 64)),
        (String(repeating: "\u{AC00}", count: 22), .tooLong(maxBytes: 64)),  // 66 bytes
        ("bob\npppd-plugin = /tmp/x.so", .forbiddenCharacter(codePoint: 0x0A)),
        ("bob\r\npppd-plugin = /tmp/x.so", .forbiddenCharacter(codePoint: 0x0D)),
        ("bob\rpppd-plugin = /tmp/x.so", .forbiddenCharacter(codePoint: 0x0D)),
        ("bob\0", .forbiddenCharacter(codePoint: 0x00)),
        ("bob\u{0B}x", .forbiddenCharacter(codePoint: 0x0B)),
        ("bob\u{0C}x", .forbiddenCharacter(codePoint: 0x0C)),
        ("bob\tx", .forbiddenCharacter(codePoint: 0x09)),
        ("bob\u{1B}[31m", .forbiddenCharacter(codePoint: 0x1B)),
        ("bob\u{7F}", .forbiddenCharacter(codePoint: 0x7F)),
        ("bob\u{85}x", .forbiddenCharacter(codePoint: 0x85)),
        ("bob\u{9F}x", .forbiddenCharacter(codePoint: 0x9F)),
        ("bob\u{2028}x", .forbiddenCharacter(codePoint: 0x2028)),
        ("bob\u{2029}x", .forbiddenCharacter(codePoint: 0x2029)),
        ("\u{FEFF}bob", .forbiddenCharacter(codePoint: 0xFEFF)),
        (" bob", .leadingOrTrailingWhitespace),
        ("bob ", .leadingOrTrailingWhitespace),
        ("\u{A0}bob", .leadingOrTrailingWhitespace),
        ("bob\u{3000}", .leadingOrTrailingWhitespace),
        ("bob\u{2003}", .leadingOrTrailingWhitespace),
    ] as [(String, Problem)])
    func rejectsUsername(_ username: String, _ problem: Problem) {
        expectRejected(Self.request(username: username), .username, problem)
        #expect(throws: ConfigRenderer.ValidationError(field: .username, problem: problem)) {
            try ConfigRenderer.validateUsername(username)
        }
    }

    // MARK: - Realm

    @Test(arguments: [
        (String(repeating: "r", count: 64), .tooLong(maxBytes: 63)),
        ("staff\nhost = evil.example.com", .forbiddenCharacter(codePoint: 0x0A)),
        ("staff\0", .forbiddenCharacter(codePoint: 0x00)),
        ("staff\u{2028}", .forbiddenCharacter(codePoint: 0x2028)),
        (" staff", .leadingOrTrailingWhitespace),
        ("staff ", .leadingOrTrailingWhitespace),
    ] as [(String, Problem)])
    func rejectsRealm(_ realm: String, _ problem: Problem) {
        expectRejected(Self.request(realm: realm), .realm, problem)
        #expect(throws: ConfigRenderer.ValidationError(field: .realm, problem: problem)) { try ConfigRenderer.validateRealm(realm) }
    }

    @Test(arguments: [nil, "", "staff", String(repeating: "r", count: 63)] as [String?])
    func acceptsRealm(_ realm: String?) throws {
        try ConfigRenderer.validateRealm(realm)
        try ConfigRenderer.validate(Self.request(realm: realm))
    }

    // MARK: - Trusted certificates

    @Test(arguments: [
        ([digestA.uppercased()], .invalidDigest(index: 0)),
        ([String(digestA.dropLast())], .invalidDigest(index: 0)),
        ([digestA + "0"], .invalidDigest(index: 0)),
        ([String(digestA.dropLast()) + "g"], .invalidDigest(index: 0)),
        ([digestA + " "], .invalidDigest(index: 0)),
        ([" " + String(digestA.dropFirst())], .invalidDigest(index: 0)),
        ([""], .invalidDigest(index: 0)),
        ([digestA + "\nhost = evil.example.com"], .invalidDigest(index: 0)),
        ([String(digestA.prefix(63)) + "\n"], .invalidDigest(index: 0)),
        ([digestA, digestB, "sha256:" + digestC], .invalidDigest(index: 2)),
        ([digestA, String(repeating: "０", count: 64)], .invalidDigest(index: 1)),
        (Array(repeating: digestA, count: 9), .tooManyDigests(max: 8)),
    ] as [([String], Problem)])
    func rejectsTrustedCerts(_ digests: [String], _ problem: Problem) {
        expectRejected(Self.request(trustedCerts: digests), .trustedCert, problem)
    }

    // MARK: - Password

    @Test(arguments: [
        "s3cr3t", String(repeating: "a", count: 255), String(repeating: "\u{AC00}", count: 85),  // 255 bytes
        " spaced password ", "p=ss#word", "tab\tinside", "ünïcödé-pässwörd", "\u{2028}",
    ])
    func acceptsPassword(_ password: String) throws {
        try ConfigRenderer.validatePassword(password)
        try ConfigRenderer.validate(Self.request(password: password))
    }

    @Test(arguments: [
        ("", .empty),
        ("pass\nword", .forbiddenCharacter(codePoint: 0x0A)),
        ("password\n", .forbiddenCharacter(codePoint: 0x0A)),
        ("pass\rword", .forbiddenCharacter(codePoint: 0x0D)),
        ("pass\r\nword", .forbiddenCharacter(codePoint: 0x0D)),
        ("pass\0word", .forbiddenCharacter(codePoint: 0x00)),
        (String(repeating: "a", count: 256), .tooLong(maxBytes: 255)),
        (String(repeating: "\u{AC00}", count: 86), .tooLong(maxBytes: 255)),  // 258 bytes
    ] as [(String, Problem)])
    func rejectsPassword(_ password: String, _ problem: Problem) {
        let expected = ConfigRenderer.ValidationError(field: .password, problem: problem)
        #expect(throws: expected) { try ConfigRenderer.validatePassword(password) }
        // render() validates the whole request, so the helper cannot spawn with an un-injectable password.
        expectRejected(Self.request(password: password), .password, problem)
    }

    // MARK: - Errors

    @Test func reportsTheFirstInvalidFieldInFileOrder() {
        let request = Self.request(host: "bad host", port: 0, username: "", trustedCerts: ["x"], password: "")
        #expect(throws: ConfigRenderer.ValidationError(field: .host, problem: .invalidHost)) {
            try ConfigRenderer.render(request)
        }
    }

    @Test func errorDescriptionsNameTheFieldButNeverTheValue() throws {
        let secret = "hunter2-very-secret"
        let cases: [(ConnectRequest, String)] = [
            (Self.request(password: secret + "\n"), "password contains forbidden character U+000A"),
            (Self.request(password: String(repeating: secret, count: 20)), "password must be at most 255 UTF-8 bytes"),
            (Self.request(username: secret + "\npppd-plugin = /tmp/x.so"), "username contains forbidden character U+000A"),
            (Self.request(username: secret + "\u{2028}"), "username contains forbidden character U+2028"),
            (Self.request(host: secret + "_host"), "host must be a DNS hostname (letters, digits and hyphens, labels of 1-63) or a dotted-quad IPv4 address"),
            (Self.request(realm: " " + secret), "realm must not start or end with whitespace"),
            (Self.request(port: 0), "port must be between 1 and 65535"),
            (Self.request(trustedCerts: [Self.digestA, secret]), "trusted-cert entry 1 is not 64 lowercase hex characters"),
            (Self.request(trustedCerts: Array(repeating: Self.digestA, count: 9)), "trusted-cert allows at most 8 digests"),
            (Self.request(username: ""), "username must not be empty"),
        ]
        for (request, message) in cases {
            let error = try #require(throws: ConfigRenderer.ValidationError.self) { try ConfigRenderer.render(request) }
            #expect(error.description == message)
            #expect(!error.description.contains(secret))
            #expect(!String(reflecting: error).contains(secret))
        }
        #expect(ConfigRenderer.ValidationError(field: .host, problem: .notRoundTrippable).description
            == "host would not be read back unchanged by openfortivpn")
    }
}
