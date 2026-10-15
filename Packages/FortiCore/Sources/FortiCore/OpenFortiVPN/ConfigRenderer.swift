import FortiShared

/// Renders the openfortivpn config file that the root helper passes with `-c`.
///
/// The helper never trusts the app, so every field is re-validated here and the output is limited
/// to `host`, `port`, `username`, optional `realm` and one `trusted-cert` line per digest. The
/// password is never written to the file; it goes to openfortivpn's stdin (`validatePassword`).
public enum ConfigRenderer {
    /// A field the helper refuses to pass to openfortivpn. Descriptions never include the value.
    public struct ValidationError: Error, Equatable, Sendable, CustomStringConvertible {
        public enum Field: String, Sendable {
            case host
            case port
            case username
            case realm
            case trustedCert = "trusted-cert"
            case password
        }

        public enum Problem: Equatable, Sendable {
            case empty
            case tooLong(maxBytes: Int)
            case outOfRange
            /// Code point of the first forbidden character (controls, U+2028, U+2029, U+FEFF; CR/LF/NUL for the password).
            case forbiddenCharacter(codePoint: UInt32)
            case leadingOrTrailingWhitespace
            /// Neither an LDH hostname (labels of 1-63 letters, digits, hyphens) nor a dotted-quad IPv4 address.
            case invalidHost
            /// The digest at `index` is not 64 lowercase hex characters.
            case invalidDigest(index: Int)
            case tooManyDigests(max: Int)
            /// openfortivpn would not read the rendered line back as exactly this value.
            case notRoundTrippable
        }

        public let field: Field
        public let problem: Problem

        public init(field: Field, problem: Problem) {
            self.field = field
            self.problem = problem
        }

        public var description: String {
            let detail: String
            switch problem {
            case .empty:
                detail = "must not be empty"
            case .tooLong(let maxBytes):
                detail = "must be at most \(maxBytes) UTF-8 bytes"
            case .outOfRange:
                detail = "must be between 1 and 65535"
            case .forbiddenCharacter(let codePoint):
                let hex = String(codePoint, radix: 16, uppercase: true)
                detail = "contains forbidden character U+\(String(repeating: "0", count: max(0, 4 - hex.count)))\(hex)"
            case .leadingOrTrailingWhitespace:
                detail = "must not start or end with whitespace"
            case .invalidHost:
                detail = "must be a DNS hostname (letters, digits and hyphens, labels of 1-63) or a dotted-quad IPv4 address"
            case .invalidDigest(let index):
                detail = "entry \(index) is not 64 lowercase hex characters"
            case .tooManyDigests(let max):
                detail = "allows at most \(max) digests"
            case .notRoundTrippable:
                detail = "would not be read back unchanged by openfortivpn"
            }
            return "\(field.rawValue) \(detail)"
        }
    }

    // Byte limits are the openfortivpn 1.24.1 buffer sizes in src/config.h: load_config() silently
    // truncates longer values with strncpy, which would make openfortivpn use a different value.
    static let maxHostBytes = 253  // GATEWAY_HOST_SIZE
    static let maxUsernameBytes = 64  // USERNAME_SIZE
    static let maxRealmBytes = 63  // REALM_SIZE
    static let maxTrustedCerts = 8

    /// read_password() (src/userinput.c) is called from main.c with `cfg.password, PASSWORD_SIZE`
    /// (256). It copies at most 256 bytes and stops early only at '\n' or EOF. A password of 256 bytes
    /// or more is therefore truncated and its terminating '\n' stays unread in stdin's buffer, where
    /// the next read_password() call (the OTP prompt) would consume it as an empty answer. Strictly
    /// fewer than 256 bytes guarantees the '\n' we write is consumed together with the password.
    static let maxPasswordBytes = 255

    /// Validates `request` (including the password) and returns the config file contents.
    public static func render(_ request: ConnectRequest) throws(ValidationError) -> String {
        try validate(request)

        var entries: [(field: ValidationError.Field, key: String, value: String)] = [
            (.host, "host", request.host),
            (.port, "port", String(request.port)),
            (.username, "username", request.username),
        ]
        if let realm = request.realm, !realm.isEmpty {
            entries.append((.realm, "realm", realm))
        }
        for digest in request.trustedCerts {
            entries.append((.trustedCert, "trusted-cert", digest))
        }
        let text = entries.map { "\($0.key) = \($0.value)\n" }.joined()

        // Defense in depth: read the output back exactly as openfortivpn's load_config() would.
        let reread = OpenFortiVPNConfigSyntax.entriesAsReadByOpenFortiVPN(Array(text.utf8))
        for (index, entry) in entries.enumerated() {
            guard index < reread.count,
                  reread[index].key == Array(entry.key.utf8),
                  reread[index].value == Array(entry.value.utf8)
            else { throw ValidationError(field: entry.field, problem: .notRoundTrippable) }
        }
        guard reread.count == entries.count else {
            throw ValidationError(field: entries[entries.count - 1].field, problem: .notRoundTrippable)
        }
        return text
    }

    /// Runs every check `render` runs, without producing output. The app can use this to catch a
    /// config the helper would reject before connecting.
    public static func validate(_ request: ConnectRequest) throws(ValidationError) {
        try validateHost(request.host)

        guard (1...65_535).contains(request.port) else { throw ValidationError(field: .port, problem: .outOfRange) }

        try validateUsername(request.username)
        try validateRealm(request.realm)

        guard request.trustedCerts.count <= maxTrustedCerts else {
            throw ValidationError(field: .trustedCert, problem: .tooManyDigests(max: maxTrustedCerts))
        }
        for (index, digest) in request.trustedCerts.enumerated() where !OpenFortiVPNConfigSyntax.isDigest(digest) {
            throw ValidationError(field: .trustedCert, problem: .invalidDigest(index: index))
        }

        try validatePassword(request.password)
    }

    /// Checks a password before it is written (followed by '\n') to openfortivpn's stdin:
    /// non-empty, no CR/LF/NUL, fewer than 256 UTF-8 bytes. Leading/trailing spaces are allowed.
    public static func validatePassword(_ password: String) throws(ValidationError) {
        guard !password.isEmpty else { throw ValidationError(field: .password, problem: .empty) }
        // LF ends read_password() early, CR would become part of the password (pipe, no tty
        // translation), NUL ends the C string.
        if let bad = password.unicodeScalars.first(where: { $0 == "\n" || $0 == "\r" || $0 == "\0" }) {
            throw ValidationError(field: .password, problem: .forbiddenCharacter(codePoint: bad.value))
        }
        guard password.utf8.count <= maxPasswordBytes else {
            throw ValidationError(field: .password, problem: .tooLong(maxBytes: maxPasswordBytes))
        }
    }

    // MARK: - Field rules

    /// The `host` rules of `validate`: non-empty, at most 253 bytes, no forbidden characters or
    /// surrounding whitespace, and an LDH hostname or dotted-quad IPv4 address (no scheme, port or path).
    public static func validateHost(_ host: String) throws(ValidationError) {
        try validateText(host, field: .host, maxBytes: maxHostBytes)
        guard isValidHost(host) else { throw ValidationError(field: .host, problem: .invalidHost) }
    }

    /// The `username` rules of `validate`: non-empty, at most 64 bytes, no forbidden characters or
    /// surrounding whitespace.
    public static func validateUsername(_ username: String) throws(ValidationError) {
        try validateText(username, field: .username, maxBytes: maxUsernameBytes)
    }

    /// The `realm` rules of `validate`: `nil` or empty means no realm; otherwise at most 63 bytes, no
    /// forbidden characters or surrounding whitespace.
    public static func validateRealm(_ realm: String?) throws(ValidationError) {
        guard let realm, !realm.isEmpty else { return }
        try validateText(realm, field: .realm, maxBytes: maxRealmBytes)
    }

    private static func validateText(_ value: String, field: ValidationError.Field, maxBytes: Int) throws(ValidationError) {
        guard !value.isEmpty else { throw ValidationError(field: field, problem: .empty) }
        guard value.utf8.count <= maxBytes else { throw ValidationError(field: field, problem: .tooLong(maxBytes: maxBytes)) }
        if let bad = value.unicodeScalars.first(where: isForbidden) {
            throw ValidationError(field: field, problem: .forbiddenCharacter(codePoint: bad.value))
        }
        // Stricter than config.c's ASCII isspace() trimming: no Unicode whitespace at either end.
        guard let first = value.unicodeScalars.first, let last = value.unicodeScalars.last,
              !first.properties.isWhitespace, !last.properties.isWhitespace
        else { throw ValidationError(field: field, problem: .leadingOrTrailingWhitespace) }
    }

    /// Cc controls (includes U+0085 NEL), the Unicode line/paragraph separators and the BOM.
    private static func isForbidden(_ scalar: Unicode.Scalar) -> Bool {
        scalar.properties.generalCategory == .control
            || scalar == "\u{2028}" || scalar == "\u{2029}" || scalar == "\u{FEFF}"
    }

    static func isValidHost(_ host: String) -> Bool {
        isCanonicalIPv4(host) || isLDHHostname(host)
    }

    /// Four decimal octets 0-255 without leading zeros (inet_aton would read `010` as octal 8).
    static func isCanonicalIPv4(_ host: String) -> Bool {
        let parts = host.utf8.split(separator: UInt8(ascii: "."), omittingEmptySubsequences: false)
        return parts.count == 4 && parts.allSatisfy { part in
            guard (1...3).contains(part.count), part.allSatisfy(isASCIIDigit) else { return false }
            if part.count > 1, part.first == UInt8(ascii: "0") { return false }
            return part.reduce(0) { $0 * 10 + Int($1 - UInt8(ascii: "0")) } <= 255
        }
    }

    /// RFC 1123 LDH labels of 1-63 bytes, no leading/trailing hyphen, no trailing dot. The last label
    /// must not be numeric (RFC 3696 §2), so getaddrinfo can never read it as a shorthand or hex IPv4
    /// address such as `127.1`, `1.2.3`, `999.1.1.1` or `0x7f000001`.
    static func isLDHHostname(_ host: String) -> Bool {
        let labels = host.utf8.split(separator: UInt8(ascii: "."), omittingEmptySubsequences: false)
        guard let last = labels.last else { return false }
        let hyphen = UInt8(ascii: "-")
        for label in labels {
            guard (1...63).contains(label.count),
                  label.allSatisfy({ isASCIIDigit($0) || isASCIILetter($0) || $0 == hyphen }),
                  label.first != hyphen, label.last != hyphen
            else { return false }
        }
        let isDecimal = last.allSatisfy(isASCIIDigit)
        let isHex = last.count >= 2 && last.first == UInt8(ascii: "0")
            && (last.dropFirst().first == UInt8(ascii: "x") || last.dropFirst().first == UInt8(ascii: "X"))
            && last.dropFirst(2).allSatisfy { isASCIIDigit($0) || (UInt8(ascii: "a")...UInt8(ascii: "f")).contains($0 | 0x20) }
        return !isDecimal && !isHex
    }

    private static func isASCIIDigit(_ byte: UInt8) -> Bool {
        (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(byte)
    }

    private static func isASCIILetter(_ byte: UInt8) -> Bool {
        (UInt8(ascii: "a")...UInt8(ascii: "z")).contains(byte | 0x20)
    }
}
