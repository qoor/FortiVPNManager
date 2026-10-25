import Foundation
import Yams

/// Strict reader for `config.yaml` (DESIGN §5.7, loading and validation). Pure: no file system access.
///
/// Pipeline: size/UTF-8 → `Yams` compose (single document, mapping root) → `schema_version` →
/// reject anchors, aliases, `<<` merge keys and explicit tags → typed, range-checked fields with
/// defaults for missing keys and warnings for unknown keys.
public enum ConfigCodec {
    /// Decodes and validates `config.yaml` bytes. An empty document (or one with only comments)
    /// yields the defaults.
    public static func decode(_ data: Data) throws(ConfigLoadError) -> ConfigLoadResult {
        guard data.count <= ConfigLimits.maxFileSize else {
            throw ConfigLoadError(.fileTooLarge(byteCount: data.count, limit: ConfigLimits.maxFileSize))
        }
        let text: String
        switch ConfigText.decodeUTF8(data) {
        case let .text(decoded): text = decoded
        case let .invalid(location): throw ConfigLoadError(.invalidUTF8, at: location)
        }
        // Yams builds the node tree recursively, up to libyaml's 1000-level nesting cap. That needs
        // more than the 512 KiB stack of a secondary (e.g. Swift concurrency) thread, so a
        // pathological file must not be composed on the caller's stack.
        return try LargeStackThread.run { composeResult(text: text) }.get()
    }

    /// The bytes ``ConfigTemplate`` produces for `config` (UTF-8).
    public static func encode(_ config: AppConfig) -> Data {
        Data(ConfigTemplate.render(config).utf8)
    }

    // MARK: - Compose

    private static func composeResult(text: String) -> Result<ConfigLoadResult, ConfigLoadError> {
        do {
            return .success(try compose(text: text))
        } catch {
            return .failure(error)
        }
    }

    private static func compose(text: String) throws(ConfigLoadError) -> ConfigLoadResult {
        // `Parser` (not `Yams.compose`) so it stays alive while we walk the tree: `Node.anchor`
        // is a weak reference owned by the parser and would read as nil once it is released.
        // A new parser per call; Yams types are not Sendable.
        let parser: Parser
        let root: Node?
        do {
            parser = try Parser(yaml: text, resolver: .default, constructor: .default, encoding: .utf8)
            root = try parser.singleRoot()
        } catch {
            throw mapYamsError(error, text: text)
        }
        return try withExtendedLifetime(parser) { () throws(ConfigLoadError) -> ConfigLoadResult in
            guard let root, !isNull(root) else {
                return ConfigLoadResult(config: AppConfig())
            }
            guard case let .mapping(rootMapping) = root else {
                if root.anchor != nil { throw ConfigLoadError(.anchorOrAlias, at: location(of: root)) }
                throw ConfigLoadError(.rootNotMapping, at: location(of: root))
            }
            // A file from a newer app may legitimately violate this version's rules, so the version
            // is judged before anything else in the document.
            let versionEntry = rootMapping.first { $0.key.scalar?.string == ConfigPath.schemaVersion }
            if let versionNode = versionEntry?.value {
                try checkSchemaVersion(versionNode)
            }
            try checkStructure(root)
            var reader = FieldReader()
            let config = try reader.readRoot(rootMapping)
            return ConfigLoadResult(config: config, warnings: reader.warnings)
        }
    }

    private static func checkSchemaVersion(_ node: Node) throws(ConfigLoadError) {
        if node.anchor != nil { throw ConfigLoadError(.anchorOrAlias, at: location(of: node)) }
        guard case let .scalar(scalar) = node else {
            throw ConfigLoadError(.typeMismatch(path: ConfigPath.schemaVersion, expected: .integer),
                                  at: location(of: node))
        }
        if hasExplicitTag(scalar) { return }  // reported by `checkStructure`
        guard let raw = try readIntegerText(node, path: ConfigPath.schemaVersion) else { return }
        let supported = 1...AppConfig.currentSchemaVersion
        if let version = Int(raw), version > supported.upperBound {
            throw ConfigLoadError(.newerSchemaVersion(version), at: location(of: node))
        }
        guard let version = Int(raw), supported.contains(version) else {
            throw ConfigLoadError(.outOfRange(path: ConfigPath.schemaVersion, raw: raw, range: supported),
                                  at: location(of: node))
        }
    }

    /// Rejects anchors/aliases, merge keys, complex keys and explicit tags anywhere in the document,
    /// in document order. Iterative (explicit stack) so deep nesting cannot overflow the thread stack.
    private static func checkStructure(_ root: Node) throws(ConfigLoadError) {
        enum Pending {
            case key(Node)
            case value(Node)
        }
        var stack: [Pending] = [.value(root)]
        while let pending = stack.popLast() {
            switch pending {
            case let .key(key):
                if key.anchor != nil {
                    throw ConfigLoadError(.anchorOrAlias, at: location(of: key))
                }
                guard case let .scalar(keyScalar) = key else {
                    throw ConfigLoadError(.complexKey, at: location(of: key))
                }
                if keyScalar.style == .plain && keyScalar.string == "<<" {
                    throw ConfigLoadError(.mergeKey, at: location(of: key))
                }
            case let .value(node):
                if node.anchor != nil {
                    throw ConfigLoadError(.anchorOrAlias, at: location(of: node))
                }
                switch node {
                case let .scalar(scalar):
                    if hasExplicitTag(scalar) {
                        throw ConfigLoadError(.explicitTag(scalar.tag.rawValue), at: location(of: node))
                    }
                case let .mapping(mapping):
                    if !mapping.tag.rawValue.isEmpty {
                        throw ConfigLoadError(.explicitTag(mapping.tag.rawValue), at: location(of: node))
                    }
                    for (key, value) in mapping.reversed() {
                        stack.append(.value(value))
                        stack.append(.key(key))
                    }
                case let .sequence(sequence):
                    if !sequence.tag.rawValue.isEmpty {
                        throw ConfigLoadError(.explicitTag(sequence.tag.rawValue), at: location(of: node))
                    }
                    stack.append(contentsOf: sequence.reversed().map(Pending.value))
                case .alias:
                    throw ConfigLoadError(.anchorOrAlias, at: location(of: node))
                }
            }
        }
    }

    // MARK: - Yams errors

    private static func mapYamsError(_ error: any Error, text: String) -> ConfigLoadError {
        guard let yamlError = error as? YamlError else {
            return ConfigLoadError(.syntax(problem: String(describing: error)))
        }
        switch yamlError {
        case let .reader(problem, offset, value, _):
            // Reader errors only carry a Character offset; translate it to line/column.
            let location = offset.flatMap { ConfigText.location(ofCharacterOffset: $0, in: text) }
                ?? ConfigText.firstUnreadableScalar(in: text)?.1
            if value >= 0, problem.contains("control characters") {
                return ConfigLoadError(.disallowedCharacter(scalar: UInt32(value)), at: location)
            }
            return ConfigLoadError(.syntax(problem: problem), at: location)
        case let .scanner(context, problem, mark, _), let .parser(context, problem, mark, _):
            if problem.contains("Maximum nesting level") {
                return ConfigLoadError(.nestingTooDeep, at: location(of: mark))
            }
            return ConfigLoadError(.syntax(problem: describe(problem, context: context)), at: location(of: mark))
        case let .composer(context, problem, mark, _):
            if problem.contains("undefined alias") {
                return ConfigLoadError(.anchorOrAlias, at: location(of: mark))
            }
            if problem.contains("another document") {
                return ConfigLoadError(.multipleDocuments, at: location(of: mark))
            }
            return ConfigLoadError(.syntax(problem: describe(problem, context: context)), at: location(of: mark))
        case let .duplicatedKeysInMapping(duplicates, context):
            return ConfigLoadError(.duplicateKey(duplicates.first ?? ""), at: location(of: context.mark))
        case .dataCouldNotBeDecoded:
            return ConfigLoadError(.invalidUTF8)
        default:
            return ConfigLoadError(.syntax(problem: yamlError.description))
        }
    }

    private static func describe(_ problem: String, context: YamlError.Context?) -> String {
        guard let context, !context.text.isEmpty else { return problem }
        return "\(context.text): \(problem)"
    }

    // MARK: - Node helpers

    static func location(of node: Node) -> LineColumn? {
        node.mark.map(location(of:))
    }

    static func location(of mark: Mark) -> LineColumn {
        LineColumn(line: mark.line, column: mark.column)
    }

    /// Plain `~`, `null`, `Null`, `NULL` or an empty value.
    static func isNull(_ node: Node) -> Bool {
        guard case let .scalar(scalar) = node, scalar.style == .plain, scalar.tag.rawValue.isEmpty else {
            return false
        }
        switch scalar.string {
        case "", "~", "null", "Null", "NULL": return true
        default: return false
        }
    }

    /// Plain scalars carry no tag unless one was written; quoted/block scalars get `!!str` implicitly.
    static func hasExplicitTag(_ scalar: Node.Scalar) -> Bool {
        switch scalar.style {
        case .plain, .any:
            return !scalar.tag.rawValue.isEmpty
        case .singleQuoted, .doubleQuoted, .literal, .folded:
            return scalar.tag.rawValue != Tag.Name.str.rawValue
        }
    }

    /// `^(0|[1-9][0-9]*)$`: the only integer spelling YAML 1.1 and 1.2 agree on.
    static func isCanonicalDecimal(_ raw: String) -> Bool {
        let bytes = Array(raw.utf8)
        guard let first = bytes.first, bytes.allSatisfy({ (0x30...0x39).contains($0) }) else { return false }
        return bytes.count == 1 || first != 0x30
    }

    /// Integer field. `nil` for a null value (use the default).
    static func readInteger(_ node: Node, path: String, range: ClosedRange<Int>) throws(ConfigLoadError) -> Int? {
        guard let raw = try readIntegerText(node, path: path) else { return nil }
        guard let value = Int(raw), range.contains(value) else {
            throw ConfigLoadError(.outOfRange(path: path, raw: raw, range: range), at: location(of: node))
        }
        return value
    }

    /// Text of an unquoted integer matching `^(0|[1-9][0-9]*)$`, or `nil` for null.
    /// Quoted numbers get a dedicated error; YAML 1.1 spellings (`0443`, `2:30`, `0x1BB`, `1_0`) are rejected.
    static func readIntegerText(_ node: Node, path: String) throws(ConfigLoadError) -> String? {
        guard case let .scalar(scalar) = node else {
            throw ConfigLoadError(.typeMismatch(path: path, expected: .integer), at: location(of: node))
        }
        let raw = scalar.string
        switch scalar.style {
        case .plain, .any:
            if isNull(node) { return nil }
            guard isCanonicalDecimal(raw) else {
                throw ConfigLoadError(.invalidInteger(path: path, raw: raw), at: location(of: node))
            }
            return raw
        case .singleQuoted, .doubleQuoted:
            if looksNumeric(raw) {
                throw ConfigLoadError(.quotedNumber(path: path, raw: raw), at: location(of: node))
            }
            throw ConfigLoadError(.typeMismatch(path: path, expected: .integer), at: location(of: node))
        case .literal, .folded:
            throw ConfigLoadError(.typeMismatch(path: path, expected: .integer), at: location(of: node))
        }
    }

    /// Boolean field: plain `true`/`True`/`TRUE`/`false`/`False`/`FALSE` (YAML 1.2 core schema).
    static func readBoolean(_ node: Node, path: String) throws(ConfigLoadError) -> Bool? {
        guard case let .scalar(scalar) = node else {
            throw ConfigLoadError(.typeMismatch(path: path, expected: .boolean), at: location(of: node))
        }
        let raw = scalar.string
        switch scalar.style {
        case .plain, .any:
            if isNull(node) { return nil }
            switch raw {
            case "true", "True", "TRUE": return true
            case "false", "False", "FALSE": return false
            default:
                throw ConfigLoadError(.invalidBoolean(path: path, raw: raw), at: location(of: node))
            }
        case .singleQuoted, .doubleQuoted:
            let word = raw.trimmingCharacters(in: .whitespaces).lowercased()
            if ["true", "false", "yes", "no", "on", "off", "y", "n"].contains(word) {
                throw ConfigLoadError(.quotedBoolean(path: path, raw: raw), at: location(of: node))
            }
            throw ConfigLoadError(.typeMismatch(path: path, expected: .boolean), at: location(of: node))
        case .literal, .folded:
            throw ConfigLoadError(.typeMismatch(path: path, expected: .boolean), at: location(of: node))
        }
    }

    /// String field: the scalar text as written (plain text is never re-typed, so `0123` stays `0123`).
    static func readString(_ node: Node, path: String) throws(ConfigLoadError) -> String? {
        guard case let .scalar(scalar) = node else {
            throw ConfigLoadError(.typeMismatch(path: path, expected: .string), at: location(of: node))
        }
        if isNull(node) { return nil }
        if let bad = ConfigText.firstDisallowedScalar(inField: scalar.string) {
            throw ConfigLoadError(.invalidCharacter(path: path, scalar: bad.value), at: location(of: node))
        }
        return scalar.string
    }

    /// Enumerated string field (quoted or plain), matched exactly against the raw values of `Choice`.
    /// `nil` for a null value (use the default).
    static func readChoice<Choice: RawRepresentable & CaseIterable>(
        _ node: Node, path: String, as _: Choice.Type
    ) throws(ConfigLoadError) -> Choice? where Choice.RawValue == String {
        guard let raw = try readString(node, path: path) else { return nil }
        guard let choice = Choice(rawValue: raw) else {
            throw ConfigLoadError(
                .invalidChoice(path: path, raw: raw, allowed: Choice.allCases.map(\.rawValue)),
                at: location(of: node))
        }
        return choice
    }

    /// A quoted value that the user probably meant as a number (`"443"`, `'0x1BB'`, `"-5"`).
    private static func looksNumeric(_ raw: String) -> Bool {
        let trimmed = raw.trimmingCharacters(in: .whitespaces)
        guard let first = trimmed.unicodeScalars.first else { return false }
        let startsLikeNumber = ("0"..."9").contains(first) || first == "+" || first == "-" || first == "."
        return startsLikeNumber && trimmed.unicodeScalars.contains { ("0"..."9").contains($0) }
    }
}

// MARK: - Field reader

/// Walks the validated root mapping and builds an ``AppConfig``, collecting warnings.
private struct FieldReader {
    var warnings: [ConfigWarning] = []

    mutating func readRoot(_ mapping: Node.Mapping) throws(ConfigLoadError) -> AppConfig {
        var config = AppConfig()
        for entry in try entries(of: mapping) {
            switch AppConfig.CodingKeys(rawValue: entry.key) {
            case .schemaVersion:
                break  // checked by `ConfigCodec.checkSchemaVersion`
            case .vpn:
                try readVPN(entry.value, into: &config.vpn)
            case .otp:
                try readOTP(entry.value, into: &config.otp)
            case .connection:
                try readConnection(entry.value, into: &config.connection)
            case nil:
                unknownKey(entry, parent: nil)
            }
        }
        let connection = config.connection
        if connection.proactiveReconnect.marginMinutes >= connection.sessionMaxMinutes {
            throw ConfigLoadError(
                .marginNotLessThanSession(margin: connection.proactiveReconnect.marginMinutes,
                                          session: connection.sessionMaxMinutes),
                at: marginLocation ?? sessionLocation)
        }
        let schedule = connection.schedule
        if schedule.start >= schedule.end {
            throw ConfigLoadError(.scheduleEndNotAfterStart(start: schedule.start, end: schedule.end),
                                  at: scheduleEndLocation ?? scheduleStartLocation)
        }
        return config
    }

    private var marginLocation: LineColumn?
    private var sessionLocation: LineColumn?
    private var scheduleStartLocation: LineColumn?
    private var scheduleEndLocation: LineColumn?

    private mutating func readVPN(_ node: Node, into vpn: inout AppConfig.VPN) throws(ConfigLoadError) {
        guard let mapping = try section(node, path: ConfigPath.vpn) else { return }
        for entry in try entries(of: mapping) {
            switch AppConfig.VPN.CodingKeys(rawValue: entry.key) {
            case .host:
                vpn.host = try ConfigCodec.readString(entry.value, path: ConfigPath.host) ?? vpn.host
            case .port:
                vpn.port = try ConfigCodec.readInteger(entry.value, path: ConfigPath.port, range: ConfigLimits.port)
                    ?? vpn.port
            case .username:
                vpn.username = try ConfigCodec.readString(entry.value, path: ConfigPath.username) ?? vpn.username
            case .realm:
                vpn.realm = try ConfigCodec.readString(entry.value, path: ConfigPath.realm) ?? vpn.realm
            case .trustedCerts:
                vpn.trustedCerts = try readTrustedCerts(entry.value)
            case nil:
                unknownKey(entry, parent: ConfigPath.vpn)
            }
        }
    }

    private mutating func readOTP(_ node: Node, into otp: inout AppConfig.OTP) throws(ConfigLoadError) {
        guard let mapping = try section(node, path: ConfigPath.otp) else { return }
        for entry in try entries(of: mapping) {
            switch AppConfig.OTP.CodingKeys(rawValue: entry.key) {
            case .gmailAddress:
                otp.gmailAddress = try ConfigCodec.readString(entry.value, path: ConfigPath.gmailAddress)
                    ?? otp.gmailAddress
            case .method:
                otp.method = try ConfigCodec.readChoice(
                    entry.value, path: ConfigPath.otpMethod, as: AppConfig.OTP.Method.self) ?? otp.method
            case .googleClientID:
                otp.googleClientID = try ConfigCodec.readString(entry.value, path: ConfigPath.googleClientID)
                    ?? otp.googleClientID
            case nil:
                unknownKey(entry, parent: ConfigPath.otp)
            }
        }
    }

    private mutating func readConnection(
        _ node: Node, into connection: inout AppConfig.Connection
    ) throws(ConfigLoadError) {
        guard let mapping = try section(node, path: ConfigPath.connection) else { return }
        for entry in try entries(of: mapping) {
            switch AppConfig.Connection.CodingKeys(rawValue: entry.key) {
            case .autoConnectOnLaunch:
                connection.autoConnectOnLaunch = try ConfigCodec.readBoolean(
                    entry.value, path: ConfigPath.autoConnectOnLaunch) ?? connection.autoConnectOnLaunch
            case .sessionMaxMinutes:
                if let value = try ConfigCodec.readInteger(
                    entry.value, path: ConfigPath.sessionMaxMinutes, range: ConfigLimits.sessionMaxMinutes) {
                    connection.sessionMaxMinutes = value
                    sessionLocation = ConfigCodec.location(of: entry.value)
                }
            case .proactiveReconnect:
                try readProactive(entry.value, into: &connection.proactiveReconnect)
            case .reconnectAfterSleep:
                try readAfterSleep(entry.value, into: &connection.reconnectAfterSleep)
            case .schedule:
                try readSchedule(entry.value, into: &connection.schedule)
            case nil:
                unknownKey(entry, parent: ConfigPath.connection)
            }
        }
    }

    private mutating func readProactive(
        _ node: Node, into value: inout AppConfig.Connection.ProactiveReconnect
    ) throws(ConfigLoadError) {
        guard let mapping = try section(node, path: ConfigPath.proactiveReconnect) else { return }
        for entry in try entries(of: mapping) {
            switch AppConfig.Connection.ProactiveReconnect.CodingKeys(rawValue: entry.key) {
            case .enabled:
                value.enabled = try ConfigCodec.readBoolean(entry.value, path: ConfigPath.proactiveEnabled)
                    ?? value.enabled
            case .marginMinutes:
                if let minutes = try ConfigCodec.readInteger(
                    entry.value, path: ConfigPath.marginMinutes, range: ConfigLimits.marginMinutes) {
                    value.marginMinutes = minutes
                    marginLocation = ConfigCodec.location(of: entry.value)
                }
            case nil:
                unknownKey(entry, parent: ConfigPath.proactiveReconnect)
            }
        }
    }

    private mutating func readAfterSleep(
        _ node: Node, into value: inout AppConfig.Connection.ReconnectAfterSleep
    ) throws(ConfigLoadError) {
        guard let mapping = try section(node, path: ConfigPath.reconnectAfterSleep) else { return }
        for entry in try entries(of: mapping) {
            switch AppConfig.Connection.ReconnectAfterSleep.CodingKeys(rawValue: entry.key) {
            case .enabled:
                value.enabled = try ConfigCodec.readBoolean(entry.value, path: ConfigPath.sleepEnabled)
                    ?? value.enabled
            case .minSleepMinutes:
                value.minSleepMinutes = try ConfigCodec.readInteger(
                    entry.value, path: ConfigPath.minSleepMinutes, range: ConfigLimits.minSleepMinutes)
                    ?? value.minSleepMinutes
            case nil:
                unknownKey(entry, parent: ConfigPath.reconnectAfterSleep)
            }
        }
    }

    private mutating func readSchedule(
        _ node: Node, into value: inout AppConfig.Connection.Schedule
    ) throws(ConfigLoadError) {
        guard let mapping = try section(node, path: ConfigPath.schedule) else { return }
        for entry in try entries(of: mapping) {
            switch AppConfig.Connection.Schedule.CodingKeys(rawValue: entry.key) {
            case .enabled:
                value.enabled = try ConfigCodec.readBoolean(entry.value, path: ConfigPath.scheduleEnabled)
                    ?? value.enabled
            case .days:
                value.days = try readDays(entry.value) ?? value.days
            case .start:
                if let time = try readTime(entry.value, path: ConfigPath.scheduleStart) {
                    value.start = time
                    scheduleStartLocation = ConfigCodec.location(of: entry.value)
                }
            case .end:
                if let time = try readTime(entry.value, path: ConfigPath.scheduleEnd) {
                    value.end = time
                    scheduleEndLocation = ConfigCodec.location(of: entry.value)
                }
            case .awayMinutes:
                value.awayMinutes = try ConfigCodec.readInteger(
                    entry.value, path: ConfigPath.awayMinutes, range: ConfigLimits.awayMinutes)
                    ?? value.awayMinutes
            case nil:
                unknownKey(entry, parent: ConfigPath.schedule)
            }
        }
    }

    /// `schedule.days`: a list of `mon` … `sun`; repeats are dropped and the order is normalized.
    /// `nil` for a null value (use the default).
    private func readDays(_ node: Node) throws(ConfigLoadError) -> [AppConfig.Connection.Schedule.Day]? {
        if ConfigCodec.isNull(node) { return nil }
        guard case let .sequence(sequence) = node else {
            throw ConfigLoadError(.typeMismatch(path: ConfigPath.scheduleDays, expected: .sequence),
                                  at: ConfigCodec.location(of: node))
        }
        var days: [AppConfig.Connection.Schedule.Day] = []
        for item in sequence {
            guard let day = try ConfigCodec.readChoice(
                item, path: ConfigPath.scheduleDays, as: AppConfig.Connection.Schedule.Day.self) else {
                throw ConfigLoadError(
                    .invalidChoice(path: ConfigPath.scheduleDays, raw: "",
                                   allowed: AppConfig.Connection.Schedule.Day.allCases.map(\.rawValue)),
                    at: ConfigCodec.location(of: item))
            }
            days.append(day)
        }
        return AppConfig.Connection.Schedule.normalizedDays(days)
    }

    /// `HH:mm` time, quoted or plain (a plain `18:00` is read as text, never as a YAML 1.1 base-60
    /// number). `nil` for a null value (use the default).
    private func readTime(_ node: Node, path: String) throws(ConfigLoadError) -> TimeOfDay? {
        guard let raw = try ConfigCodec.readString(node, path: path) else { return nil }
        guard let time = TimeOfDay(parsing: raw) else {
            throw ConfigLoadError(.invalidTime(path: path, raw: raw), at: ConfigCodec.location(of: node))
        }
        return time
    }

    /// `trusted_certs`: entries are normalized to lowercase; anything that is not a 64-hex digest,
    /// and repeats, are dropped with a warning instead of failing the whole file.
    private mutating func readTrustedCerts(_ node: Node) throws(ConfigLoadError) -> [String] {
        if ConfigCodec.isNull(node) { return [] }
        guard case let .sequence(sequence) = node else {
            throw ConfigLoadError(.typeMismatch(path: ConfigPath.trustedCerts, expected: .sequence),
                                  at: ConfigCodec.location(of: node))
        }
        var digests: [String] = []
        for (index, item) in sequence.enumerated() {
            let itemLocation = ConfigCodec.location(of: item)
            guard case let .scalar(scalar) = item, !ConfigCodec.isNull(item),
                  let digest = TrustedCertDigest.normalize(scalar.string)
            else {
                warnings.append(ConfigWarning(.invalidTrustedCert(index: index), at: itemLocation))
                continue
            }
            if digests.contains(digest) {
                warnings.append(ConfigWarning(.duplicateTrustedCert(index: index), at: itemLocation))
                continue
            }
            digests.append(digest)
        }
        return digests
    }

    // MARK: Helpers

    private struct Entry {
        var key: String
        var keyNode: Node
        var value: Node
    }

    /// `nil` for a null section (defaults); otherwise the mapping, or a type error.
    private func section(_ node: Node, path: String) throws(ConfigLoadError) -> Node.Mapping? {
        if ConfigCodec.isNull(node) { return nil }
        guard case let .mapping(mapping) = node else {
            throw ConfigLoadError(.typeMismatch(path: path, expected: .mapping), at: ConfigCodec.location(of: node))
        }
        return mapping
    }

    /// Scalar keys in document order. Yams already rejects most duplicates; this also catches keys
    /// Yams considers different (e.g. `1` vs `"1"`) but that spell the same name.
    private func entries(of mapping: Node.Mapping) throws(ConfigLoadError) -> [Entry] {
        var seen = Set<String>()
        var result: [Entry] = []
        for (keyNode, value) in mapping {
            guard case let .scalar(keyScalar) = keyNode else {
                throw ConfigLoadError(.complexKey, at: ConfigCodec.location(of: keyNode))
            }
            guard seen.insert(keyScalar.string).inserted else {
                throw ConfigLoadError(.duplicateKey(keyScalar.string), at: ConfigCodec.location(of: keyNode))
            }
            result.append(Entry(key: keyScalar.string, keyNode: keyNode, value: value))
        }
        return result
    }

    private mutating func unknownKey(_ entry: Entry, parent: String?) {
        let path = parent.map { "\($0).\(entry.key)" } ?? entry.key
        warnings.append(ConfigWarning(.unknownKey(path: path), at: ConfigCodec.location(of: entry.keyNode)))
    }
}
