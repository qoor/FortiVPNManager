import Foundation

/// 1-based position in `config.yaml`. Columns count Unicode scalars (the libyaml convention);
/// a leading UTF-8 BOM is not counted.
public struct LineColumn: Sendable, Hashable, CustomStringConvertible {
    public var line: Int
    public var column: Int

    public init(line: Int, column: Int) {
        self.line = line
        self.column = column
    }

    /// Localized label, e.g. `Line 3, column 7`.
    public var description: String { String(localized: "Line \(line), column \(column)", bundle: .main) }
}

/// Successful decode: the merged config plus non-fatal findings.
public struct ConfigLoadResult: Sendable, Equatable {
    public var config: AppConfig
    public var warnings: [ConfigWarning]

    public init(config: AppConfig, warnings: [ConfigWarning] = []) {
        self.config = config
        self.warnings = warnings
    }
}

/// Why `config.yaml` (or an in-memory ``AppConfig``) was rejected. The app keeps its last-known-good
/// config and shows ``displayMessage``.
public struct ConfigLoadError: Error, Sendable, Equatable, LocalizedError, CustomStringConvertible {
    /// Expected YAML node kind, used by ``Kind/typeMismatch(path:expected:)``.
    public enum ValueKind: Sendable, Equatable {
        case mapping
        case sequence
        case string
        case integer
        case boolean
    }

    public enum Kind: Sendable, Equatable {
        case fileTooLarge(byteCount: Int, limit: Int)
        case invalidUTF8
        /// A raw character libyaml refuses to read (e.g. a C0 control or U+FFFE), anywhere in the file.
        case disallowedCharacter(scalar: UInt32)
        /// Any other YAML syntax error; `problem` is libyaml's (English) description.
        case syntax(problem: String)
        case multipleDocuments
        /// More than libyaml's 1000 nested collections.
        case nestingTooDeep
        case duplicateKey(String)
        case rootNotMapping
        case anchorOrAlias
        case mergeKey
        /// Explicit YAML tag such as `!!str` or `!custom`.
        case explicitTag(String)
        /// A mapping key that is not a scalar.
        case complexKey
        case typeMismatch(path: String, expected: ValueKind)
        /// Unquoted integer whose text is not `^(0|[1-9][0-9]*)$` (e.g. `0443`, `2:30`, `0x1BB`, `1_0`).
        case invalidInteger(path: String, raw: String)
        /// Unquoted boolean other than `true`/`false` (e.g. `yes`, `on`).
        case invalidBoolean(path: String, raw: String)
        /// A number written in quotes, e.g. `port: "443"`.
        case quotedNumber(path: String, raw: String)
        /// A boolean written in quotes, e.g. `enabled: "true"`.
        case quotedBoolean(path: String, raw: String)
        /// A string that is not one of the accepted words, e.g. `method: pop3` (`allowed` in file spelling).
        case invalidChoice(path: String, raw: String, allowed: [String])
        case outOfRange(path: String, raw: String, range: ClosedRange<Int>)
        case marginNotLessThanSession(margin: Int, session: Int)
        /// A string field contains a control character, line/paragraph separator, BOM or noncharacter.
        case invalidCharacter(path: String, scalar: UInt32)
        /// `schema_version` is newer than this build understands. The file must be treated as read-only.
        case newerSchemaVersion(Int)
        /// Only from ``AppConfig/validate()``: a `trusted_certs` entry is not a normalized digest.
        case invalidTrustedCert(index: Int)
    }

    public var kind: Kind
    public var location: LineColumn?

    public init(_ kind: Kind, at location: LineColumn? = nil) {
        self.kind = kind
        self.location = location
    }

    /// Localized, user-facing explanation without the location.
    public var message: String {
        switch kind {
        case let .fileTooLarge(byteCount, limit):
            return String(localized: "The config file is too large (\(byteCount) bytes). It must be \(limit / 1024) KiB (\(limit) bytes) or smaller.", bundle: .main)
        case .invalidUTF8:
            return String(localized: "The config file is not valid UTF-8 text. Save it as UTF-8.", bundle: .main)
        case let .disallowedCharacter(scalar):
            return String(localized: "The config file contains a character that is not allowed (\(ConfigText.codePoint(scalar))). Remove that character.", bundle: .main)
        case let .syntax(problem):
            return String(localized: "YAML syntax error (\(problem)).", bundle: .main)
        case .multipleDocuments:
            return String(localized: "The config file must contain only one YAML document. Remove the document separator (---).", bundle: .main)
        case .nestingTooDeep:
            return String(localized: "The YAML structure is nested too deeply.", bundle: .main)
        case let .duplicateKey(key):
            return String(localized: "The key '\(key)' appears more than once at the same level. Keep only one.", bundle: .main)
        case .rootNotMapping:
            return String(localized: "The top level of the config file must be in 'key: value' form.", bundle: .main)
        case .anchorOrAlias:
            return String(localized: "Anchors (&) and aliases (*) are not allowed. Write the values out directly.", bundle: .main)
        case .mergeKey:
            return String(localized: "Merge keys (<<) are not allowed. Write the values out directly.", bundle: .main)
        case let .explicitTag(tag):
            return String(localized: "YAML tags (\(tag)) are not allowed. Write just the value, without a tag.", bundle: .main)
        case .complexKey:
            return String(localized: "Keys must be plain strings.", bundle: .main)
        case let .typeMismatch(path, expected):
            return String(localized: "The value of \(path) has the wrong type. \(Self.describe(expected)).", bundle: .main)
        case let .invalidInteger(path, raw):
            return [
                String(localized: "\(path) is set to '\(ConfigText.preview(raw))', which is not an accepted number format.", bundle: .main),
                String(localized: "Write it as a decimal number without leading zeros (e.g. \(ConfigText.leaf(path)): 443).", bundle: .main),
                String(localized: "Formats such as 0443 (octal), 2:30 (base 60), 0x1BB (hex) and 1_000 are not accepted.", bundle: .main),
            ].joined(separator: " ")
        case let .invalidBoolean(path, raw):
            return String(localized: "\(path) is set to '\(ConfigText.preview(raw))', which is not allowed. Use true or false.", bundle: .main)
        case let .quotedNumber(path, raw):
            return [
                String(localized: "\(path) has a quoted value \"\(ConfigText.preview(raw))\".", bundle: .main),
                String(localized: "Write numbers without quotes (e.g. \(ConfigText.leaf(path)): 443).", bundle: .main),
            ].joined(separator: " ")
        case let .quotedBoolean(path, raw):
            return [
                String(localized: "\(path) has a quoted value \"\(ConfigText.preview(raw))\".", bundle: .main),
                String(localized: "Write true/false without quotes (e.g. \(ConfigText.leaf(path)): true).", bundle: .main),
            ].joined(separator: " ")
        case let .invalidChoice(path, raw, allowed):
            let choices = allowed.joined(separator: ", ")
            return String(localized: "\(path) is set to '\(ConfigText.preview(raw))', which is not allowed. Use one of: \(choices).", bundle: .main)
        case let .outOfRange(path, raw, range):
            if range.lowerBound == range.upperBound {
                return String(localized: "\(path) is set to \(ConfigText.preview(raw)), which is not allowed. It must be \(range.lowerBound).", bundle: .main)
            }
            return String(localized: "\(path) is set to \(ConfigText.preview(raw)), which is outside the allowed range (\(range.lowerBound)–\(range.upperBound)).", bundle: .main)
        case let .marginNotLessThanSession(margin, session):
            return String(localized: "\(ConfigPath.marginMinutes) (\(margin)) must be less than \(ConfigPath.sessionMaxMinutes) (\(session)).", bundle: .main)
        case let .invalidCharacter(path, scalar):
            return [
                String(localized: "\(path) contains a character that is not allowed (\(ConfigText.codePoint(scalar))).", bundle: .main),
                String(localized: "Control characters, line breaks, BOMs and similar characters are not allowed.", bundle: .main),
            ].joined(separator: " ")
        case let .newerSchemaVersion(version):
            return [
                String(localized: "This config file was written by a newer version of the app (schema_version \(version)).", bundle: .main),
                String(localized: "Update the app. Until then, this file is treated as read-only and is never overwritten.", bundle: .main),
            ].joined(separator: " ")
        case let .invalidTrustedCert(index):
            return String(localized: "In \(ConfigPath.trustedCerts), entry \(index + 1) is not a SHA-256 digest (64 lowercase hex digits).", bundle: .main)
        }
    }

    /// ``message`` prefixed with the location when known, e.g. `Line 3, column 9: …`.
    public var displayMessage: String {
        guard let location else { return message }
        return "\(location): \(message)"
    }

    public var errorDescription: String? { displayMessage }
    public var description: String { displayMessage }

    private static func describe(_ kind: ValueKind) -> String {
        switch kind {
        case .mapping: return String(localized: "It must be nested entries in 'key: value' form", bundle: .main)
        case .sequence: return String(localized: "It must be a list (e.g. [] or '- value' lines)", bundle: .main)
        case .string: return String(localized: "It must be a string", bundle: .main)
        case .integer: return String(localized: "It must be an unquoted integer", bundle: .main)
        case .boolean: return String(localized: "It must be an unquoted true or false", bundle: .main)
        }
    }
}

/// Non-fatal finding while reading or writing `config.yaml`.
public struct ConfigWarning: Sendable, Equatable, CustomStringConvertible {
    public enum Kind: Sendable, Equatable {
        /// Key this version does not know. It is ignored and disappears on the next save.
        case unknownKey(path: String)
        /// `trusted_certs[index]` is not a SHA-256 hex digest and was dropped.
        case invalidTrustedCert(index: Int)
        /// `trusted_certs[index]` repeats an earlier entry and was dropped.
        case duplicateTrustedCert(index: Int)
        /// The file has `count` hard links; an atomic save replaces only this name.
        case multipleHardLinks(count: Int)
        /// File permissions are not 0600 (fixed on the next save).
        case insecurePermissions(mode: UInt16)
        /// Save changed the permissions from `previousMode` to 0600.
        case permissionsCorrected(previousMode: UInt16)
    }

    public var kind: Kind
    public var location: LineColumn?

    public init(_ kind: Kind, at location: LineColumn? = nil) {
        self.kind = kind
        self.location = location
    }

    /// Localized, user-facing explanation without the location.
    public var message: String {
        switch kind {
        case let .unknownKey(path):
            return String(localized: "Ignoring unknown key '\(path)'. It will be removed the next time you save in Settings.", bundle: .main)
        case let .invalidTrustedCert(index):
            return String(localized: "In \(ConfigPath.trustedCerts), entry \(index + 1) is not a SHA-256 digest (64 hex digits) and is ignored.", bundle: .main)
        case let .duplicateTrustedCert(index):
            return String(localized: "In \(ConfigPath.trustedCerts), entry \(index + 1) duplicates an earlier entry and is ignored.", bundle: .main)
        case let .multipleHardLinks(count):
            return String(localized: "The config file has \(count) hard links. When the app saves, only this path is replaced with a new file; the other links keep the old contents.", bundle: .main)
        case let .insecurePermissions(mode):
            return String(localized: "The config file permissions are \(ConfigText.octal(mode)). They will be changed to 0600 on the next save.", bundle: .main)
        case let .permissionsCorrected(previousMode):
            return String(localized: "Changed the config file permissions from \(ConfigText.octal(previousMode)) to 0600.", bundle: .main)
        }
    }

    /// ``message`` prefixed with the location when known.
    public var displayMessage: String {
        guard let location else { return message }
        return "\(location): \(message)"
    }

    public var description: String { displayMessage }
}
