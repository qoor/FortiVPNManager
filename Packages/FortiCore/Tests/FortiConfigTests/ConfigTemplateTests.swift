import Foundation
import Testing
@testable import FortiConfig

@Suite struct ConfigTemplateTests {
    @Test func defaultTemplateDecodesToDefaults() throws {
        let result = try ConfigCodec.decode(ConfigCodec.encode(AppConfig()))
        #expect(result.config == AppConfig())
        #expect(result.warnings.isEmpty)
    }

    @Test func defaultTemplateBody() {
        let text = ConfigTemplate.render(AppConfig())
        let body = text.split(separator: "\n", omittingEmptySubsequences: false)
            .filter { !$0.hasPrefix("#") && !$0.trimmingCharacters(in: .whitespaces).hasPrefix("#") }
            .joined(separator: "\n")
        #expect(body == """
            schema_version: 1
            vpn:
              host: ""
              port: 443
              username: ""
              realm: ""
              trusted_certs: []
            otp:
              gmail_address: ""
              method: "imap"
              google_client_id: ""
            connection:
              auto_connect_on_launch: true
              session_max_minutes: 180
              proactive_reconnect: { enabled: true, margin_minutes: 5 }
              reconnect_after_sleep: { enabled: false, min_sleep_minutes: 30 }

            """)
        #expect(text.hasSuffix("\n"))
    }

    @Test func headerAndFieldComments() {
        let text = ConfigTemplate.render(AppConfig())
        let header = text.split(separator: "\n").prefix { $0.hasPrefix("#") }.joined(separator: "\n")
        #expect(header.contains("Keychain"))
        #expect(text.contains(
            "  # How the app reads the OTP emails: \"imap\" (Gmail app password) or \"gmail_api\" (Gmail API with Google "
                + "sign-in; falls back to IMAP when the sign-in fails and an app password is stored).\n  method: \"imap\"\n"))
        #expect(header.contains("Comments you added and unknown keys are removed then"))
        #expect(header.contains("Do not write leading zeros in numbers"))
        #expect(header.contains("public repository"))
        #expect(text.contains(
            "  # Filled in on first launch from an existing openfortivpn config, if any (e.g. 8443). "
                + "443 is the default when the file has no value.\n  port: 443\n"))
        #expect(text.contains(
            "  # SHA-256 digests of the server certificate. The app fills these in on the first connection "
                + "and manages them; no need to edit by hand.\n"
                + "  # When the certificate changes, the app automatically adds the new value at the front "
                + "(up to 4 entries).\n  trusted_certs: []\n"))
        // Every key has a comment line right above it except the schema version and nested flow keys.
        let lines = text.split(separator: "\n").map(String.init)
        for key in ["host:", "port:", "username:", "realm:", "gmail_address:", "method:", "google_client_id:",
                    "auto_connect_on_launch:",
                    "session_max_minutes:", "proactive_reconnect:", "reconnect_after_sleep:"] {
            let index = lines.firstIndex { $0.trimmingCharacters(in: .whitespaces).hasPrefix(key) }
            #expect(index.map { lines[$0 - 1].trimmingCharacters(in: .whitespaces).hasPrefix("#") } == true, "\(key)")
        }
    }

    @Test func trustedCertsAreABlockList() throws {
        let text = ConfigTemplate.render(nonDefaultConfig)
        #expect(text.contains("  trusted_certs:\n    - \"\(digestA)\"\n    - \"\(digestB)\"\notp:\n"))
        let result = try ConfigCodec.decode(Data(text.utf8))
        #expect(result.config == nonDefaultConfig)
        #expect(result.warnings.isEmpty)
    }

    static let trickyStrings: [String] = [
        "", " ", "  leading", "trailing  ", "\"", "\\", "\\\"", "'", "''", "#", " # not a comment", "a: b", "a:b",
        "- item", "? key", "|", ">", "&anchor", "*alias", "!tag", "!!str", "%YAML", "@at", "`tick`", "{a: b}", "[1, 2]",
        "---", "...", "~", "null", "true", "no", "0443", "0x1BB", "2:30", "1e3", ".inf", "<<", "\u{D55C}\u{AE00} realm",
        "e\u{301}", "🚀🇰🇷", "\u{A0}nbsp\u{A0}", "\u{E000}", "\u{FFFD}", "\u{10FFFD}", "a,b", "${HOME}",
        "user@example.com", "DOMAIN\\user", String(repeating: "x", count: 5000),
    ]

    @Test("String fields round-trip exactly", arguments: trickyStrings)
    func stringRoundTrip(value: String) throws {
        let config = AppConfig(
            vpn: AppConfig.VPN(host: value, username: value, realm: value),
            otp: AppConfig.OTP(gmailAddress: value, googleClientID: value))
        try config.validate()
        let rendered = ConfigTemplate.render(config)
        let result = try ConfigCodec.decode(Data(rendered.utf8))
        #expect(result.config == config)
        #expect(result.warnings.isEmpty)
    }

    @Test func renderIsStable() throws {
        let once = ConfigTemplate.render(nonDefaultConfig)
        let twice = ConfigTemplate.render(try ConfigCodec.decode(Data(once.utf8)).config)
        #expect(once == twice)
    }

    /// Deterministic pseudo-random configs (fixed-seed LCG) over the whole valid domain.
    @Test func randomValidConfigsRoundTrip() throws {
        var rng = SeededGenerator(seed: 0x5EED_F0F1)
        let pool = Self.trickyStrings.filter { $0.count < 100 }
        for _ in 0..<300 {
            func string() -> String {
                (0..<Int.random(in: 0...3, using: &rng)).map { _ in pool.randomElement(using: &rng)! }.joined()
            }
            let session = Int.random(in: ConfigLimits.sessionMaxMinutes, using: &rng)
            var vpn = AppConfig.VPN(
                host: string(), port: Int.random(in: ConfigLimits.port, using: &rng),
                username: string(), realm: string())
            for _ in 0..<Int.random(in: 0...5, using: &rng) {
                let digest = (0..<64).map { _ in "0123456789abcdef".randomElement(using: &rng)! }
                vpn.pinTrustedCert(String(digest))
            }
            let config = AppConfig(
                vpn: vpn,
                otp: AppConfig.OTP(
                    gmailAddress: string(),
                    method: AppConfig.OTP.Method.allCases.randomElement(using: &rng)!,
                    googleClientID: string()),
                connection: AppConfig.Connection(
                    autoConnectOnLaunch: Bool.random(using: &rng),
                    sessionMaxMinutes: session,
                    proactiveReconnect: .init(
                        enabled: Bool.random(using: &rng),
                        marginMinutes: Int.random(in: 1...min(60, session - 1), using: &rng)),
                    reconnectAfterSleep: .init(
                        enabled: Bool.random(using: &rng),
                        minSleepMinutes: Int.random(in: ConfigLimits.minSleepMinutes, using: &rng))))
            try config.validate()
            let result = try ConfigCodec.decode(ConfigCodec.encode(config))
            #expect(result.config == config)
            #expect(result.warnings.isEmpty)
        }
    }

    @Test("Invalid strings still render as well-formed YAML and are rejected by the codec", arguments: [
        "\u{00}", "\u{01}", "a\tb", "line\nbreak", "\r", "\u{1B}", "\u{7F}", "\u{80}", "\u{85}", "\u{9F}",
        "\u{2028}", "\u{2029}", "a\u{FEFF}", scalar(0xFDD0), scalar(0xFFFE), scalar(0xFFFF), scalar(0x1FFFF), scalar(0x10FFFF),
    ])
    func invalidStringsRenderSafely(value: String) throws {
        let config = AppConfig(vpn: AppConfig.VPN(host: value))
        let rendered = ConfigTemplate.render(config)
        #expect(rendered.unicodeScalars.allSatisfy { $0 == "\n" || !ConfigText.isDisallowedInField($0) })
        let error = try #require(loadError(rendered))
        let bad = try #require(value.unicodeScalars.first(where: ConfigText.isDisallowedInField))
        #expect(error.kind == .invalidCharacter(path: "vpn.host", scalar: bad.value))
    }

    /// Yams decodes scalar bytes with Foundation, which drops a *leading* U+FEFF. Either way a
    /// decoded string field never contains a BOM.
    @Test("Leading BOM in a value never survives decoding", arguments: ["\u{FEFF}", "\u{FEFF}gw"])
    func leadingBOMInValue(value: String) throws {
        let rendered = ConfigTemplate.render(AppConfig(vpn: AppConfig.VPN(host: value)))
        do {
            let host = try ConfigCodec.decode(Data(rendered.utf8)).config.vpn.host
            #expect(!host.unicodeScalars.contains("\u{FEFF}"))
        } catch {
            #expect(error.kind == .invalidCharacter(path: "vpn.host", scalar: 0xFEFF))
        }
    }

    @Test("Double-quoted escapes", arguments: [
        ("plain", "\"plain\""), ("say \"hi\"", "\"say \\\"hi\\\"\""), ("C:\\dir", "\"C:\\\\dir\""),
        ("\t\n\r", "\"\\t\\n\\r\""), ("\u{00}\u{07}\u{08}\u{0B}\u{0C}\u{1B}", "\"\\0\\a\\b\\v\\f\\e\""),
        ("\u{01}\u{7F}", "\"\\x01\\x7F\""), ("\u{85}\u{2028}\u{2029}", "\"\\N\\L\\P\""),
        ("\u{FEFF}" + scalar(0xFFFE), "\"\\uFEFF\\uFFFE\""), (scalar(0x1FFFE), "\"\\U0001FFFE\""), ("\u{D55C}🚀", "\"\u{D55C}🚀\""),
    ])
    func quoting(value: String, expected: String) {
        #expect(ConfigTemplate.quoted(value) == expected)
    }
}

/// SplitMix64: tiny deterministic generator for reproducible "random" tests.
struct SeededGenerator: RandomNumberGenerator {
    private var state: UInt64

    init(seed: UInt64) {
        state = seed
    }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}
