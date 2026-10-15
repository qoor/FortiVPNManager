import Foundation

/// Fatal framing errors. After one is thrown the parser stays failed; drop the connection.
public enum IMAPParseError: Error, Sendable, Equatable {
    /// A `{n}` literal announced more than `maxLiteralSize` bytes (or an unparsable length).
    case literalTooLarge(declared: UInt64)
    /// One response (including its literals) grew beyond `maxResponseSize` bytes.
    case responseTooLarge
}

/// Resumable, byte-oriented IMAP response parser.
///
/// Feed bytes exactly as they arrive from the socket, in any chunking; each call returns the
/// responses completed by that chunk. Literals (`{n}\r\n` + n bytes) may be split at any offset.
/// Malformed but well-framed responses become `.other(raw)` instead of failing.
public struct IMAPResponseParser: Sendable {
    public let maxLiteralSize: Int
    public let maxResponseSize: Int

    /// Bytes of the response being assembled (lines and literal payloads, CRLFs included).
    private var buffer: [UInt8] = []
    /// Start of the current line segment within `buffer` (just after the last literal payload).
    private var lineStart = 0
    /// Literal payload bytes still expected.
    private var literalRemaining = 0
    private var failure: IMAPParseError?

    public init(maxLiteralSize: Int = 1 << 20, maxResponseSize: Int = 4 << 20) {
        self.maxLiteralSize = maxLiteralSize
        self.maxResponseSize = maxResponseSize
    }

    /// Number of bytes buffered for a response that is not complete yet.
    public var bufferedByteCount: Int { buffer.count }

    /// Consumes `bytes` and returns every response completed by them, in order.
    public mutating func feed(_ bytes: some Sequence<UInt8>) throws(IMAPParseError) -> [IMAPResponse] {
        if let failure { throw failure }
        var responses: [IMAPResponse] = []
        for byte in bytes {
            guard buffer.count < maxResponseSize else { try fail(.responseTooLarge) }
            buffer.append(byte)
            if literalRemaining > 0 {
                literalRemaining -= 1
                if literalRemaining == 0 { lineStart = buffer.count }
                continue
            }
            guard byte == IMAPByte.lf else { continue }
            var contentEnd = buffer.count - 1
            if contentEnd > lineStart, buffer[contentEnd - 1] == IMAPByte.cr { contentEnd -= 1 }
            switch Self.literalLength(buffer, lineStart: lineStart, contentEnd: contentEnd) {
            case .none:
                responses.append(IMAPResponseDecoder.decode(Array(buffer[..<contentEnd])))
                buffer.removeAll(keepingCapacity: buffer.capacity <= 64 * 1024)
                lineStart = 0
            case .some(let declared):
                guard declared <= UInt64(maxLiteralSize) else { try fail(.literalTooLarge(declared: declared)) }
                guard buffer.count + Int(declared) <= maxResponseSize else { try fail(.responseTooLarge) }
                literalRemaining = Int(declared)
                if literalRemaining == 0 { lineStart = buffer.count }
            }
        }
        return responses
    }

    private mutating func fail(_ error: IMAPParseError) throws(IMAPParseError) -> Never {
        failure = error
        buffer.removeAll()
        throw error
    }

    /// If the line segment ends with `{digits}` (optionally `{digits+}`), returns the literal size.
    /// Continuation requests (`+ ...`) never announce literals.
    private static func literalLength(_ buffer: [UInt8], lineStart: Int, contentEnd: Int) -> UInt64? {
        guard contentEnd > lineStart, buffer[contentEnd - 1] == IMAPByte.closeBrace else { return nil }
        if lineStart == 0, buffer.first == IMAPByte.plus { return nil }
        var index = contentEnd - 2
        if index >= lineStart, buffer[index] == IMAPByte.plus { index -= 1 }
        let digitsEnd = index + 1
        while index >= lineStart, IMAPByte.isDigit(buffer[index]) { index -= 1 }
        guard index >= lineStart, buffer[index] == IMAPByte.openBrace, index + 1 < digitsEnd else { return nil }
        var value: UInt64 = 0
        for byte in buffer[(index + 1)..<digitsEnd] {
            let (product, overflow1) = value.multipliedReportingOverflow(by: 10)
            let (sum, overflow2) = product.addingReportingOverflow(UInt64(byte - IMAPByte.zero))
            // Saturate; the caller rejects it as too large.
            if overflow1 || overflow2 { return UInt64.max }
            value = sum
        }
        return value
    }
}

enum IMAPByte {
    static let cr: UInt8 = 0x0D
    static let lf: UInt8 = 0x0A
    static let space: UInt8 = 0x20
    static let tab: UInt8 = 0x09
    static let quote: UInt8 = 0x22
    static let backslash: UInt8 = 0x5C
    static let openParen: UInt8 = 0x28
    static let closeParen: UInt8 = 0x29
    static let openBracket: UInt8 = 0x5B
    static let closeBracket: UInt8 = 0x5D
    static let openBrace: UInt8 = 0x7B
    static let closeBrace: UInt8 = 0x7D
    static let plus: UInt8 = 0x2B
    static let star: UInt8 = 0x2A
    static let zero: UInt8 = 0x30
    static let tilde: UInt8 = 0x7E

    static func isDigit(_ byte: UInt8) -> Bool { (0x30...0x39).contains(byte) }
}
