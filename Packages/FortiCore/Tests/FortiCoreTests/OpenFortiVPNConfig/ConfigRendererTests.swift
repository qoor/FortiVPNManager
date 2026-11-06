import Testing
@testable import FortiCore

@Suite struct ConfigRendererTests {
    typealias Problem = ConfigRenderer.ValidationError.Problem

    // MARK: - Host

    @Test(arguments: [
        "vpn.example.com", "VPN.Example.COM", "vpn", "localhost", "vpn-1.example.com", "10.example.com",
        "203.0.113.10", "0.0.0.0", "255.255.255.255", "1.2.3.4",
        String(repeating: "a", count: 63) + ".com",
        String(repeating: "a.", count: 126) + "a",  // 253 bytes
    ])
    func acceptsHost(_ host: String) throws {
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
    }

    // MARK: - Username

    @Test(arguments: [
        "a", String(repeating: "a", count: 64), String(repeating: "\u{AC00}", count: 21), "alice smith", "bob=x",
        "#bob", "bob#1", "alice@corp.example", "名前", "DOMAIN\\alice",
    ])
    func acceptsUsername(_ username: String) throws {
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
        #expect(throws: ConfigRenderer.ValidationError(field: .realm, problem: problem)) { try ConfigRenderer.validateRealm(realm) }
    }

    @Test(arguments: [nil, "", "staff", String(repeating: "r", count: 63)] as [String?])
    func acceptsRealm(_ realm: String?) throws {
        try ConfigRenderer.validateRealm(realm)
    }

    // MARK: - Password

    @Test(arguments: [
        "s3cr3t", String(repeating: "a", count: 255), String(repeating: "\u{AC00}", count: 85),  // 255 bytes
        " spaced password ", "p=ss#word", "tab\tinside", "ünïcödé-pässwörd", "\u{2028}",
    ])
    func acceptsPassword(_ password: String) throws {
        try ConfigRenderer.validatePassword(password)
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
    }

    // MARK: - Errors

    @Test func errorDescriptionsNameTheFieldButNeverTheValue() throws {
        let secret = "hunter2-very-secret"
        let cases: [(() throws -> Void, String)] = [
            ({ try ConfigRenderer.validatePassword(secret + "\n") }, "password contains forbidden character U+000A"),
            ({ try ConfigRenderer.validatePassword(String(repeating: secret, count: 20)) }, "password must be at most 255 UTF-8 bytes"),
            ({ try ConfigRenderer.validateUsername(secret + "\npppd-plugin = /tmp/x.so") }, "username contains forbidden character U+000A"),
            ({ try ConfigRenderer.validateUsername(secret + "\u{2028}") }, "username contains forbidden character U+2028"),
            ({ try ConfigRenderer.validateHost(secret + "_host") }, "host must be a DNS hostname (letters, digits and hyphens, labels of 1-63) or a dotted-quad IPv4 address"),
            ({ try ConfigRenderer.validateRealm(" " + secret) }, "realm must not start or end with whitespace"),
            ({ try ConfigRenderer.validateUsername("") }, "username must not be empty"),
        ]
        for (check, message) in cases {
            let error = try #require(throws: ConfigRenderer.ValidationError.self) { try check() }
            #expect(error.description == message)
            #expect(!error.description.contains(secret))
            #expect(!String(reflecting: error).contains(secret))
        }
    }
}
