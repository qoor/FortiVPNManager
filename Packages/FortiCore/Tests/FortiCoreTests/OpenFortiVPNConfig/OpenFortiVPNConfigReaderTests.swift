import Foundation
import Testing
@testable import FortiCore

@Suite struct OpenFortiVPNConfigReaderTests {
    static let digestA = "4d1b72371c42a69bd949048883897aa673059f2e7b1cc992a650584132aae8f3"
    static let digestB = "e0ef56d6eb603d9ffe87e74c28b8210b0ea93839e2ea4fb64f1fc2c66e95e391"

    /// Same shape as the user's real ~/.config/openfortivpn/config (values masked). `insecure-cert`
    /// is not an openfortivpn key; it was harmless only because it was the last line.
    static let userFileShape = """
        host = 203.0.113.10
        port = 8443
        username = alice
        password = s3cr3t
        trusted-cert = \(digestA)
        insecure-cert = 1

        """

    @Test func fixtureDigestsAreWellFormed() {
        #expect(OpenFortiVPNConfigSyntax.isDigest(Self.digestA))
        #expect(OpenFortiVPNConfigSyntax.isDigest(Self.digestB))
    }

    @Test func importsEveryValueFromUserFileShape() {
        let config = OpenFortiVPNConfigReader.parse(Self.userFileShape)
        #expect(config == ImportedOpenFortiVPNConfig(
            host: "203.0.113.10",
            port: 8443,
            username: "alice",
            password: "s3cr3t",
            trustedCerts: [Self.digestA],
            unknownKeys: ["insecure-cert"]
        ))
        #expect(config.effectivePort == 8443)
        #expect(config.invalidLines.isEmpty)
    }

    @Test func unknownKeyBeforeOtherKeysDoesNotStopReading() {
        let config = OpenFortiVPNConfigReader.parse("""
            insecure-cert = 1
            host = 203.0.113.10
            port = 8443
            username = alice
            password = s3cr3t
            trusted-cert = \(Self.digestA)
            """)
        #expect(config.host == "203.0.113.10")
        #expect(config.port == 8443)
        #expect(config.username == "alice")
        #expect(config.password == "s3cr3t")
        #expect(config.trustedCerts == [Self.digestA])
        #expect(config.unknownKeys == ["insecure-cert"])
    }

    @Test func unknownKeysInTheMiddleAreReportedOnceInOrder() {
        let config = OpenFortiVPNConfigReader.parse("""
            host = vpn.example.com
            insecure-cert = 1
            port = 10443
            bogus = x
            insecure-cert = 0
            username = alice
            """)
        #expect(config.host == "vpn.example.com")
        #expect(config.port == 10443)
        #expect(config.username == "alice")
        #expect(config.unknownKeys == ["insecure-cert", "bogus"])
        #expect(config.invalidLines.isEmpty)
    }

    @Test func missingPortIsNilAndDefaultsTo443() {
        let config = OpenFortiVPNConfigReader.parse("host = vpn.example.com\nusername = alice\n")
        #expect(config.port == nil)
        #expect(config.effectivePort == 443)
        #expect(ImportedOpenFortiVPNConfig.defaultPort == 443)
    }

    @Test func uppercaseDigestIsNormalizedToLowercase() {
        let config = OpenFortiVPNConfigReader.parse("trusted-cert = \(Self.digestA.uppercased())\n")
        #expect(config.trustedCerts == [Self.digestA])
        #expect(config.invalidLines.isEmpty)
    }

    @Test func multipleDigestsAreKeptInOrderDeduplicatedAndInvalidOnesReported() {
        let config = OpenFortiVPNConfigReader.parse("""
            trusted-cert = \(Self.digestB)
            trusted-cert = \(Self.digestA)
            trusted-cert = \(Self.digestB.uppercased())
            trusted-cert = \(Self.digestA.dropLast())
            trusted-cert = \(Self.digestA)0
            trusted-cert = \(Self.digestA.dropLast())g
            trusted-cert = \(Self.digestA.prefix(32)) \(Self.digestA.suffix(31))
            trusted-cert =
            """)
        #expect(config.trustedCerts == [Self.digestB, Self.digestA])
        #expect(config.invalidLines == [4, 5, 6, 7, 8])
    }

    @Test(arguments: [
        "host=vpn.example.com\nport=8443\nusername=alice\n",
        "host = vpn.example.com\nport = 8443\nusername = alice\n",
        "  host   =   vpn.example.com   \n\tport\t=\t8443\t\nusername= alice\n",
        "host = vpn.example.com\r\nport = 8443\r\nusername = alice\r\n",
        "host = vpn.example.com\r\n\r\nport = 8443\r\n# comment\r\nusername = alice",
        "\u{FEFF}host = vpn.example.com\nport = 8443\nusername = alice\n",
    ])
    func acceptsSpacingVariantsCRLFAndBOM(_ text: String) {
        let config = OpenFortiVPNConfigReader.parse(text)
        #expect(config == ImportedOpenFortiVPNConfig(host: "vpn.example.com", port: 8443, username: "alice"))
    }

    @Test func skipsCommentsAndBlankLinesButKeepsHashInsideValues() {
        let config = OpenFortiVPNConfigReader.parse("""
            # openfortivpn config
            #host = commented.example.com

               # indented comment = with equals sign
            \t
            host = vpn.example.com
            password = p#ss # not a comment
            realm = staff
            """)
        #expect(config.host == "vpn.example.com")
        #expect(config.password == "p#ss # not a comment")
        #expect(config.realm == "staff")
        #expect(config.unknownKeys.isEmpty)
        #expect(config.invalidLines.isEmpty)
    }

    @Test func splitsAtFirstEqualsSign() {
        let config = OpenFortiVPNConfigReader.parse("password = a=b==\nusername==alice\n")
        #expect(config.password == "a=b==")
        #expect(config.username == "=alice")
    }

    @Test func reportsMalformedLinesWithOneBasedNumbers() {
        let config = OpenFortiVPNConfigReader.parse("""
            host = vpn.example.com
            this line has no equals sign
            = value without key
            port = 8443
            """)
        #expect(config.host == "vpn.example.com")
        #expect(config.port == 8443)
        #expect(config.invalidLines == [2, 3])
    }

    @Test func laterLinesOverrideEarlierOnesAndEmptyValuesClear() {
        let config = OpenFortiVPNConfigReader.parse("""
            host = first.example.com
            host = second.example.com
            username = alice
            username =
            realm =
            password =\t
            """)
        #expect(config.host == "second.example.com")
        #expect(config.username == nil)
        #expect(config.realm == nil)
        #expect(config.password == nil)
        #expect(config.invalidLines.isEmpty)
    }

    @Test func rejectedPortKeepsThePreviousValue() {
        let config = OpenFortiVPNConfigReader.parse("port = 8443\nport = 0\nport = 70000\n")
        #expect(config.port == 8443)
        #expect(config.invalidLines == [2, 3])
    }

    @Test(arguments: [
        ("8443", 8443),
        ("1", 1),
        ("65535", 65535),
        ("+443", 443),
        ("8443 # office", 8443),  // strtol stops at the first non-digit
        ("8443abc", 8443),
        ("1e3", 1),
        ("0x20FB", 8443),  // strtol base 0: hex
        ("0X20fb", 8443),
        ("010", 8),  // strtol base 0: octal
        ("0443", 291),
    ] as [(String, Int)])
    func parsesPortLikeStrtolBase0(value: String, expected: Int) {
        let config = OpenFortiVPNConfigReader.parse("port = \(value)\n")
        #expect(config.port == expected)
        #expect(config.invalidLines.isEmpty)
    }

    @Test(arguments: ["0", "-1", "-8443", "65536", "99999999999999999999999", "abc", "0x", "08443"])
    func rejectsPortsOpenFortiVPNRejects(_ value: String) {
        let config = OpenFortiVPNConfigReader.parse("port = \(value)\n")
        #expect(config.port == nil)
        #expect(config.invalidLines == [1])
    }

    @Test func emptyPortValueIsInvalid() {
        let config = OpenFortiVPNConfigReader.parse("port =\n")
        #expect(config.port == nil)
        #expect(config.invalidLines == [1])
    }

    @Test func distinguishesIgnoredOpenFortiVPNOptionsFromUnknownKeys() {
        let config = OpenFortiVPNConfigReader.parse("""
            set-dns = 0
            pppd-use-peerdns = 1
            otp = 123456
            pppd = 1
            Host = upper.example.com
            set-dns = 1
            host = vpn.example.com
            """)
        #expect(config.ignoredKeys == ["set-dns", "pppd-use-peerdns", "otp"])
        // `pppd` only exists in non-pppd builds; keys are case-sensitive (strcmp).
        #expect(config.unknownKeys == ["pppd", "Host"])
        #expect(config.host == "vpn.example.com")
    }

    @Test func controlCharactersInImportedValuesAreReportedNotImported() {
        let config = OpenFortiVPNConfigReader.parse("""
            host = vpn\u{1B}[31m.example.com
            username = ali\u{85}ce
            realm = st\u{2028}aff
            password = tab\tinside
            """)
        #expect(config.host == nil)
        #expect(config.username == nil)
        #expect(config.realm == nil)
        #expect(config.password == "tab\tinside")
        #expect(config.invalidLines == [1, 2, 3])
    }

    @Test func classicMacLineEndingsDoNotProduceAHostWithCarriageReturns() {
        let config = OpenFortiVPNConfigReader.parse("host = vpn.example.com\rport = 8443\r")
        #expect(config.host == nil)
        #expect(config.port == nil)
        #expect(config.invalidLines == [1])
    }

    @Test func nulByteMakesTheLineInvalid() {
        let config = OpenFortiVPNConfigReader.parse("host = vpn\0.example.com\nport = 8443\n")
        #expect(config.host == nil)
        #expect(config.port == 8443)
        #expect(config.invalidLines == [1])
    }

    @Test func invalidUTF8IsReportedPerLineInsteadOfDecodedLossily() {
        var data = Data("host = vpn.example.com\npassword = caf".utf8)
        data.append(0xE9)  // Latin-1 "é"
        data.append(contentsOf: Data("\n\u{FF}key = 1\n".utf8))
        data.append(contentsOf: [0xFF, 0x3D, 0x31, 0x0A])  // "\xFF=1"
        data.append(contentsOf: Data("username = alice\n".utf8))

        let config = OpenFortiVPNConfigReader.parse(data: data)
        #expect(config.host == "vpn.example.com")
        #expect(config.password == nil)
        #expect(config.username == "alice")
        #expect(config.unknownKeys == ["\u{FF}key"])
        #expect(config.invalidLines == [2, 4])
    }

    @Test func dataAndStringParsingAgree() {
        #expect(OpenFortiVPNConfigReader.parse(data: Data(Self.userFileShape.utf8)) == OpenFortiVPNConfigReader.parse(Self.userFileShape))
    }

    @Test(arguments: ["", "\n", "\n\n\n", "# only a comment\n", "   "])
    func emptyOrCommentOnlyFilesImportNothing(_ text: String) {
        #expect(OpenFortiVPNConfigReader.parse(text) == ImportedOpenFortiVPNConfig())
    }

    @Test func lastLineWithoutNewlineIsRead() {
        let config = OpenFortiVPNConfigReader.parse("host = vpn.example.com\nrealm = staff")
        #expect(config.realm == "staff")
    }

    @Test func descriptionRedactsThePassword() {
        let config = OpenFortiVPNConfigReader.parse(Self.userFileShape)
        #expect(!config.description.contains("s3cr3t"))
        #expect(!String(reflecting: config).contains("s3cr3t"))
        #expect(!"\(config)".contains("s3cr3t"))
        #expect(config.description.contains("<redacted>"))
        #expect(config.description.contains("203.0.113.10"))
    }

    @Test func defaultCandidatePathsAreUserConfigThenHomebrew() {
        let home = URL(filePath: "/Users/alice", directoryHint: .isDirectory)
        let paths = OpenFortiVPNConfigReader.defaultCandidatePaths(home: home)
        #expect(paths.map { $0.path(percentEncoded: false) } == [
            "/Users/alice/.config/openfortivpn/config",
            "/opt/homebrew/etc/openfortivpn/openfortivpn/config",
        ])
        #expect(paths.allSatisfy { $0.isFileURL })
    }
}

/// The byte-level model of openfortivpn's `load_config()` that the renderer's round-trip check uses.
@Suite struct OpenFortiVPNConfigSyntaxTests {
    private func entries(_ text: String) -> [[String]] {
        OpenFortiVPNConfigSyntax.entriesAsReadByOpenFortiVPN(Array(text.utf8)).map {
            [String(decoding: $0.key, as: UTF8.self), String(decoding: $0.value, as: UTF8.self)]
        }
    }

    @Test func matchesLoadConfigTokenization() {
        #expect(entries("host = vpn\n") == [["host", "vpn"]])
        #expect(entries("host=vpn") == [["host", "vpn"]])
        #expect(entries("a = b = c\n") == [["a", "b = c"]])
        #expect(entries("password = p#ss\n") == [["password", "p#ss"]])
        #expect(entries("host = vpn\r\n") == [["host", "vpn"]])
        #expect(entries(" \t host \u{0B}=\u{0C} vpn \t\n") == [["host", "vpn"]])
        #expect(entries("key =\n") == [["key", ""]])
    }

    @Test func onlyColumnZeroHashIsAComment() {
        #expect(entries("#host = vpn\n").isEmpty)
        // openfortivpn reads this as the unknown key "# host" and stops parsing the file.
        #expect(entries("  # host = vpn\n") == [["# host", "vpn"]])
    }

    @Test func skipsShortAndMalformedLines() {
        #expect(entries("\n").isEmpty)
        #expect(entries("x\n").isEmpty)  // no '=' ("Bad line")
        #expect(entries("\r\n").isEmpty)
        #expect(entries("=").isEmpty)  // getline() returned 1 byte
        #expect(entries("=\n") == [["", ""]])
    }

    @Test func nulEndsTheCString() {
        #expect(entries("host = vpn\0evil\n") == [["host", "vpn"]])
        #expect(entries("host\0 = vpn\n").isEmpty)
    }

    @Test func lineSplittingFollowsGetline() {
        let lines = OpenFortiVPNConfigSyntax.lines(of: Array("a\n\nb".utf8)[...])
        #expect(lines.map { Array($0.bytes) } == [Array("a".utf8), [], Array("b".utf8)])
        #expect(lines.map(\.hasNewline) == [true, true, false])
        #expect(OpenFortiVPNConfigSyntax.lines(of: Array("a\n".utf8)[...]).count == 1)
        #expect(OpenFortiVPNConfigSyntax.lines(of: ArraySlice<UInt8>()).isEmpty)
    }

    @Test(arguments: [
        ("443", 443 as Int?), ("  443", 443), ("-0", nil), ("0x1BB", 443), ("0x1bbz", 443), ("0xg", nil),
        ("00", nil), ("0", nil), ("077", 63), ("09", nil), ("65535", 65535), ("65536", nil), ("", nil), ("+", nil),
    ] as [(String, Int?)])
    func portMatchesStrtol(value: String, expected: Int?) {
        #expect(OpenFortiVPNConfigSyntax.port(from: Array(value.utf8)) == expected)
    }
}
