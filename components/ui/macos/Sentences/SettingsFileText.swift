import Foundation

// Sentences about the settings file (config.yaml) and IPv4 entries. Paths such as `vpn.port` come
// as written in the file; the few rules that name fixed keys spell them here.

extension AppInternal.ConfigFileError {
    var message: String {
        switch self {
        case let .invalidSettings(problem):
            String(localized: "The settings to save are invalid. \(problem.kind.message)")
        case let .unreadableFile(path, problem):
            String(localized: "Can't read the config file (\(path)). \(problem.displayMessage)")
        case let .notRegularFile(path):
            String(localized: "The config file path (\(path)) is not a regular file.")
        case let .danglingSymlink(path):
            String(localized: "The config file (\(path)) is a symbolic link to a target that doesn't exist.")
        case let .keptChanging(path):
            String(localized: "The config file (\(path)) kept changing while it was being saved, so it wasn't saved. Try again.")
        case let .io(operation, path, errno):
            String(localized: "Config file operation (\(operation)) failed: \(path): \(errorText(errno))")
        }
    }
}

extension AppInternal.ConfigProblem {
    /// The message, after the location when it is known: "Line 3, column 9: …".
    var displayMessage: String {
        guard let location else { return kind.message }
        return "\(location.description): \(kind.message)"
    }
}

extension AppInternal.ConfigWarning {
    var displayMessage: String {
        guard let location else { return kind.message }
        return "\(location.description): \(kind.message)"
    }
}

extension AppInternal.LineColumn {
    var description: String { String(localized: "Line \(line), column \(column)") }
}

extension AppInternal.ConfigProblemKind {
    var message: String {
        switch self {
        case let .fileTooLarge(byteCount, limit):
            return String(localized: "The config file is too large (\(byteCount) bytes). It must be \(limit / 1024) KiB (\(limit) bytes) or smaller.")
        case .invalidUtf8:
            return String(localized: "The config file is not valid UTF-8 text. Save it as UTF-8.")
        case let .disallowedCharacter(scalar):
            return String(localized: "The config file contains a character that is not allowed (\(codePoint(scalar))). Remove that character.")
        case let .syntax(problem):
            return String(localized: "YAML syntax error (\(problem)).")
        case .multipleDocuments:
            return String(localized: "The config file must contain only one YAML document. Remove the document separator (---).")
        case .nestingTooDeep:
            return String(localized: "The YAML structure is nested too deeply.")
        case let .duplicateKey(key):
            return String(localized: "The key '\(key)' appears more than once at the same level. Keep only one.")
        case .rootNotMapping:
            return String(localized: "The top level of the config file must be in 'key: value' form.")
        case .anchorOrAlias:
            return String(localized: "Anchors (&) and aliases (*) are not allowed. Write the values out directly.")
        case .mergeKey:
            return String(localized: "Merge keys (<<) are not allowed. Write the values out directly.")
        case let .explicitTag(tag):
            return String(localized: "YAML tags (\(tag)) are not allowed. Write just the value, without a tag.")
        case .complexKey:
            return String(localized: "Keys must be plain strings.")
        case let .typeMismatch(path, expected):
            return String(localized: "The value of \(path) has the wrong type. \(expected.requirement).")
        case let .invalidInteger(path, raw):
            return [
                String(localized: "\(path) is set to '\(preview(raw))', which is not an accepted number format."),
                String(localized: "Write it as a decimal number without leading zeros (e.g. \(leaf(path)): 443)."),
                String(localized: "Formats such as 0443 (octal), 2:30 (base 60), 0x1BB (hex) and 1_000 are not accepted."),
            ].joined(separator: " ")
        case let .invalidBoolean(path, raw):
            return String(localized: "\(path) is set to '\(preview(raw))', which is not allowed. Use true or false.")
        case let .quotedNumber(path, raw):
            return [
                String(localized: "\(path) has a quoted value \"\(preview(raw))\"."),
                String(localized: "Write numbers without quotes (e.g. \(leaf(path)): 443)."),
            ].joined(separator: " ")
        case let .quotedBoolean(path, raw):
            return [
                String(localized: "\(path) has a quoted value \"\(preview(raw))\"."),
                String(localized: "Write true/false without quotes (e.g. \(leaf(path)): true)."),
            ].joined(separator: " ")
        case let .invalidChoice(path, raw, allowed):
            let choices = allowed.joined(separator: ", ")
            return String(localized: "\(path) is set to '\(preview(raw))', which is not allowed. Use one of: \(choices).")
        case let .outOfRange(path, raw, minimum, maximum):
            if minimum == maximum {
                return String(localized: "\(path) is set to \(preview(raw)), which is not allowed. It must be \(minimum).")
            }
            return String(localized: "\(path) is set to \(preview(raw)), which is outside the allowed range (\(minimum)–\(maximum)).")
        case let .marginNotLessThanSession(margin, session):
            return String(localized: "\(SettingsKey.marginMinutes) (\(margin)) must be less than \(SettingsKey.sessionMaxMinutes) (\(session)).")
        case let .invalidTime(path, raw):
            return String(localized: "\(path) is set to '\(preview(raw))', which is not a time. Write it as \"HH:mm\" between 00:00 and 23:59 (e.g. \(leaf(path)): \"09:00\").")
        case let .scheduleEndNotAfterStart(start, end):
            return String(localized: "\(SettingsKey.scheduleEnd) (\(end.description)) must be later than \(SettingsKey.scheduleStart) (\(start.description)).")
        case let .invalidCharacter(path, scalar):
            return [
                String(localized: "\(path) contains a character that is not allowed (\(codePoint(scalar)))."),
                String(localized: "Control characters, line breaks, BOMs and similar characters are not allowed."),
            ].joined(separator: " ")
        case let .newerSchemaVersion(version):
            return [
                String(localized: "This config file was written by a newer version of the app (schema_version \(version))."),
                String(localized: "Update the app. Until then, this file is treated as read-only and is never overwritten."),
            ].joined(separator: " ")
        case let .invalidTrustedCert(index):
            return String(localized: "In \(SettingsKey.trustedCerts), entry \(index + 1) is not a SHA-256 digest (64 lowercase hex digits).")
        case let .invalidRouteEntry(path, index, raw, problem):
            return String(localized: "In \(path), entry \(index + 1): \(problem.message(for: raw))")
        case let .tooManyRouteEntries(path, count, limit):
            return String(localized: "\(path) has \(count) entries. It can have at most \(limit).")
        case let .duplicateRouteEntry(path, index):
            return String(localized: "In \(path), entry \(index + 1) repeats an earlier entry.")
        case let .overlappingRouteEntry(path, index, other):
            return String(localized: "In \(path), entry \(index + 1) overlaps \(other), which is already listed. Entries in one list must not overlap.")
        }
    }
}

extension AppInternal.ValueKind {
    fileprivate var requirement: String {
        switch self {
        case .mapping: String(localized: "It must be nested entries in 'key: value' form")
        case .sequence: String(localized: "It must be a list (e.g. [] or '- value' lines)")
        case .string: String(localized: "It must be a string")
        case .integer: String(localized: "It must be an unquoted integer")
        case .boolean: String(localized: "It must be an unquoted true or false")
        }
    }
}

extension AppInternal.ConfigWarningKind {
    var message: String {
        switch self {
        case let .unknownKey(path):
            String(localized: "Ignoring unknown key '\(path)'. It will be removed the next time you save in Settings.")
        case let .invalidTrustedCert(index):
            String(localized: "In \(SettingsKey.trustedCerts), entry \(index + 1) is not a SHA-256 digest (64 hex digits) and is ignored.")
        case let .duplicateTrustedCert(index):
            String(localized: "In \(SettingsKey.trustedCerts), entry \(index + 1) duplicates an earlier entry and is ignored.")
        case let .duplicateRouteEntry(path, index):
            String(localized: "In \(path), entry \(index + 1) duplicates an earlier entry and is ignored.")
        case let .multipleHardLinks(count):
            String(localized: "The config file has \(count) hard links. When the app saves, only this path is replaced with a new file; the other links keep the old contents.")
        case let .insecurePermissions(mode):
            String(localized: "The config file permissions are \(octal(mode)). They will be changed to 0600 on the next save.")
        case let .permissionsCorrected(previousMode):
            String(localized: "Changed the config file permissions from \(octal(previousMode)) to 0600.")
        }
    }
}

extension AppInternal.AddressProblem {
    /// The explanation for the entry `raw`, as typed or written in the file.
    func message(for raw: String) -> String {
        let text = preview(raw)
        return switch self {
        case .malformed:
            String(localized: "'\(text)' is not an IPv4 address or network. Write it like 10.30.161.16 or 10.30.170.0/24.")
        case let .prefixOutOfRange(length, minimum, maximum):
            String(localized: "'\(text)' has the prefix length /\(length). Use /\(minimum) to /\(maximum).")
        case .nonContiguousMask:
            String(localized: "'\(text)' has a subnet mask that is not contiguous.")
        case .unroutable:
            String(localized: "'\(text)' is a loopback, multicast or reserved address and can't be routed.")
        case let .hostBitsSet(network):
            String(localized: "'\(text)' has bits set after the prefix. Did you mean \(network)?")
        }
    }
}

extension AppInternal.TimeOfDay {
    /// `HH:mm`, as the settings file writes it.
    var description: String {
        let hh = hour < 10 ? "0\(hour)" : "\(hour)"
        let mm = minute < 10 ? "0\(minute)" : "\(minute)"
        return "\(hh):\(mm)"
    }
}

/// Keys of the settings file that rules name without carrying the path.
private enum SettingsKey {
    static let trustedCerts = "vpn.trusted_certs"
    static let sessionMaxMinutes = "connection.session_max_minutes"
    static let marginMinutes = "connection.proactive_reconnect.margin_minutes"
    static let scheduleStart = "connection.schedule.start"
    static let scheduleEnd = "connection.schedule.end"
}

/// The system's description of `errno`.
func errorText(_ errno: Int32) -> String {
    String(cString: strerror(errno))
}

/// "U+0007".
private func codePoint(_ value: UInt32) -> String {
    let hex = String(value, radix: 16, uppercase: true)
    return "U+" + String(repeating: "0", count: max(0, 4 - hex.count)) + hex
}

/// Permission bits as four octal digits: "0644".
private func octal(_ mode: UInt32) -> String {
    let digits = String(mode & 0o7777, radix: 8)
    return String(repeating: "0", count: max(0, 4 - digits.count)) + digits
}

/// The last component of a dotted key path.
private func leaf(_ path: String) -> String {
    path.split(separator: ".").last.map(String.init) ?? path
}

/// At most 40 scalars of user text on one line, for messages.
func preview(_ raw: String) -> String {
    let limit = 40
    var scalars = String.UnicodeScalarView()
    var count = 0
    for scalar in raw.unicodeScalars {
        if count == limit {
            scalars.append("…")
            break
        }
        scalars.append(isDisallowedInText(scalar) ? "\u{FFFD}" : scalar)
        count += 1
    }
    return String(scalars)
}

/// Control characters, line and paragraph separators, the BOM and noncharacters.
private func isDisallowedInText(_ scalar: Unicode.Scalar) -> Bool {
    switch scalar.value {
    case 0x2028, 0x2029, 0xFEFF:
        true
    default:
        scalar.properties.generalCategory == .control || scalar.properties.isNoncharacterCodePoint
    }
}
