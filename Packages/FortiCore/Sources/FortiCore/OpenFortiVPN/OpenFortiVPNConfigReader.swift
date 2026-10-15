import Foundation

/// Values imported from an existing openfortivpn config file (Account tab, "import existing
/// openfortivpn config"). The source file is only read, never modified.
public struct ImportedOpenFortiVPNConfig: Equatable, Sendable {
    /// openfortivpn's port when the config file has no usable `port` line.
    public static let defaultPort = 443

    public var host: String?
    /// `nil` when the file has no usable `port` line; see `effectivePort`.
    public var port: Int?
    public var username: String?
    /// Plaintext password from the file (destined for the Keychain). Redacted from `description`.
    public var password: String?
    public var realm: String?
    /// Valid `trusted-cert` digests, lowercased and deduplicated, in file order.
    public var trustedCerts: [String]
    /// Keys openfortivpn 1.24.1 does not know (e.g. `insecure-cert`), deduplicated, in file order.
    /// openfortivpn stops reading the file at the first one; this reader keeps going.
    public var unknownKeys: [String]
    /// Valid openfortivpn options that FortiVPNManager does not carry over (e.g. `set-dns`, `otp`).
    public var ignoredKeys: [String]
    /// 1-based numbers of lines that could not be used: no `=`, empty key, undecodable bytes,
    /// control characters, a port openfortivpn would reject, or a malformed digest.
    public var invalidLines: [Int]

    public init(
        host: String? = nil,
        port: Int? = nil,
        username: String? = nil,
        password: String? = nil,
        realm: String? = nil,
        trustedCerts: [String] = [],
        unknownKeys: [String] = [],
        ignoredKeys: [String] = [],
        invalidLines: [Int] = []
    ) {
        self.host = host
        self.port = port
        self.username = username
        self.password = password
        self.realm = realm
        self.trustedCerts = trustedCerts
        self.unknownKeys = unknownKeys
        self.ignoredKeys = ignoredKeys
        self.invalidLines = invalidLines
    }

    /// The port openfortivpn would connect to: `port`, or 443 when the file has none.
    public var effectivePort: Int { port ?? Self.defaultPort }
}

extension ImportedOpenFortiVPNConfig: CustomStringConvertible, CustomDebugStringConvertible {
    public var description: String {
        "ImportedOpenFortiVPNConfig(host: \(host.debugDescription), port: \(port.map(String.init) ?? "nil"), "
            + "username: \(username.debugDescription), password: \(password == nil ? "nil" : "<redacted>"), "
            + "realm: \(realm.debugDescription), trustedCerts: \(trustedCerts), unknownKeys: \(unknownKeys), "
            + "ignoredKeys: \(ignoredKeys), invalidLines: \(invalidLines))"
    }

    public var debugDescription: String { description }
}

/// Lenient reader for openfortivpn config files (`key = value` lines, `#` comments).
///
/// Mirrors openfortivpn 1.24.1 `load_config()` (src/config.c) for the keys it imports, except that it
/// reads every known key to the end of the file instead of stopping at the first unknown key, and it
/// also treats indented `#` lines as comments.
public enum OpenFortiVPNConfigReader {
    /// Files to try, in order: `~/.config/openfortivpn/config`, then Homebrew's
    /// `/opt/homebrew/etc/openfortivpn/openfortivpn/config`.
    public static func defaultCandidatePaths(home: URL) -> [URL] {
        [
            home.appending(path: ".config/openfortivpn/config", directoryHint: .notDirectory),
            URL(filePath: "/opt/homebrew/etc/openfortivpn/openfortivpn/config", directoryHint: .notDirectory),
        ]
    }

    public static func parse(_ text: String) -> ImportedOpenFortiVPNConfig {
        parse(bytes: Array(text.utf8))
    }

    /// Parses raw file contents. Lines whose imported value is not valid UTF-8 are reported in
    /// `invalidLines` instead of being decoded lossily (a mangled password would cause lockouts).
    public static func parse(data: Data) -> ImportedOpenFortiVPNConfig {
        parse(bytes: Array(data))
    }

    /// Keys this reader imports.
    private static let importedKeys: Set<String> = ["host", "port", "username", "password", "realm", "trusted-cert"]

    /// Every other key `load_config()` accepts in the macOS build of 1.24.1 (built with
    /// `/usr/sbin/pppd`, so the `pppd` key of the non-pppd branch is unknown there).
    private static let otherKnownKeys: Set<String> = [
        "otp", "otp-prompt", "otp-delay", "cookie", "cookie-on-stdin", "no-ftm-push", "pinentry",
        "set-dns", "sni", "set-routes", "half-internet-routes", "persistent",
        "pppd-use-peerdns", "pppd-log", "pppd-plugin", "pppd-ipparam", "pppd-ifname", "pppd-call",
        "pppd-accept-remote", "ppp-system", "use-resolvconf", "use-syslog",
        "ca-file", "user-cert", "saml-login", "user-key", "pem-passphrase", "insecure-ssl",
        "cipher-list", "min-tls", "seclevel-1", "user-agent", "hostcheck", "check-virtual-desktop",
    ]

    static func parse(bytes: [UInt8]) -> ImportedOpenFortiVPNConfig {
        typealias Syntax = OpenFortiVPNConfigSyntax
        var result = ImportedOpenFortiVPNConfig()
        var body = bytes[...]
        if body.starts(with: [0xEF, 0xBB, 0xBF]) { body = body.dropFirst(3) }  // UTF-8 BOM

        for (offset, line) in Syntax.lines(of: body).enumerated() {
            let lineNumber = offset + 1
            let content = Syntax.trimmed(line.bytes)
            if content.isEmpty || content.first == UInt8(ascii: "#") { continue }

            guard !line.bytes.contains(0),
                  let (keyBytes, valueBytes) = Syntax.split(line.bytes),
                  let key = String(validating: keyBytes, as: UTF8.self), !key.isEmpty
            else {
                result.invalidLines.append(lineNumber)
                continue
            }
            guard importedKeys.contains(key) else {
                if otherKnownKeys.contains(key) {
                    appendIfAbsent(key, to: &result.ignoredKeys)
                } else {
                    appendIfAbsent(key, to: &result.unknownKeys)
                }
                continue
            }
            guard let value = String(validating: valueBytes, as: UTF8.self),
                  !value.unicodeScalars.contains(where: isDisallowedInValue)
            else {
                result.invalidLines.append(lineNumber)
                continue
            }

            // Later lines override earlier ones (as in openfortivpn); an empty value clears the field.
            switch key {
            case "host": result.host = value.isEmpty ? nil : value
            case "username": result.username = value.isEmpty ? nil : value
            case "password": result.password = value.isEmpty ? nil : value
            case "realm": result.realm = value.isEmpty ? nil : value
            case "port":
                // Like openfortivpn, a rejected port leaves the previous value in place.
                if let port = Syntax.port(from: valueBytes) {
                    result.port = port
                } else {
                    result.invalidLines.append(lineNumber)
                }
            case "trusted-cert":
                // openfortivpn compares against lowercase hex, so uppercase digests never matched
                // there; normalizing makes them useful (same rule as config.yaml `trusted_certs`).
                let digest = value.lowercased()
                if Syntax.isDigest(digest) {
                    appendIfAbsent(digest, to: &result.trustedCerts)
                } else {
                    result.invalidLines.append(lineNumber)
                }
            default:
                break
            }
        }
        return result
    }

    private static func appendIfAbsent(_ element: String, to array: inout [String]) {
        if !array.contains(element) { array.append(element) }
    }

    /// Control characters (other than TAB) cannot be part of a usable host, username, realm or
    /// password, so lines containing them are reported instead of imported.
    private static func isDisallowedInValue(_ scalar: Unicode.Scalar) -> Bool {
        scalar != "\t" && (scalar.properties.generalCategory == .control
            || scalar == "\u{2028}" || scalar == "\u{2029}" || scalar == "\u{FEFF}")
    }
}

/// Byte-level model of how openfortivpn 1.24.1 `load_config()` (src/config.c) tokenizes a file.
///
/// - Lines come from `getline()`: split on LF only, so a CR before the LF stays in the line and is
///   then trimmed as whitespace.
/// - A line is skipped when `getline()` returned fewer than 2 bytes (LF included) or when its very
///   first byte is `#` (an indented `#` is not a comment).
/// - The line is split at the *first* `=`, so values may contain `=` and `#` (no inline comments).
/// - Key and value are trimmed with `isspace()` in the C locale (openfortivpn never calls
///   `setlocale`), i.e. only SP, HT, LF, VT, FF and CR.
/// - Everything is a C string, so a NUL byte ends the line early.
enum OpenFortiVPNConfigSyntax {
    struct Line {
        var bytes: ArraySlice<UInt8>
        /// Whether `getline()` returned a terminating LF with this line.
        var hasNewline: Bool
    }

    static func isSpace(_ byte: UInt8) -> Bool {
        byte == 0x20 || (0x09...0x0D).contains(byte)
    }

    static func trimmed(_ bytes: ArraySlice<UInt8>) -> ArraySlice<UInt8> {
        guard let first = bytes.firstIndex(where: { !isSpace($0) }),
              let last = bytes.lastIndex(where: { !isSpace($0) })
        else { return bytes[bytes.endIndex...] }
        return bytes[first...last]
    }

    /// The lines `getline()` would return, without their LF. A trailing LF does not start a new line.
    static func lines(of bytes: ArraySlice<UInt8>) -> [Line] {
        var segments = bytes.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: false)
        let endsWithNewline = segments.last?.isEmpty == true
        if endsWithNewline { segments.removeLast() }
        return segments.indices.map { index in
            Line(bytes: segments[index], hasNewline: index < segments.count - 1 || endsWithNewline)
        }
    }

    /// Splits at the first `=` and trims both sides. `nil` when there is no `=` ("Bad line").
    static func split(_ line: ArraySlice<UInt8>) -> (key: ArraySlice<UInt8>, value: ArraySlice<UInt8>)? {
        guard let equals = line.firstIndex(of: UInt8(ascii: "=")) else { return nil }
        return (trimmed(line[..<equals]), trimmed(line[line.index(after: equals)...]))
    }

    /// The `(key, value)` pairs `load_config()` would act on, in file order, with its exact skipping
    /// rules. `ConfigRenderer` uses this to prove that its output is read back unchanged.
    static func entriesAsReadByOpenFortiVPN(_ bytes: [UInt8]) -> [(key: [UInt8], value: [UInt8])] {
        lines(of: bytes[...]).compactMap { line in
            let read = line.bytes.count + (line.hasNewline ? 1 : 0)
            if read < 2 || line.bytes.first == UInt8(ascii: "#") { return nil }
            let cString = line.bytes.prefix { $0 != 0 }
            guard let (key, value) = split(cString) else { return nil }
            return (Array(key), Array(value))
        }
    }

    /// `strtol(value, NULL, 0)` followed by config.c's `1...65535` check: decimal, `0x` hex or
    /// leading-zero octal, optional sign, parsing stops at the first non-digit (`8443 # x` is 8443).
    /// `nil` when openfortivpn would print "Bad port" and ignore the line.
    static func port(from value: some Collection<UInt8>) -> Int? {
        var index = value.startIndex
        func peek(_ offset: Int = 0) -> UInt8? {
            let target = value.index(index, offsetBy: offset, limitedBy: value.endIndex) ?? value.endIndex
            return target < value.endIndex ? value[target] : nil
        }
        while let byte = peek(), isSpace(byte) { index = value.index(after: index) }

        var negative = false
        if let sign = peek(), sign == UInt8(ascii: "+") || sign == UInt8(ascii: "-") {
            negative = sign == UInt8(ascii: "-")
            index = value.index(after: index)
        }

        var base = 10
        if peek() == UInt8(ascii: "0") {
            if let x = peek(1), x == UInt8(ascii: "x") || x == UInt8(ascii: "X"),
               let digit = peek(2), digitValue(digit).map({ $0 < 16 }) == true {
                base = 16
                index = value.index(index, offsetBy: 2)
            } else {
                base = 8  // the leading 0 itself parses as an octal digit
            }
        }

        var magnitude = 0
        var sawDigit = false
        while let byte = peek(), let digit = digitValue(byte), digit < base {
            sawDigit = true
            magnitude = min(magnitude * base + digit, 65_536)  // anything above 65535 is rejected anyway
            index = value.index(after: index)
        }
        guard sawDigit else { return nil }  // strtol returns 0
        let port = negative ? -magnitude : magnitude
        return (1...65_535).contains(port) ? port : nil
    }

    private static func digitValue(_ byte: UInt8) -> Int? {
        switch byte {
        case UInt8(ascii: "0")...UInt8(ascii: "9"): Int(byte - UInt8(ascii: "0"))
        case UInt8(ascii: "a")...UInt8(ascii: "z"): Int(byte - UInt8(ascii: "a")) + 10
        case UInt8(ascii: "A")...UInt8(ascii: "Z"): Int(byte - UInt8(ascii: "A")) + 10
        default: nil
        }
    }

    /// `^[0-9a-f]{64}$`: the lowercase hex SHA-256 that tunnel.c compares with `memcmp`.
    static func isDigest(_ string: String) -> Bool {
        string.utf8.count == 64 && string.utf8.allSatisfy {
            (UInt8(ascii: "0")...UInt8(ascii: "9")).contains($0) || (UInt8(ascii: "a")...UInt8(ascii: "f")).contains($0)
        }
    }
}
