import Foundation

/// Generic IMAP data item: atom, string (quoted or literal), `NIL`, or parenthesized list.
indirect enum IMAPValue: Equatable {
    case atom(String)
    case string([UInt8])
    case nilValue
    case list([IMAPValue])

    var stringBytes: [UInt8]? {
        switch self {
        case .string(let bytes): bytes
        case .atom(let atom): Array(atom.utf8)
        case .nilValue, .list: nil
        }
    }
}

/// Cursor over the bytes of one complete response (literal payloads inline, final CRLF removed).
struct IMAPScanner {
    let bytes: [UInt8]
    var position = 0

    init(_ bytes: [UInt8]) {
        self.bytes = bytes
    }

    var isAtEnd: Bool { position >= bytes.count }

    func peek(offset: Int = 0) -> UInt8? {
        let index = position + offset
        return index < bytes.count ? bytes[index] : nil
    }

    mutating func consume(_ byte: UInt8) -> Bool {
        guard peek() == byte else { return false }
        position += 1
        return true
    }

    /// Consumes one or more spaces; `false` if there was none.
    @discardableResult
    mutating func consumeSpaces() -> Bool {
        let start = position
        while peek() == IMAPByte.space { position += 1 }
        return position > start
    }

    /// Remaining bytes as text (lossy UTF-8).
    mutating func restText() -> String {
        defer { position = bytes.count }
        return isAtEnd ? "" : String(decoding: bytes[position...], as: UTF8.self)
    }

    static func isAtomByte(_ byte: UInt8) -> Bool {
        switch byte {
        case 0...0x20, 0x7F, IMAPByte.openParen, IMAPByte.closeParen, IMAPByte.openBrace,
             IMAPByte.quote, IMAPByte.closeBracket:
            false
        default:
            true
        }
    }

    /// Reads a (lenient) atom. A `[` opens a section that runs to the matching `]` and may contain
    /// spaces and parentheses, so `BODY[HEADER.FIELDS (SUBJECT DATE)]<0>` is one atom.
    mutating func readAtom(allowCloseBracket: Bool = false) -> String? {
        let start = position
        while let byte = peek() {
            if byte == IMAPByte.openBracket {
                guard let end = matchingBracket(from: position) else { break }
                position = end + 1
            } else if Self.isAtomByte(byte) || (allowCloseBracket && byte == IMAPByte.closeBracket) {
                position += 1
            } else {
                break
            }
        }
        guard position > start else { return nil }
        return String(decoding: bytes[start..<position], as: UTF8.self)
    }

    /// Index of the `]` matching the `[` at `index`, not crossing a line break.
    func matchingBracket(from index: Int) -> Int? {
        var depth = 0
        var cursor = index
        while cursor < bytes.count {
            switch bytes[cursor] {
            case IMAPByte.openBracket: depth += 1
            case IMAPByte.closeBracket:
                depth -= 1
                if depth == 0 { return cursor }
            case IMAPByte.cr, IMAPByte.lf: return nil
            default: break
            }
            cursor += 1
        }
        return nil
    }

    /// Reads a 32-bit unsigned number; `nil` on no digits or overflow.
    mutating func readNumber() -> UInt32? {
        let start = position
        var value: UInt32 = 0
        while let byte = peek(), IMAPByte.isDigit(byte) {
            let (product, o1) = value.multipliedReportingOverflow(by: 10)
            let (sum, o2) = product.addingReportingOverflow(UInt32(byte - IMAPByte.zero))
            guard !o1, !o2 else {
                position = start
                return nil
            }
            value = sum
            position += 1
        }
        return position > start ? value : nil
    }

    /// Reads a quoted string (with `\"`/`\\` escapes) or a literal (`{n}` / `~{n}` CRLF + n bytes).
    mutating func readString() -> [UInt8]? {
        let start = position
        if consume(IMAPByte.quote) {
            var result: [UInt8] = []
            while let byte = peek() {
                position += 1
                switch byte {
                case IMAPByte.quote:
                    return result
                case IMAPByte.backslash:
                    guard let escaped = peek() else { break }
                    result.append(escaped)
                    position += 1
                case IMAPByte.cr, IMAPByte.lf:
                    position = start
                    return nil
                default:
                    result.append(byte)
                }
            }
            position = start
            return nil
        }
        if peek() == IMAPByte.tilde, peek(offset: 1) == IMAPByte.openBrace { position += 1 }
        guard consume(IMAPByte.openBrace) else {
            position = start
            return nil
        }
        guard let count = readNumber() else {
            position = start
            return nil
        }
        _ = consume(IMAPByte.plus)
        guard consume(IMAPByte.closeBrace) else {
            position = start
            return nil
        }
        _ = consume(IMAPByte.cr)
        guard consume(IMAPByte.lf), bytes.count - position >= Int(count) else {
            position = start
            return nil
        }
        let payload = Array(bytes[position..<(position + Int(count))])
        position += Int(count)
        return payload
    }

    /// Maximum parenthesis nesting accepted; deeper input is treated as malformed (no stack overflow).
    static let maxNesting = 32

    /// Reads any data item. `NIL` (any case) becomes `.nilValue`.
    mutating func readValue(depth: Int = 0) -> IMAPValue? {
        guard let byte = peek() else { return nil }
        switch byte {
        case IMAPByte.openParen:
            guard depth < Self.maxNesting else { return nil }
            let start = position
            position += 1
            var items: [IMAPValue] = []
            while true {
                consumeSpaces()
                if consume(IMAPByte.closeParen) { return .list(items) }
                guard let item = readValue(depth: depth + 1) else {
                    position = start
                    return nil
                }
                items.append(item)
            }
        case IMAPByte.quote, IMAPByte.openBrace:
            return readString().map(IMAPValue.string)
        case IMAPByte.tilde where peek(offset: 1) == IMAPByte.openBrace:
            return readString().map(IMAPValue.string)
        default:
            guard let atom = readAtom(allowCloseBracket: true) else { return nil }
            return atom.uppercased() == "NIL" ? .nilValue : .atom(atom)
        }
    }
}

/// Grammar for one complete response. Never fails: anything unrecognised becomes `.other(raw)`.
enum IMAPResponseDecoder {
    static func decode(_ bytes: [UInt8]) -> IMAPResponse {
        var scanner = IMAPScanner(bytes)
        return parse(&scanner) ?? .other(String(decoding: bytes, as: UTF8.self))
    }

    private static func parse(_ s: inout IMAPScanner) -> IMAPResponse? {
        if s.consume(IMAPByte.plus) {
            _ = s.consume(IMAPByte.space)
            return .continuation(s.restText())
        }
        if s.consume(IMAPByte.star) {
            guard s.consumeSpaces() else { return nil }
            return parseUntagged(&s)
        }
        guard let tag = s.readAtom(allowCloseBracket: true), !tag.utf8.contains(IMAPByte.plus),
              s.consumeSpaces(),
              let keyword = s.readAtom(), let status = IMAPStatus(rawValue: keyword.uppercased())
        else { return nil }
        let (code, text) = parseResponseText(&s)
        return .tagged(tag: tag, status: status, code: code, text: text)
    }

    private static func parseUntagged(_ s: inout IMAPScanner) -> IMAPResponse? {
        if let byte = s.peek(), IMAPByte.isDigit(byte) {
            guard let number = s.readNumber(), s.consumeSpaces(), let keyword = s.readAtom()?.uppercased() else {
                return nil
            }
            switch keyword {
            case "EXISTS": return s.isAtEnd ? .exists(number) : nil
            case "EXPUNGE": return s.isAtEnd ? .expunge(number) : nil
            case "FETCH":
                guard s.consumeSpaces() else { return nil }
                return parseFetch(sequenceNumber: number, &s)
            default: return nil
            }
        }
        guard let keyword = s.readAtom()?.uppercased() else { return nil }
        switch keyword {
        case "OK", "NO", "BAD":
            let (code, text) = parseResponseText(&s)
            return .untaggedStatus(IMAPStatus(rawValue: keyword)!, code: code, text: text)
        case "BYE":
            let (code, text) = parseResponseText(&s)
            return .bye(code: code, text: text)
        case "CAPABILITY":
            var names: [String] = []
            while s.consumeSpaces(), let name = s.readAtom() { names.append(name) }
            return s.isAtEnd ? .capability(names) : nil
        case "LIST", "XLIST":
            return parseList(&s)
        case "SEARCH":
            var uids: [UInt32] = []
            while true {
                s.consumeSpaces()
                // End, or a trailing CONDSTORE `(MODSEQ n)` which we do not need.
                if s.isAtEnd || s.peek() == IMAPByte.openParen { break }
                guard let uid = s.readNumber() else { return nil }
                uids.append(uid)
            }
            return .search(uids)
        default:
            return nil
        }
    }

    /// `[SP] ["[" code "]" SP] text`. Missing text is tolerated.
    private static func parseResponseText(_ s: inout IMAPScanner) -> (IMAPResponseCode?, String) {
        s.consumeSpaces()
        var code: IMAPResponseCode?
        if s.peek() == IMAPByte.openBracket, let end = s.matchingBracket(from: s.position) {
            let inner = String(decoding: s.bytes[(s.position + 1)..<end], as: UTF8.self)
            code = parseCode(inner)
            s.position = end + 1
            s.consumeSpaces()
        }
        return (code, s.restText())
    }

    static func parseCode(_ inner: String) -> IMAPResponseCode {
        let trimmed = inner.trimmingCharacters(in: .whitespaces)
        let parts = trimmed.split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
        let name = parts.first.map { $0.uppercased() } ?? ""
        let argument = parts.count > 1 ? String(parts[1]).trimmingCharacters(in: .whitespaces) : nil
        switch name {
        case "ALERT": return .alert
        case "AUTHENTICATIONFAILED": return .authenticationFailed
        case "AUTHORIZATIONFAILED": return .authorizationFailed
        case "UNAVAILABLE": return .unavailable
        case "LIMIT": return .limit
        case "READ-ONLY": return .readOnly
        case "READ-WRITE": return .readWrite
        case "TRYCREATE": return .tryCreate
        case "NONEXISTENT": return .nonExistent
        case "UIDVALIDITY":
            if let value = argument.flatMap({ UInt32($0) }) { return .uidValidity(value) }
        case "UIDNEXT":
            if let value = argument.flatMap({ UInt32($0) }) { return .uidNext(value) }
        case "CAPABILITY":
            return .capability(argument.map { $0.split(separator: " ").map(String.init) } ?? [])
        default:
            break
        }
        return .other(name: name, argument: argument)
    }

    /// `(attributes) SP delimiter SP mailbox [SP extended-data]`.
    private static func parseList(_ s: inout IMAPScanner) -> IMAPResponse? {
        guard s.consumeSpaces(), case .list(let attributeValues)? = s.readValue() else { return nil }
        var attributes: [String] = []
        for value in attributeValues {
            guard case .atom(let attribute) = value else { return nil }
            attributes.append(attribute)
        }
        guard s.consumeSpaces(), let delimiterValue = s.readValue() else { return nil }
        let delimiter: String?
        switch delimiterValue {
        case .nilValue: delimiter = nil
        case .string(let bytes): delimiter = String(decoding: bytes, as: UTF8.self)
        default: return nil
        }
        guard s.consumeSpaces(), let nameValue = s.readValue(), let nameBytes = nameValue.stringBytes else {
            return nil
        }
        return .list(IMAPListEntry(
            attributes: attributes,
            delimiter: delimiter,
            name: String(decoding: nameBytes, as: UTF8.self)
        ))
    }

    /// `(name value name value ...)`. Unknown items are skipped; known items must be well-typed.
    private static func parseFetch(sequenceNumber: UInt32, _ s: inout IMAPScanner) -> IMAPResponse? {
        guard case .list(let items)? = s.readValue(), items.count.isMultiple(of: 2) else { return nil }
        var fetch = IMAPFetchResponse(sequenceNumber: sequenceNumber)
        for index in stride(from: 0, to: items.count, by: 2) {
            guard case .atom(let rawName) = items[index] else { return nil }
            let name = rawName.uppercased()
            let value = items[index + 1]
            switch name {
            case "UID":
                guard case .atom(let digits) = value, let uid = UInt32(digits) else { return nil }
                fetch.uid = uid
            case "INTERNALDATE":
                guard case .string(let bytes) = value else { return nil }
                fetch.internalDate = IMAPInternalDate.parse(String(decoding: bytes, as: UTF8.self))
            case "FLAGS":
                guard case .list(let flagValues) = value else { return nil }
                var flags: [String] = []
                for flag in flagValues {
                    guard case .atom(let atom) = flag else { return nil }
                    flags.append(atom)
                }
                fetch.flags = flags
            case _ where name == "RFC822.HEADER" || (name.hasPrefix("BODY[") && name.dropFirst(5).hasPrefix("HEADER")):
                switch value {
                case .string(let bytes): fetch.headerBytes = Data(bytes)
                case .nilValue: fetch.headerBytes = nil
                default: return nil
                }
            default:
                break
            }
        }
        return .fetch(fetch)
    }
}
