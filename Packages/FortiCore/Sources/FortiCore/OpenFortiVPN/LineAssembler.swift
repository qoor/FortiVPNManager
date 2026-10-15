import FortiShared

/// One display line produced by `LineAssembler`.
public struct AssembledLine: Equatable, Sendable {
    /// Display-safe text: invalid UTF-8 is replaced with U+FFFD and control characters other than
    /// TAB are shown in caret notation (`^[`, `^@`, `^?`; C1 controls as `M-^X`).
    public var text: String
    /// True when the text was emitted before its terminating LF arrived: an idle flush (typically
    /// an input prompt), a hard split of an over-long line, or the end of the stream.
    public var isPartial: Bool

    public init(text: String, isPartial: Bool) {
        self.text = text
        self.isPartial = isPartial
    }
}

/// Splits one raw output stream (openfortivpn stdout, or pppd/route stderr) into display lines
/// for the Log tab and for `OpenFortiVPNOutputParser` (DESIGN §5.2, output handling).
///
/// Rules:
/// - Lines end at LF; one trailing CR is dropped.
/// - When no bytes arrive for `idleFlush`, the unfinished line is emitted with `isPartial = true`.
///   openfortivpn prints its prompts without a newline (userinput.c:344-345) and the `"\n"` it
///   prints after reading stdin (userinput.c:367) stays in the stdio buffer until the next log
///   line flushes it, so a lone LF that only terminates an already emitted partial line is
///   swallowed instead of producing an empty line. Text that continues after a partial flush is
///   emitted as its own line.
/// - Lines longer than `maxLineBytes` are hard-split on a UTF-8 scalar boundary.
///
/// Time is injected: pass the reading time to `append(_:now:)`, and call `flushIfIdle(now:)`
/// when `idleFlushDeadline` passes.
public struct LineAssembler: Sendable {
    public let source: OutputSource
    public let maxLineBytes: Int
    public let idleFlush: Duration

    /// Bytes of the current line that have not been emitted yet.
    private var pending: [UInt8] = []
    /// Part of the current line was already emitted (idle flush or hard split).
    private var currentLineHasEmittedPart = false
    /// Time of the last `append` that carried at least one byte.
    private var lastInputAt: ContinuousClock.Instant?

    public init(source: OutputSource, maxLineBytes: Int = 16_384, idleFlush: Duration = .milliseconds(250)) {
        precondition(maxLineBytes >= 4, "maxLineBytes must hold at least one UTF-8 scalar")
        self.source = source
        self.maxLineBytes = maxLineBytes
        self.idleFlush = idleFlush
    }

    /// When `flushIfIdle(now:)` will emit the pending unfinished line; `nil` when it has nothing
    /// to emit (no pending bytes, or only a held-back CR / partial UTF-8 sequence).
    public var idleFlushDeadline: ContinuousClock.Instant? {
        guard let lastInputAt, pending.count > Self.holdBackCount(pending) else { return nil }
        return lastInputAt + idleFlush
    }

    /// Feeds bytes read at `now`. A pending line whose idle deadline already passed is flushed
    /// first, so the result does not depend on how promptly the caller's timer fired.
    public mutating func append<Bytes: Sequence>(_ bytes: Bytes, now: ContinuousClock.Instant) -> [AssembledLine]
    where Bytes.Element == UInt8 {
        var lines = flushIfIdle(now: now)
        let input = Array(bytes)
        guard !input.isEmpty else { return lines }
        lastInputAt = now
        var start = 0
        while start < input.count {
            let end = input[start...].firstIndex(of: Self.lf) ?? input.count
            appendWithinLine(input[start..<end], into: &lines)
            if end < input.count, let line = terminateLine() {
                lines.append(line)
            }
            start = end + 1
        }
        return lines
    }

    /// Emits the unfinished line as partial if no bytes arrived for `idleFlush`.
    ///
    /// A trailing CR or an incomplete UTF-8 sequence is held back so that a following LF or the
    /// rest of the scalar still pairs up with it.
    public mutating func flushIfIdle(now: ContinuousClock.Instant) -> [AssembledLine] {
        guard !pending.isEmpty, let lastInputAt, now - lastInputAt >= idleFlush else { return [] }
        let emitCount = pending.count - Self.holdBackCount(pending)
        guard emitCount > 0 else { return [] }
        let text = Self.render(pending[..<emitCount])
        pending.removeFirst(emitCount)
        currentLineHasEmittedPart = true
        return [AssembledLine(text: text, isPartial: true)]
    }

    /// Emits whatever is left at end of stream (as a partial line) and resets the assembler.
    public mutating func finish() -> [AssembledLine] {
        defer {
            pending.removeAll()
            currentLineHasEmittedPart = false
            lastInputAt = nil
        }
        var content = pending[...]
        if content.last == Self.cr {
            content = content.dropLast()
        }
        guard !content.isEmpty else { return [] }
        return [AssembledLine(text: Self.render(content), isPartial: true)]
    }

    // MARK: - Private

    private static let lf: UInt8 = 0x0A
    private static let cr: UInt8 = 0x0D

    /// Appends bytes that contain no LF, hard-splitting whenever the line would exceed the limit.
    private mutating func appendWithinLine(_ segment: ArraySlice<UInt8>, into lines: inout [AssembledLine]) {
        var index = segment.startIndex
        while index < segment.endIndex {
            let room = maxLineBytes - pending.count
            if room > 0 {
                let end = min(segment.endIndex, index + room)
                pending.append(contentsOf: segment[index..<end])
                index = end
                continue
            }
            let byte = segment[index]
            // A CR right at the limit may still be the CR of a CRLF, so it may overhang by one.
            let mayOverhang = byte == Self.cr && pending.count == maxLineBytes
            if !mayOverhang {
                lines.append(hardSplit(before: byte))
            }
            pending.append(byte)
            index += 1
        }
    }

    private mutating func terminateLine() -> AssembledLine? {
        var content = pending[...]
        if content.last == Self.cr {
            content = content.dropLast()
        }
        let continuesEmittedLine = currentLineHasEmittedPart
        let line: AssembledLine? = if content.isEmpty && continuesEmittedLine {
            nil
        } else {
            AssembledLine(text: Self.render(content), isPartial: false)
        }
        pending.removeAll(keepingCapacity: true)
        currentLineHasEmittedPart = false
        return line
    }

    /// Emits the first `maxLineBytes` of `pending` (which holds `maxLineBytes` bytes, or one more
    /// when the last is a CR) because `next` makes the line too long. The cut backs off to the
    /// start of a multi-byte scalar that continues past it.
    private mutating func hardSplit(before next: UInt8) -> AssembledLine {
        var cut = maxLineBytes
        let firstAfterCut = cut < pending.count ? pending[cut] : next
        if Self.isContinuation(firstAfterCut) {
            let lowest = max(1, cut - 3)
            var index = cut - 1
            while index >= lowest, Self.isContinuation(pending[index]) {
                index -= 1
            }
            if index >= lowest, pending[index] >= 0xC0 {
                cut = index
            }
        }
        let text = Self.render(pending[..<cut])
        pending.removeFirst(cut)
        currentLineHasEmittedPart = true
        return AssembledLine(text: text, isPartial: true)
    }

    private static func isContinuation(_ byte: UInt8) -> Bool {
        byte & 0xC0 == 0x80
    }

    /// Number of trailing bytes an idle flush keeps back: a CR, or an incomplete UTF-8 sequence.
    private static func holdBackCount(_ bytes: [UInt8]) -> Int {
        guard let last = bytes.last else { return 0 }
        if last == cr {
            return 1
        }
        let lowest = max(0, bytes.count - 4)
        var index = bytes.count - 1
        while index > lowest, isContinuation(bytes[index]) {
            index -= 1
        }
        let lead = bytes[index]
        let expected = switch lead {
        case 0xC2...0xDF: 2
        case 0xE0...0xEF: 3
        case 0xF0...0xF4: 4
        default: 1
        }
        let available = bytes.count - index
        return available < expected ? available : 0
    }

    /// Decodes UTF-8 (U+FFFD for invalid input) and makes control characters visible.
    static func render<Bytes: Collection>(_ bytes: Bytes) -> String where Bytes.Element == UInt8 {
        let decoded = String(decoding: bytes, as: UTF8.self)
        guard decoded.unicodeScalars.contains(where: needsCaretNotation) else { return decoded }
        var rendered = String.UnicodeScalarView()
        for scalar in decoded.unicodeScalars {
            let value = scalar.value
            switch value {
            case 0x09:
                rendered.append(scalar)
            case 0x00..<0x20:
                rendered.append("^")
                rendered.append(Unicode.Scalar(UInt8(value + 0x40)))
            case 0x7F:
                rendered.append(contentsOf: "^?".unicodeScalars)
            case 0x80...0x9F:
                rendered.append(contentsOf: "M-^".unicodeScalars)
                rendered.append(Unicode.Scalar(UInt8(value - 0x80 + 0x40)))
            default:
                rendered.append(scalar)
            }
        }
        return String(rendered)
    }

    private static func needsCaretNotation(_ scalar: Unicode.Scalar) -> Bool {
        let value = scalar.value
        return (value < 0x20 && value != 0x09) || (0x7F...0x9F).contains(value)
    }
}
