import Foundation
import Testing
@testable import FortiConfig

@Suite struct ConfigCodecDefaultsTests {
    @Test("Empty, comment-only and null documents give the defaults",
          arguments: ["", "\n\n", "   \n", "# only a comment\n# another\n", "---\n", "---\n...\n", "~\n", "null\n"])
    func emptyDocuments(yaml: String) throws {
        let result = try decodeYAML(yaml)
        #expect(result.config == AppConfig())
        #expect(result.warnings.isEmpty)
    }

    @Test func defaultsMatchSchema() {
        let config = AppConfig()
        #expect(config.schemaVersion == 1)
        #expect(config.vpn.host == "")
        #expect(config.vpn.port == 443)
        #expect(config.vpn.username == "")
        #expect(config.vpn.realm == "")
        #expect(config.vpn.trustedCerts == [])
        #expect(config.otp.gmailAddress == "")
        #expect(config.otp.method == .imap)
        #expect(config.otp.googleClientID == "")
        #expect(config.connection.autoConnectOnLaunch == true)
        #expect(config.connection.sessionMaxMinutes == 180)
        #expect(config.connection.proactiveReconnect.enabled == true)
        #expect(config.connection.proactiveReconnect.marginMinutes == 5)
        #expect(config.connection.reconnectAfterSleep.enabled == false)
        #expect(config.connection.reconnectAfterSleep.minSleepMinutes == 30)
    }

    @Test func fullDocumentInBlockStyle() throws {
        let yaml = """
            schema_version: 1
            vpn:
              host: vpn.example.com
              port: 8443
              username: bob
              realm: corp
              trusted_certs:
                - \(digestA)
                - "\(digestB)"
            otp:
              gmail_address: otp@example.com
              method: gmail_api
              google_client_id: \(googleClientID)
            connection:
              auto_connect_on_launch: false
              session_max_minutes: 175
              proactive_reconnect:
                enabled: false
                margin_minutes: 7
              reconnect_after_sleep:
                enabled: true
                min_sleep_minutes: 45
              schedule:
                enabled: true
                days:
                  - sat
                  - mon
                  - wed
                start: "08:30"
                end: 19:05
                away_minutes: 90
            """
        let result = try decodeYAML(yaml)
        #expect(result.config == nonDefaultConfig)
        #expect(result.warnings.isEmpty)
    }

    @Test func fullDocumentInFlowStyle() throws {
        let yaml = """
            {schema_version: 1, vpn: {host: vpn.example.com, port: 8443, username: bob, realm: corp,
             trusted_certs: [\(digestA), '\(digestB)']},
             otp: {gmail_address: otp@example.com, method: "gmail_api", google_client_id: '\(googleClientID)'},
             connection: {auto_connect_on_launch: false, session_max_minutes: 175,
             proactive_reconnect: {enabled: false, margin_minutes: 7},
             reconnect_after_sleep: {enabled: true, min_sleep_minutes: 45},
             schedule: {enabled: true, days: [mon, wed, sat, wed], start: 8:30, end: '19:05', away_minutes: 90}}}
            """
        #expect(try decodeYAML(yaml).config == nonDefaultConfig)
    }

    @Test func missingKeysAreMergedWithDefaults() throws {
        let yaml = """
            vpn:
              host: gw.example.com
            connection:
              proactive_reconnect:
                margin_minutes: 10
            """
        var expected = AppConfig()
        expected.vpn.host = "gw.example.com"
        expected.connection.proactiveReconnect.marginMinutes = 10
        let result = try decodeYAML(yaml)
        #expect(result.config == expected)
        #expect(result.warnings.isEmpty)
    }

    @Test("Null values and sections fall back to defaults",
          arguments: [
            "vpn:\n", "vpn: ~\n", "vpn: null\n", "vpn:\n  port:\n", "vpn:\n  port: ~\n", "vpn:\n  host: NULL\n",
            "connection:\n  proactive_reconnect:\n", "connection:\n  auto_connect_on_launch: Null\n",
            "schema_version:\n", "vpn:\n  trusted_certs:\n",
          ])
    func nullValues(yaml: String) throws {
        #expect(try decodeYAML(yaml).config == AppConfig())
    }

    @Test func stringFieldsKeepTheirTextAsWritten() throws {
        let yaml = """
            vpn:
              host: 10.0.0.1
              username: 0123
              realm: true
            otp:
              gmail_address: '\u{D55C}\u{AE00}@example.com'
            """
        let config = try decodeYAML(yaml).config
        #expect(config.vpn.host == "10.0.0.1")
        #expect(config.vpn.username == "0123")
        #expect(config.vpn.realm == "true")
        #expect(config.otp.gmailAddress == "\u{D55C}\u{AE00}@example.com")
    }

    @Test("Booleans accept the YAML 1.2 core spellings",
          arguments: [("true", true), ("True", true), ("TRUE", true), ("false", false), ("False", false), ("FALSE", false)])
    func booleanSpellings(raw: String, value: Bool) throws {
        let config = try decodeYAML("connection:\n  auto_connect_on_launch: \(raw)\n").config
        #expect(config.connection.autoConnectOnLaunch == value)
    }

    @Test func crlfLineEndingsAndLeadingBOM() throws {
        let yaml = "\u{FEFF}vpn:\r\n  host: gw\r\n  port: 8443\r\n"
        let config = try decodeYAML(yaml).config
        #expect(config.vpn.host == "gw")
        #expect(config.vpn.port == 8443)
        // The BOM is not counted in columns.
        #expect(loadError("\u{FEFF}vpn:\n  port: 0443\n")?.location == loc(2, 9))
        // CRLF counts as one line break.
        #expect(loadError("vpn:\r\n  host: gw\r\n  port: 0443\r\n")?.location == loc(3, 9))
    }
}

@Suite struct ConfigCodecIntegerTests {
    @Test("Integers must match ^(0|[1-9][0-9]*)$",
          arguments: ["0443", "2:30", "0x1BB", "0X1bb", "1_0", "+443", "-1", "443.0", "1e3", "0o12", "0b1", "00",
                      "abc", "1 2", ".inf", "４４３"])
    func rejectsNonCanonicalIntegers(raw: String) throws {
        let error = try #require(loadError("vpn:\n  port: \(raw)\n"))
        #expect(error.kind == .invalidInteger(path: "vpn.port", raw: raw))
        #expect(error.location == loc(2, 9))
        #expect(error.message.contains("vpn.port"))
        #expect(error.displayMessage.hasPrefix("Line 2, column 9: "))
    }

    @Test("Every integer field uses the strict rule", arguments: [
        ("schema_version: 0443\n", "schema_version", loc(1, 17)),
        ("connection:\n  session_max_minutes: 2:30\n", "connection.session_max_minutes", loc(2, 24)),
        ("connection:\n  proactive_reconnect: { enabled: true, margin_minutes: 0x1BB }\n",
         "connection.proactive_reconnect.margin_minutes", loc(2, 57)),
        ("connection:\n  reconnect_after_sleep:\n    min_sleep_minutes: 1_0\n",
         "connection.reconnect_after_sleep.min_sleep_minutes", loc(3, 24)),
    ])
    func strictIntegersEverywhere(yaml: String, path: String, location: LineColumn) throws {
        let error = try #require(loadError(yaml))
        guard case .invalidInteger(let errorPath, _) = error.kind else {
            Issue.record("unexpected \(error.kind)")
            return
        }
        #expect(errorPath == path)
        #expect(error.location == location)
    }

    @Test("Quoted numbers get a dedicated message",
          arguments: ["\"443\"", "'443'", "\"0x1BB\"", "\" 8443 \"", "'-1'"])
    func quotedNumbers(literal: String) throws {
        let error = try #require(loadError("vpn:\n  port: \(literal)\n"))
        guard case .quotedNumber(path: "vpn.port", raw: _) = error.kind else {
            Issue.record("unexpected \(error.kind)")
            return
        }
        #expect(error.location == loc(2, 9))
        #expect(error.message.contains("quote"))
        #expect(error.message.contains("port: 443"))
    }

    @Test func quotedNonNumberIsATypeError() throws {
        let error = try #require(loadError("vpn:\n  port: \"https\"\n"))
        #expect(error.kind == .typeMismatch(path: "vpn.port", expected: .integer))
        #expect(error.location == loc(2, 9))
    }

    @Test("Quoted booleans get a dedicated message",
          arguments: ["\"true\"", "'false'", "\"yes\"", "'Off'"])
    func quotedBooleans(literal: String) throws {
        let error = try #require(loadError("connection:\n  auto_connect_on_launch: \(literal)\n"))
        guard case .quotedBoolean(path: "connection.auto_connect_on_launch", raw: _) = error.kind else {
            Issue.record("unexpected \(error.kind)")
            return
        }
        #expect(error.location == loc(2, 27))
        #expect(error.message.contains("quote"))
    }

    @Test func quotedNonBooleanIsATypeError() throws {
        let error = try #require(loadError("connection:\n  auto_connect_on_launch: \"maybe\"\n"))
        #expect(error.kind == .typeMismatch(path: "connection.auto_connect_on_launch", expected: .boolean))
    }

    @Test("YAML 1.1 booleans and numbers are not booleans", arguments: ["yes", "no", "on", "off", "y", "n", "1", "0"])
    func rejectsYAML11Booleans(raw: String) throws {
        let error = try #require(loadError("connection:\n  reconnect_after_sleep: { enabled: \(raw) }\n"))
        #expect(error.kind == .invalidBoolean(path: "connection.reconnect_after_sleep.enabled", raw: raw))
        #expect(error.location == loc(2, 37))
    }

    @Test("Range limits", arguments: [
        ("vpn:\n  port: 0\n", "vpn.port", false),
        ("vpn:\n  port: 1\n", "vpn.port", true),
        ("vpn:\n  port: 65535\n", "vpn.port", true),
        ("vpn:\n  port: 65536\n", "vpn.port", false),
        ("vpn:\n  port: 99999999999999999999999\n", "vpn.port", false),
        ("connection:\n  session_max_minutes: 9\n", "connection.session_max_minutes", false),
        ("connection:\n  session_max_minutes: 10\n", "connection.session_max_minutes", true),
        ("connection:\n  session_max_minutes: 1440\n", "connection.session_max_minutes", true),
        ("connection:\n  session_max_minutes: 1441\n", "connection.session_max_minutes", false),
        ("connection:\n  proactive_reconnect: { margin_minutes: 0 }\n",
         "connection.proactive_reconnect.margin_minutes", false),
        ("connection:\n  proactive_reconnect: { margin_minutes: 1 }\n",
         "connection.proactive_reconnect.margin_minutes", true),
        ("connection:\n  proactive_reconnect: { margin_minutes: 60 }\n",
         "connection.proactive_reconnect.margin_minutes", true),
        ("connection:\n  proactive_reconnect: { margin_minutes: 61 }\n",
         "connection.proactive_reconnect.margin_minutes", false),
        ("connection:\n  reconnect_after_sleep: { min_sleep_minutes: 0 }\n",
         "connection.reconnect_after_sleep.min_sleep_minutes", false),
        ("connection:\n  reconnect_after_sleep: { min_sleep_minutes: 1 }\n",
         "connection.reconnect_after_sleep.min_sleep_minutes", true),
        ("connection:\n  reconnect_after_sleep: { min_sleep_minutes: 1440 }\n",
         "connection.reconnect_after_sleep.min_sleep_minutes", true),
        ("connection:\n  reconnect_after_sleep: { min_sleep_minutes: 1441 }\n",
         "connection.reconnect_after_sleep.min_sleep_minutes", false),
        ("connection:\n  schedule: { away_minutes: 0 }\n", "connection.schedule.away_minutes", false),
        ("connection:\n  schedule: { away_minutes: 1 }\n", "connection.schedule.away_minutes", true),
        ("connection:\n  schedule: { away_minutes: 1440 }\n", "connection.schedule.away_minutes", true),
        ("connection:\n  schedule: { away_minutes: 1441 }\n", "connection.schedule.away_minutes", false),
    ])
    func ranges(yaml: String, path: String, valid: Bool) throws {
        let error = loadError(yaml)
        if valid {
            #expect(error == nil)
        } else {
            guard case .outOfRange(let errorPath, _, _)? = error?.kind else {
                Issue.record("expected outOfRange, got \(String(describing: error))")
                return
            }
            #expect(errorPath == path)
            #expect(error?.location?.line == yaml.split(separator: "\n").count)
            #expect(error?.message.contains("allowed range") == true)
        }
    }

    @Test func marginMustBeLessThanSession() throws {
        let yaml = """
            connection:
              session_max_minutes: 30
              proactive_reconnect: { enabled: true, margin_minutes: 30 }
            """
        let error = try #require(loadError(yaml))
        #expect(error.kind == .marginNotLessThanSession(margin: 30, session: 30))
        #expect(error.location == loc(3, 57))

        // Margin key order does not matter, and the default margin (5) fits the smallest session (10).
        #expect(loadError("connection:\n  proactive_reconnect: { margin_minutes: 45 }\n  session_max_minutes: 30\n")?
            .kind == .marginNotLessThanSession(margin: 45, session: 30))
        #expect(loadError("connection:\n  session_max_minutes: 10\n") == nil)
        #expect(loadError("connection:\n  session_max_minutes: 10\n  proactive_reconnect: { margin_minutes: 9 }\n") == nil)
    }
}

@Suite struct ConfigCodecStructureTests {
    @Test("Anchors and aliases are rejected at the anchor", arguments: [
        ("vpn: &v\n  host: a\n", loc(1, 6)),
        ("base: &p 8443\nvpn:\n  port: *p\n", loc(1, 7)),
        ("vpn:\n  host: &h gw\n  username: *h\n", loc(2, 9)),
        ("vpn:\n  trusted_certs: [&d \(String(repeating: "a", count: 64))]\n", loc(2, 19)),
        ("&root\nvpn: {}\n", loc(1, 1)),
    ])
    func anchorsAndAliases(yaml: String, location: LineColumn) throws {
        let error = try #require(loadError(yaml))
        #expect(error.kind == .anchorOrAlias)
        #expect(error.location == location)
        #expect(error.message.contains("Anchors"))
    }

    @Test func undefinedAliasIsReportedAsAlias() throws {
        let error = try #require(loadError("vpn:\n  port: *nope\n"))
        #expect(error.kind == .anchorOrAlias)
        #expect(error.location == loc(2, 9))
    }

    @Test("Merge keys are rejected", arguments: [
        ("vpn:\n  <<: { port: 8443 }\n", loc(2, 3)),
        ("defaults: { port: 8443 }\nvpn:\n  host: gw\n  <<: {}\n", loc(4, 3)),
        ("<<: { vpn: {} }\n", loc(1, 1)),
    ])
    func mergeKeys(yaml: String, location: LineColumn) throws {
        let error = try #require(loadError(yaml))
        #expect(error.kind == .mergeKey)
        #expect(error.location == location)
    }

    @Test func quotedMergeLookalikeIsJustAnUnknownKey() throws {
        let result = try decodeYAML("vpn:\n  \"<<\": 1\n")
        #expect(result.warnings == [ConfigWarning(.unknownKey(path: "vpn.<<"), at: loc(2, 3))])
    }

    @Test("Explicit tags are rejected", arguments: [
        ("vpn:\n  port: !!int 443\n", "tag:yaml.org,2002:int", loc(2, 9)),
        ("vpn:\n  host: !!str gw\n", "tag:yaml.org,2002:str", loc(2, 9)),
        ("vpn:\n  host: !custom gw\n", "!custom", loc(2, 9)),
        ("vpn: !!map\n  host: gw\n", "tag:yaml.org,2002:map", loc(1, 6)),
        ("vpn:\n  trusted_certs: !!seq []\n", "tag:yaml.org,2002:seq", loc(2, 18)),
        ("vpn:\n  port: !!int \"443\"\n", "tag:yaml.org,2002:int", loc(2, 9)),
        ("schema_version: !!int 1\n", "tag:yaml.org,2002:int", loc(1, 17)),
    ])
    func explicitTags(yaml: String, tag: String, location: LineColumn) throws {
        let error = try #require(loadError(yaml))
        #expect(error.kind == .explicitTag(tag))
        #expect(error.location == location)
    }

    @Test("Root must be a mapping", arguments: [
        ("- a\n- b\n", loc(1, 1)), ("just a string\n", loc(1, 1)), ("42\n", loc(1, 1)), ("[]\n", loc(1, 1)),
        ("\n\n\"quoted\"\n", loc(3, 1)),
    ])
    func rootNotMapping(yaml: String, location: LineColumn) throws {
        let error = try #require(loadError(yaml))
        #expect(error.kind == .rootNotMapping)
        #expect(error.location == location)
    }

    @Test("Wrong node kinds", arguments: [
        ("vpn: 5\n", "vpn", ConfigLoadError.ValueKind.mapping, loc(1, 6)),
        ("vpn: []\n", "vpn", .mapping, loc(1, 6)),
        ("connection:\n  proactive_reconnect: on\n", "connection.proactive_reconnect", .mapping, loc(2, 24)),
        ("vpn:\n  host: [a]\n", "vpn.host", .string, loc(2, 9)),
        ("otp:\n  gmail_address: { a: b }\n", "otp.gmail_address", .string, loc(2, 18)),
        ("otp:\n  method: [imap]\n", "otp.method", .string, loc(2, 11)),
        ("otp:\n  google_client_id: { a: b }\n", "otp.google_client_id", .string, loc(2, 21)),
        ("vpn:\n  port: { a: 1 }\n", "vpn.port", .integer, loc(2, 9)),
        ("vpn:\n  port: |\n    443\n", "vpn.port", .integer, loc(2, 9)),
        ("connection:\n  auto_connect_on_launch: [true]\n", "connection.auto_connect_on_launch", .boolean, loc(2, 27)),
        ("vpn:\n  trusted_certs: \(String(repeating: "a", count: 64))\n", "vpn.trusted_certs", .sequence, loc(2, 18)),
        ("vpn:\n  trusted_certs: { a: b }\n", "vpn.trusted_certs", .sequence, loc(2, 18)),
        ("schema_version: [1]\n", "schema_version", .integer, loc(1, 17)),
    ])
    func typeMismatches(yaml: String, path: String, expected: ConfigLoadError.ValueKind, location: LineColumn) throws {
        let error = try #require(loadError(yaml))
        #expect(error.kind == .typeMismatch(path: path, expected: expected))
        #expect(error.location == location)
    }

    @Test func complexKeysAreRejected() throws {
        let error = try #require(loadError("vpn:\n  ? [a, b]\n  : c\n"))
        #expect(error.kind == .complexKey)
        #expect(error.location == loc(2, 5))
    }

    @Test("Duplicate keys", arguments: [
        ("vpn:\n  port: 1\n  port: 2\n", "port", 2),
        ("vpn: {}\notp: {}\nvpn: {}\n", "vpn", 1),
        ("vpn:\n  port: 1\n  \"port\": 2\n", "port", 2),
    ])
    func duplicateKeys(yaml: String, key: String, line: Int) throws {
        let error = try #require(loadError(yaml))
        #expect(error.kind == .duplicateKey(key))
        #expect(error.location?.line == line)
    }

    @Test func multipleDocumentsAreRejected() throws {
        let error = try #require(loadError("vpn: {}\n---\notp: {}\n"))
        #expect(error.kind == .multipleDocuments)
        #expect(error.location == loc(2, 1))
    }

    @Test("Syntax errors carry line and column", arguments: [
        ("vpn:\n  port: [1, 2\n", loc(3, 1)),
        ("vpn:\n\tport: 1\n", loc(2, 1)),
        ("vpn:\n  host: \"unterminated\n", loc(3, 1)),
        ("vpn:\n  host: a\n port: 1\n", loc(3, 2)),
        ("vpn: {host: a\n", loc(2, 1)),
        ("vpn:\n  host: a: b\n", loc(2, 10)),
    ])
    func syntaxErrors(yaml: String, location: LineColumn) throws {
        let error = try #require(loadError(yaml))
        guard case .syntax = error.kind else {
            Issue.record("unexpected \(error.kind)")
            return
        }
        #expect(error.location == location)
        #expect(error.message.hasPrefix("YAML syntax error"))
    }

    @Test func unknownKeysBecomeWarningsWithLocations() throws {
        let yaml = """
            schema_version: 1
            theme: dark
            vpn:
              host: gw
              insecure-cert: 1
            connection:
              proactive_reconnect: { enabled: true, jitter: 3 }
            """
        let result = try decodeYAML(yaml)
        #expect(result.config.vpn.host == "gw")
        #expect(result.warnings == [
            ConfigWarning(.unknownKey(path: "theme"), at: loc(2, 1)),
            ConfigWarning(.unknownKey(path: "vpn.insecure-cert"), at: loc(5, 3)),
            ConfigWarning(.unknownKey(path: "connection.proactive_reconnect.jitter"), at: loc(7, 41)),
        ])
        #expect(result.warnings[1].displayMessage
            == "Line 5, column 3: Ignoring unknown key 'vpn.insecure-cert'. It will be removed the next time you save in Settings.")
    }

    @Test func unknownKeyValuesAreNotTypeChecked() throws {
        let result = try decodeYAML("future:\n  port: \"443\"\n  list: [1, 2]\n")
        #expect(result.config == AppConfig())
        #expect(result.warnings.map(\.kind) == [.unknownKey(path: "future")])
    }
}

@Suite struct ConfigCodecTextTests {
    @Test func sixtyFourKiBLimit() throws {
        let head = "vpn:\n  port: 8443\n# "
        let limit = ConfigLimits.maxFileSize
        #expect(limit == 65_536)
        let exact = head + String(repeating: "x", count: limit - head.utf8.count - 1) + "\n"
        #expect(exact.utf8.count == limit)
        #expect(try decodeYAML(exact).config.vpn.port == 8443)

        let over = exact + "\n"
        let error = try #require(loadError(over))
        #expect(error.kind == .fileTooLarge(byteCount: limit + 1, limit: limit))
        #expect(error.location == nil)
        #expect(error.message.contains("64 KiB"))
    }

    @Test func invalidUTF8IsLocated() throws {
        var data = Data("vpn:\n  host: \"a".utf8)
        data.append(0xFF)
        data.append(contentsOf: Data("b\"\n".utf8))
        let error = try #require(loadError(data))
        #expect(error.kind == .invalidUTF8)
        #expect(error.location == loc(2, 11))

        // Truncated multi-byte sequence after non-ASCII text on the first line.
        let truncated = Data("# \u{D55C}\u{AE00}".utf8) + Data([0xED, 0x95])
        #expect(loadError(truncated)?.location == loc(1, 5))
        // Encoded surrogate (invalid in UTF-8).
        #expect(loadError(Data([0x61, 0x3A, 0x20, 0xED, 0xA0, 0x80]))?.kind == .invalidUTF8)
    }

    @Test("Raw characters libyaml cannot read are located from the reader offset", arguments: [
        ("vpn:\n  host: a\u{01}b\n", UInt32(0x01), loc(2, 10)),
        ("# comment\u{7F}\nvpn: {}\n", UInt32(0x7F), loc(1, 10)),
        ("# \u{D55C}\u{AE00}\u{01}\n", UInt32(0x01), loc(1, 5)),
        ("vpn: {}\r\n# \u{00}\r\n", UInt32(0x00), loc(2, 3)),
        ("vpn:\n  host: \"a\u{80}\"\n", UInt32(0x80), loc(2, 11)),
        ("vpn:\n  realm: x" + scalar(0xFFFE) + "\n", UInt32(0xFFFE), loc(2, 11)),
        ("vpn:\n  realm: e\u{301}\u{1B}\n", UInt32(0x1B), loc(2, 12)),
    ])
    func unreadableCharacters(yaml: String, scalar: UInt32, location: LineColumn) throws {
        let error = try #require(loadError(yaml))
        #expect(error.kind == .disallowedCharacter(scalar: scalar))
        #expect(error.location == location)
    }

    @Test("String fields reject control, separator, BOM and noncharacter code points", arguments: [
        ("vpn:\n  host: \"a\\tb\"\n", "vpn.host", UInt32(0x09)),
        ("vpn:\n  host: \"a\\x01b\"\n", "vpn.host", UInt32(0x01)),
        ("vpn:\n  username: \"a\\nb\"\n", "vpn.username", UInt32(0x0A)),
        ("vpn:\n  username: \"a\\Nb\"\n", "vpn.username", UInt32(0x85)),
        ("vpn:\n  realm: \"a\\x9Fb\"\n", "vpn.realm", UInt32(0x9F)),
        ("vpn:\n  realm: \"a\\Lb\"\n", "vpn.realm", UInt32(0x2028)),
        ("vpn:\n  realm: \"a\\Pb\"\n", "vpn.realm", UInt32(0x2029)),
        ("otp:\n  gmail_address: \"a\\uFEFFb\"\n", "otp.gmail_address", UInt32(0xFEFF)),
        ("otp:\n  gmail_address: \"\\uFDD0\"\n", "otp.gmail_address", UInt32(0xFDD0)),
        ("otp:\n  gmail_address: \"\\uFFFF\"\n", "otp.gmail_address", UInt32(0xFFFF)),
        ("otp:\n  gmail_address: \"\\U0001FFFE\"\n", "otp.gmail_address", UInt32(0x1FFFE)),
        ("vpn:\n  host: \"a\u{FEFF}b\"\n", "vpn.host", UInt32(0xFEFF)),
        ("vpn:\n  host: \"a\tb\"\n", "vpn.host", UInt32(0x09)),
        ("vpn:\n  host: |\n    gw\n", "vpn.host", UInt32(0x0A)),
    ])
    func invalidCharactersInStrings(yaml: String, path: String, scalar: UInt32) throws {
        let error = try #require(loadError(yaml))
        #expect(error.kind == .invalidCharacter(path: path, scalar: scalar))
        #expect(error.location?.line == 2)
        #expect(error.message.contains(ConfigText.codePoint(scalar)))
    }

    @Test func allowedUnicodeInStrings() throws {
        let config = try decodeYAML("vpn:\n  realm: \"\u{C601}\u{C5C5}\u{BD80} 🚀 \\u00A0x\"\n").config
        #expect(config.vpn.realm == "\u{C601}\u{C5C5}\u{BD80} 🚀 \u{A0}x")
    }
}

@Suite struct ConfigCodecSchemaVersionTests {
    @Test func missingVersionMeansOne() throws {
        #expect(try decodeYAML("vpn: {}\n").config.schemaVersion == 1)
        #expect(try decodeYAML("schema_version: 1\n").config.schemaVersion == 1)
    }

    @Test func newerVersionIsReadOnlyError() throws {
        let error = try #require(loadError("schema_version: 2\n"))
        #expect(error.kind == .newerSchemaVersion(2))
        #expect(error.location == loc(1, 17))
        #expect(error.message.contains("newer version"))
        #expect(error.message.contains("read-only"))
    }

    @Test func newerVersionWinsOverEverythingElse() throws {
        // A future file may use keys, values or ranges this version rejects.
        let yaml = """
            vpn:
              port: 0443
              trusted_certs: { primary: x }
            schema_version: 3
            connection:
              session_max_minutes: "forever"
            """
        #expect(loadError(yaml)?.kind == .newerSchemaVersion(3))
    }

    @Test("Invalid versions", arguments: [
        ("schema_version: 0\n", "out"), ("schema_version: 99999999999999999999999\n", "out"),
        ("schema_version: \"1\"\n", "quoted"), ("schema_version: 01\n", "invalid"), ("schema_version: v1\n", "invalid"),
    ])
    func invalidVersions(yaml: String, expected: String) throws {
        let error = try #require(loadError(yaml))
        #expect(error.location == loc(1, 17))
        switch (error.kind, expected) {
        case (.outOfRange(path: "schema_version", raw: _, range: 1...1), "out"),
             (.quotedNumber(path: "schema_version", raw: "1"), "quoted"),
             (.invalidInteger(path: "schema_version", raw: _), "invalid"):
            break
        default:
            Issue.record("unexpected \(error.kind) for \(expected)")
        }
    }
}

@Suite struct ConfigCodecTrustedCertTests {
    @Test func uppercaseIsNormalized() throws {
        let upper = digestB.uppercased()
        let config = try decodeYAML("vpn:\n  trusted_certs:\n    - \(upper)\n").config
        #expect(config.vpn.trustedCerts == [digestB])
    }

    @Test func invalidEntriesAreDroppedWithWarnings() throws {
        let yaml = """
            vpn:
              trusted_certs:
                - \(digestA)
                - not-a-digest
                - \(String(repeating: "a", count: 63))
                - \(String(repeating: "f", count: 65))
                - "\(digestB) "
                - ~
                - { sha: \(digestC) }
                - [\(digestC)]
                - \(digestC.uppercased())
                - "\(digestA.uppercased())"
                - ０\(String(repeating: "0", count: 63))
            """
        let result = try decodeYAML(yaml)
        #expect(result.config.vpn.trustedCerts == [digestA, digestC])
        #expect(result.warnings == [
            ConfigWarning(.invalidTrustedCert(index: 1), at: loc(4, 7)),
            ConfigWarning(.invalidTrustedCert(index: 2), at: loc(5, 7)),
            ConfigWarning(.invalidTrustedCert(index: 3), at: loc(6, 7)),
            ConfigWarning(.invalidTrustedCert(index: 4), at: loc(7, 7)),
            ConfigWarning(.invalidTrustedCert(index: 5), at: loc(8, 7)),
            ConfigWarning(.invalidTrustedCert(index: 6), at: loc(9, 7)),
            ConfigWarning(.invalidTrustedCert(index: 7), at: loc(10, 7)),
            ConfigWarning(.duplicateTrustedCert(index: 9), at: loc(12, 7)),
            ConfigWarning(.invalidTrustedCert(index: 10), at: loc(13, 7)),
        ])
        #expect(result.warnings[0].message.contains("entry 2"))
    }

    @Test func allDigitDigestStaysText() throws {
        let digits = String(repeating: "0123456789", count: 6) + "0123"
        let config = try decodeYAML("vpn:\n  trusted_certs: [\(digits)]\n").config
        #expect(config.vpn.trustedCerts == [digits])
    }

    @Test func emptyForms() throws {
        #expect(try decodeYAML("vpn:\n  trusted_certs: []\n").config.vpn.trustedCerts == [])
        #expect(try decodeYAML("vpn:\n  trusted_certs:\n").config.vpn.trustedCerts == [])
    }
}

/// Yams composes recursively; deep documents must neither crash a 512 KiB test thread nor pass silently.
@Suite struct ConfigCodecNestingTests {
    @Test("Deep flow nesting up to libyaml's cap decodes without exhausting the stack", arguments: [100, 999])
    func deepButAllowed(depth: Int) throws {
        let yaml = "notes: " + String(repeating: "[", count: depth) + String(repeating: "]", count: depth) + "\n"
        let result = try decodeYAML(yaml)
        #expect(result.warnings.map(\.kind) == [.unknownKey(path: "notes")])
    }

    @Test func deeperThanLibYAMLAllowsIsAnError() throws {
        let depth = 1_200
        let yaml = "vpn:\n  x: " + String(repeating: "[", count: depth) + String(repeating: "]", count: depth) + "\n"
        let error = try #require(loadError(yaml))
        #expect(error.kind == .nestingTooDeep)
        #expect(error.location?.line == 2)
    }

    @Test func deepBlockSequencesToo() throws {
        // Compact nested block sequences: `- - - … x`, one level per two bytes.
        let yaml = "notes:\n" + String(repeating: "- ", count: 990) + "x\n"
        #expect(try decodeYAML(yaml).warnings.map(\.kind) == [.unknownKey(path: "notes")])
    }

    @Test func largeStackThreadRunsSynchronouslyWithTheRequestedStack() {
        let size = LargeStackThread.run { Thread.current.stackSize }
        #expect(size >= LargeStackThread.stackSize)
        #expect(LargeStackThread.run { 6 * 7 } == 42)
    }
}

@Suite struct ConfigCodecOTPMethodTests {
    @Test("Accepted spellings of otp.method", arguments: [
        ("imap", AppConfig.OTP.Method.imap), ("\"imap\"", .imap), ("gmail_api", .gmailAPI), ("'gmail_api'", .gmailAPI),
        ("~", .imap), ("", .imap),
    ])
    func acceptedMethods(raw: String, expected: AppConfig.OTP.Method) throws {
        let result = try decodeYAML("otp:\n  method: \(raw)\n")
        #expect(result.config.otp.method == expected)
        #expect(result.warnings.isEmpty)
    }

    @Test("Other words are errors at the value", arguments: ["IMAP", "Gmail_API", "gmail-api", "pop3", "\"\"", "true", "1"])
    func rejectedMethods(raw: String) throws {
        let error = try #require(loadError("otp:\n  gmail_address: a@example.com\n  method: \(raw)\n"))
        let text = raw == "\"\"" ? "" : raw
        #expect(error.kind == .invalidChoice(path: "otp.method", raw: text, allowed: ["imap", "gmail_api"]))
        #expect(error.location == loc(3, 11))
        #expect(error.message == "otp.method is set to '\(text)', which is not allowed. Use one of: imap, gmail_api.")
    }

    @Test func missingOTPKeysKeepTheDefaults() throws {
        let config = try decodeYAML("otp:\n  gmail_address: a@example.com\n").config
        #expect(config.otp == AppConfig.OTP(gmailAddress: "a@example.com", method: .imap, googleClientID: ""))
    }

    @Test func clientIDIsTextAsWritten() throws {
        let config = try decodeYAML("otp:\n  google_client_id: 123-abc.apps.googleusercontent.com\n").config
        #expect(config.otp.googleClientID == "123-abc.apps.googleusercontent.com")
        #expect(config.otp.method == .imap)
    }

    @Test("Client ID rejects control characters", arguments: [
        ("\"id\\n\"", UInt32(0x0A)), ("\"\\tid\"", UInt32(0x09)), ("\"id\\u2028\"", UInt32(0x2028)), ("\"\\x7F\"", UInt32(0x7F)),
    ])
    func clientIDControlCharacters(literal: String, scalar: UInt32) throws {
        let error = try #require(loadError("otp:\n  google_client_id: \(literal)\n"))
        #expect(error.kind == .invalidCharacter(path: "otp.google_client_id", scalar: scalar))
        #expect(error.location == loc(2, 21))
    }
}
