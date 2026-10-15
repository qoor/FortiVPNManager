import Foundation

/// Produces unique command tags (`A0001`, `A0002`, ...) for one IMAP session.
public struct IMAPTagGenerator: Sendable, Equatable {
    public let prefix: String
    private var counter: UInt64 = 0

    /// - Parameter prefix: One or more ASCII letters/digits. Defaults to `"A"`.
    public init(prefix: String = "A") {
        precondition(
            !prefix.isEmpty && prefix.utf8.allSatisfy(IMAPCommand.isTagByte),
            "IMAP tag prefix must be non-empty ASCII alphanumerics"
        )
        self.prefix = prefix
    }

    /// Returns the next tag. Tags are zero-padded to at least four digits.
    public mutating func next() -> String {
        counter += 1
        let digits = String(counter)
        return prefix + String(repeating: "0", count: max(0, 4 - digits.count)) + digits
    }
}

/// A non-empty set of message UIDs in IMAP `sequence-set` syntax (`1,3:5,9` or `42:*`).
public struct IMAPUIDSet: Sendable, Hashable, CustomStringConvertible {
    private enum Storage: Sendable, Hashable {
        case ranges([ClosedRange<UInt32>])
        case from(UInt32)
    }

    private let storage: Storage

    /// Builds a compact set from explicit UIDs. Returns `nil` when `uids` is empty or contains 0
    /// (UID 0 is not a valid message UID).
    public init?(_ uids: some Sequence<UInt32>) {
        let sorted = Set(uids).sorted()
        guard let first = sorted.first, first > 0 else { return nil }
        var ranges: [ClosedRange<UInt32>] = []
        var lower = first
        var upper = first
        for uid in sorted.dropFirst() {
            if uid == upper + 1 {
                upper = uid
            } else {
                ranges.append(lower...upper)
                lower = uid
                upper = uid
            }
        }
        ranges.append(lower...upper)
        storage = .ranges(ranges)
    }

    private init(storage: Storage) {
        self.storage = storage
    }

    /// The open-ended range `uid:*`. A `uid` of 0 is clamped to 1.
    ///
    /// Note that servers return the highest existing message for `n:*` even when its UID is below `n`
    /// (RFC 3501 §6.4.8), so callers must filter results client-side.
    public static func from(_ uid: UInt32) -> IMAPUIDSet {
        IMAPUIDSet(storage: .from(max(uid, 1)))
    }

    /// The set as sent on the wire.
    public var description: String {
        switch storage {
        case .from(let uid):
            return "\(uid):*"
        case .ranges(let ranges):
            return ranges.map { $0.lowerBound == $0.upperBound ? "\($0.lowerBound)" : "\($0.lowerBound):\($0.upperBound)" }
                .joined(separator: ",")
        }
    }
}

/// Reasons a command cannot be serialized.
public enum IMAPCommandError: Error, Sendable, Equatable {
    /// The tag is empty or contains characters outside `[A-Za-z0-9]`.
    case invalidTag
    /// The username is empty, or the username/password contains NUL (forbidden by SASL PLAIN).
    case invalidCredentials
    /// The mailbox name contains CR, LF, NUL or non-ASCII characters. Names returned by LIST are
    /// modified UTF-7 (7-bit) unless UTF8=ACCEPT was enabled, which this client never does.
    case invalidMailboxName
    /// A search string contains CR, LF, NUL or non-ASCII characters.
    case invalidSearchString
}

/// The commands the OTP session sends. `encoded(tag:)` yields the exact wire bytes including CRLF.
public enum IMAPCommand: Sendable, Equatable {
    /// `AUTHENTICATE PLAIN <base64("\0" + username + "\0" + password)>` using SASL-IR (RFC 4959).
    case authenticatePlain(username: String, password: String)
    /// `LIST "" "*"`. SPECIAL-USE attributes (`\All`, `\Trash`, `\Junk`) arrive in the LIST replies.
    case list
    /// `SELECT "<mailbox>"`. Pass `IMAPListEntry.name` unchanged (modified UTF-7 wire form).
    case select(mailbox: String)
    /// `EXAMINE "<mailbox>"` (read-only SELECT), e.g. for the one-off `\Junk` check.
    case examine(mailbox: String)
    /// `UID SEARCH UID <set>`, typically `IMAPUIDSet.from(snapshot.uidNext)`.
    case uidSearch(IMAPUIDSet)
    /// `UID SEARCH SUBJECT "<text>"`.
    case uidSearchSubject(String)
    /// `UID FETCH <set> (UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT DATE)])`.
    case uidFetchHeaders(IMAPUIDSet)
    /// `IDLE`. Leave idle by sending `IMAPCommand.idleDone`.
    case idle
    /// `UID MOVE <set> "<mailbox>"` (RFC 6851).
    case uidMove(IMAPUIDSet, mailbox: String)
    case noop
    case logout

    /// The untagged line that terminates `IDLE`.
    public static let idleDone = Data("DONE\r\n".utf8)

    /// Serializes the command with `tag`.
    public func encoded(tag: String) throws(IMAPCommandError) -> Data {
        guard !tag.isEmpty, tag.utf8.allSatisfy(Self.isTagByte) else { throw .invalidTag }
        return Data("\(tag) \(try body())\r\n".utf8)
    }

    /// The command line without tag and CRLF.
    private func body() throws(IMAPCommandError) -> String {
        switch self {
        case let .authenticatePlain(username, password):
            guard !username.isEmpty, !username.utf8.contains(0), !password.utf8.contains(0) else {
                throw .invalidCredentials
            }
            var message: [UInt8] = [0]
            message += username.utf8
            message.append(0)
            message += password.utf8
            return "AUTHENTICATE PLAIN " + Data(message).base64EncodedString()
        case .list:
            return #"LIST "" "*""#
        case .select(let mailbox):
            return "SELECT " + (try Self.quoted(mailbox, error: .invalidMailboxName))
        case .examine(let mailbox):
            return "EXAMINE " + (try Self.quoted(mailbox, error: .invalidMailboxName))
        case .uidSearch(let set):
            return "UID SEARCH UID \(set)"
        case .uidSearchSubject(let text):
            return "UID SEARCH SUBJECT " + (try Self.quoted(text, error: .invalidSearchString))
        case .uidFetchHeaders(let set):
            return "UID FETCH \(set) (UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT DATE)])"
        case .idle:
            return "IDLE"
        case let .uidMove(set, mailbox):
            return "UID MOVE \(set) " + (try Self.quoted(mailbox, error: .invalidMailboxName))
        case .noop:
            return "NOOP"
        case .logout:
            return "LOGOUT"
        }
    }

    /// Wraps `value` in an IMAP quoted string, escaping `"` and `\`.
    static func quoted(_ value: String, error: IMAPCommandError) throws(IMAPCommandError) -> String {
        var result = "\""
        for scalar in value.unicodeScalars {
            guard scalar.isASCII, scalar != "\r", scalar != "\n", scalar != "\0" else { throw error }
            if scalar == "\"" || scalar == "\\" { result.unicodeScalars.append("\\") }
            result.unicodeScalars.append(scalar)
        }
        result.append("\"")
        return result
    }

    static func isTagByte(_ byte: UInt8) -> Bool {
        (0x30...0x39).contains(byte) || (0x41...0x5A).contains(byte) || (0x61...0x7A).contains(byte)
    }
}

extension IMAPCommand: CustomStringConvertible, CustomDebugStringConvertible {
    /// A log-safe rendering; credentials are redacted.
    public var description: String {
        switch self {
        case .authenticatePlain(let username, _):
            return "AUTHENTICATE PLAIN <redacted for \(username)>"
        default:
            // Every other command is free of secrets.
            return (try? body()) ?? "<invalid>"
        }
    }

    public var debugDescription: String { "IMAPCommand(\(description))" }
}
