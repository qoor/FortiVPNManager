import Foundation

/// Completion status of a command, or the status of an untagged `OK`/`NO`/`BAD`.
public enum IMAPStatus: String, Sendable, Hashable {
    case ok = "OK"
    case no = "NO"
    case bad = "BAD"
}

/// The bracketed response code of a status response (`[UIDVALIDITY 3]`, `[ALERT]`, ...).
public enum IMAPResponseCode: Sendable, Hashable {
    case alert
    case unavailable
    case limit
    case uidValidity(UInt32)
    case uidNext(UInt32)
    /// Any other code. `name` is uppercased; `argument` is the raw text after the name, if any.
    case other(name: String, argument: String?)
}

/// One complete server response.
public enum IMAPResponse: Sendable, Hashable {
    /// `tag OK|NO|BAD [code] text` — completion of the command sent with `tag`.
    case tagged(tag: String, status: IMAPStatus, code: IMAPResponseCode?, text: String)
    /// Untagged `* OK|NO|BAD [code] text` (e.g. `[UIDVALIDITY n]`, `[UIDNEXT n]` after SELECT).
    case untaggedStatus(IMAPStatus, code: IMAPResponseCode?, text: String)
    /// `* BYE` — the server is closing the connection.
    case bye(code: IMAPResponseCode?, text: String)
    /// `* LIST (attributes) "delimiter" name`.
    case list(IMAPListEntry)
    /// `* n EXISTS`.
    case exists(UInt32)
    /// `* SEARCH uid uid ...`; empty when nothing matched.
    case search([UInt32])
    /// `* n FETCH (...)`.
    case fetch(IMAPFetchResponse)
    /// `+ text` — command continuation request (e.g. the `+ idling` reply to IDLE).
    case continuation(String)
    /// Any other well-framed response, as raw text without the trailing CRLF.
    case other(String)
}

/// The RFC 6154 SPECIAL-USE mailbox attributes the OTP flow needs.
public enum IMAPSpecialUse: String, Sendable, Hashable {
    case all = "\\All"
    case junk = "\\Junk"
    case trash = "\\Trash"
}

/// One `* LIST` reply.
public struct IMAPListEntry: Sendable, Hashable {
    /// Attributes exactly as sent, e.g. `["\\HasNoChildren", "\\All"]`.
    public var attributes: [String]
    /// Mailbox name in wire form (modified UTF-7). Pass this unchanged to `SELECT`/`UID MOVE`.
    public var name: String

    public init(attributes: [String], name: String) {
        self.attributes = attributes
        self.name = name
    }

    /// Human-readable name (modified UTF-7 decoded), e.g. the Korean Gmail trash folder
    /// `[Gmail]/&1zTJwNG1-` becomes `[Gmail]/` followed by its Hangul name.
    public var displayName: String {
        ModifiedUTF7.decode(name) ?? name
    }

    /// Case-insensitive attribute check (`hasAttribute("\\Noselect")`).
    public func hasAttribute(_ attribute: String) -> Bool {
        attributes.contains { $0.caseInsensitiveCompare(attribute) == .orderedSame }
    }

    /// `false` for `\Noselect` / `\NonExistent` entries.
    public var isSelectable: Bool {
        !hasAttribute("\\Noselect") && !hasAttribute("\\NonExistent")
    }
}

/// The mailboxes the OTP flow needs, resolved from LIST replies by SPECIAL-USE attribute.
public struct IMAPSpecialMailboxes: Sendable, Hashable {
    public var all: IMAPListEntry?
    public var trash: IMAPListEntry?
    public var junk: IMAPListEntry?

    /// Picks the first selectable entry carrying each attribute.
    public init(listEntries: [IMAPListEntry]) {
        func first(_ use: IMAPSpecialUse) -> IMAPListEntry? {
            listEntries.first { $0.isSelectable && $0.hasAttribute(use.rawValue) }
        }
        all = first(.all)
        trash = first(.trash)
        junk = first(.junk)
    }

    /// Convenience over raw responses (non-LIST responses are ignored).
    public init(responses: [IMAPResponse]) {
        self.init(listEntries: responses.compactMap {
            if case .list(let entry) = $0 { entry } else { nil }
        })
    }
}

/// The parts of a `* n FETCH (...)` reply the OTP flow uses. Absent items are `nil`.
public struct IMAPFetchResponse: Sendable, Hashable {
    public var uid: UInt32?
    /// Parsed `INTERNALDATE` (server receive time).
    public var internalDate: Date?
    /// Contents of `BODY[HEADER...]` / `RFC822.HEADER` (the raw header block).
    public var headerBytes: Data?

    public init(uid: UInt32? = nil, internalDate: Date? = nil, headerBytes: Data? = nil) {
        self.uid = uid
        self.internalDate = internalDate
        self.headerBytes = headerBytes
    }
}

/// `UIDVALIDITY`/`UIDNEXT` of the `\All` mailbox, taken right after SELECT and before connecting.
public struct MailboxSnapshot: Sendable, Hashable {
    public var uidValidity: UInt32
    public var uidNext: UInt32

    public init(uidValidity: UInt32, uidNext: UInt32) {
        self.uidValidity = uidValidity
        self.uidNext = uidNext
    }
}

extension MailboxSnapshot {
    /// Builds the snapshot from the untagged replies to `SELECT`/`EXAMINE`
    /// (`* OK [UIDVALIDITY n]` and `* OK [UIDNEXT n]`). Returns `nil` if either is missing.
    public init?(selectResponses: [IMAPResponse]) {
        var validity: UInt32?
        var next: UInt32?
        for response in selectResponses {
            let code: IMAPResponseCode?
            switch response {
            case .untaggedStatus(_, let c, _), .tagged(_, _, let c, _): code = c
            default: code = nil
            }
            switch code {
            case .uidValidity(let v)?: validity = v
            case .uidNext(let n)?: next = n
            default: break
            }
        }
        guard let validity, let next else { return nil }
        self.init(uidValidity: validity, uidNext: next)
    }
}
