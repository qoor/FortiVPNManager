import Foundation
import Testing
@testable import FortiConfig

@Suite struct AppConfigValidationTests {
    @Test func validConfigsPass() throws {
        try AppConfig().validate()
        try nonDefaultConfig.validate()
    }

    @Test("Each rule", arguments: [
        ({ @Sendable (c: inout AppConfig) in c.schemaVersion = 2 }, ConfigLoadError.Kind.newerSchemaVersion(2)),
        ({ @Sendable (c: inout AppConfig) in c.schemaVersion = 0 },
         .outOfRange(path: "schema_version", raw: "0", range: 1...1)),
        ({ @Sendable (c: inout AppConfig) in c.vpn.port = 0 }, .outOfRange(path: "vpn.port", raw: "0", range: 1...65535)),
        ({ @Sendable (c: inout AppConfig) in c.vpn.port = 70000 },
         .outOfRange(path: "vpn.port", raw: "70000", range: 1...65535)),
        ({ @Sendable (c: inout AppConfig) in c.vpn.host = "gw\n" }, .invalidCharacter(path: "vpn.host", scalar: 0x0A)),
        ({ @Sendable (c: inout AppConfig) in c.vpn.username = "\u{2028}" },
         .invalidCharacter(path: "vpn.username", scalar: 0x2028)),
        ({ @Sendable (c: inout AppConfig) in c.vpn.realm = "\u{FEFF}" }, .invalidCharacter(path: "vpn.realm", scalar: 0xFEFF)),
        ({ @Sendable (c: inout AppConfig) in c.otp.gmailAddress = "a" + scalar(0xFFFE) },
         .invalidCharacter(path: "otp.gmail_address", scalar: 0xFFFE)),
        ({ @Sendable (c: inout AppConfig) in c.otp.googleClientID = "id\u{0B}" },
         .invalidCharacter(path: "otp.google_client_id", scalar: 0x0B)),
        ({ @Sendable (c: inout AppConfig) in c.vpn.trustedCerts = [digestA, digestB.uppercased()] },
         .invalidTrustedCert(index: 1)),
        ({ @Sendable (c: inout AppConfig) in c.connection.sessionMaxMinutes = 9 },
         .outOfRange(path: "connection.session_max_minutes", raw: "9", range: 10...1440)),
        ({ @Sendable (c: inout AppConfig) in c.connection.proactiveReconnect.marginMinutes = 61 },
         .outOfRange(path: "connection.proactive_reconnect.margin_minutes", raw: "61", range: 1...60)),
        ({ @Sendable (c: inout AppConfig) in c.connection.reconnectAfterSleep.minSleepMinutes = 0 },
         .outOfRange(path: "connection.reconnect_after_sleep.min_sleep_minutes", raw: "0", range: 1...1440)),
        ({ @Sendable (c: inout AppConfig) in
            c.connection.sessionMaxMinutes = 20
            c.connection.proactiveReconnect.marginMinutes = 20
        }, .marginNotLessThanSession(margin: 20, session: 20)),
    ] as [(@Sendable (inout AppConfig) -> Void, ConfigLoadError.Kind)])
    func rules(mutate: @Sendable (inout AppConfig) -> Void, expected: ConfigLoadError.Kind) {
        var config = AppConfig()
        mutate(&config)
        let error = #expect(throws: ConfigLoadError.self) { try config.validate() }
        #expect(error?.kind == expected)
        #expect(error?.location == nil)
    }

    /// A config whose rendered file the codec would refuse must not validate.
    @Test func renderedFileMustFitTheSizeLimit() throws {
        let limit = ConfigLimits.maxFileSize
        let overhead = ConfigCodec.encode(AppConfig()).count  // with an empty realm
        var fits = AppConfig()
        fits.vpn.realm = String(repeating: "r", count: limit - overhead)
        #expect(ConfigCodec.encode(fits).count == limit)
        try fits.validate()
        #expect(try ConfigCodec.decode(ConfigCodec.encode(fits)).config == fits)

        var tooLarge = fits
        tooLarge.vpn.realm += "r"
        let error = #expect(throws: ConfigLoadError.self) { try tooLarge.validate() }
        #expect(error?.kind == .fileTooLarge(byteCount: limit + 1, limit: limit))
        #expect(error?.location == nil)

        // Multi-byte text counts in UTF-8 bytes.
        var korean = AppConfig()
        korean.vpn.realm = String(repeating: "\u{D55C}", count: 70_000 / 3)
        #expect(throws: ConfigLoadError(.fileTooLarge(byteCount: ConfigCodec.encode(korean).count, limit: limit))) {
            try korean.validate()
        }
    }
}
