import Foundation

/// Text helpers shared by the codec, the template and diagnostics.
enum ConfigText {
    // MARK: Character classes

    /// Characters rejected in string fields: Unicode Cc (includes U+0085), U+2028, U+2029,
    /// U+FEFF and noncharacters.
    static func isDisallowedInField(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 0x2028, 0x2029, 0xFEFF:
            return true
        default:
            return scalar.properties.generalCategory == .control || scalar.properties.isNoncharacterCodePoint
        }
    }

    static func firstDisallowedScalar(inField value: String) -> Unicode.Scalar? {
        value.unicodeScalars.first(where: isDisallowedInField)
    }

    /// libyaml's reader whitelist (`yaml_parser_update_buffer`): anything else is a reader error.
    static func isReadableByLibYAML(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 0x09, 0x0A, 0x0D, 0x20...0x7E, 0x85, 0xA0...0xD7FF, 0xE000...0xFFFD, 0x10000...0x10FFFF:
            return true
        default:
            return false
        }
    }

    // MARK: Locations

    /// Tracks line/column the way libyaml counts them: CR, LF, CRLF, NEL, LS and PS end a line;
    /// every other scalar advances the column by one.
    struct LocationTracker {
        private(set) var line = 1
        private(set) var column = 1
        private var afterCR = false

        var location: LineColumn { LineColumn(line: line, column: column) }

        mutating func advance(over scalar: Unicode.Scalar) {
            switch scalar.value {
            case 0x0A:
                if afterCR {
                    afterCR = false  // second half of CRLF
                    return
                }
                newLine()
            case 0x0D:
                newLine()
                afterCR = true
                return
            case 0x85, 0x2028, 0x2029:
                newLine()
            default:
                column += 1
            }
            afterCR = false
        }

        private mutating func newLine() {
            line += 1
            column = 1
        }
    }

    /// Location of the `Character` at `characterOffset` (the unit Yams uses for reader errors).
    static func location(ofCharacterOffset characterOffset: Int, in text: String) -> LineColumn? {
        guard characterOffset >= 0,
              let target = text.index(text.startIndex, offsetBy: characterOffset, limitedBy: text.endIndex)
        else { return nil }
        var tracker = LocationTracker()
        for scalar in text.unicodeScalars[..<target] {
            tracker.advance(over: scalar)
        }
        return tracker.location
    }

    /// First scalar libyaml would refuse to read, with its location.
    static func firstUnreadableScalar(in text: String) -> (Unicode.Scalar, LineColumn)? {
        var tracker = LocationTracker()
        for scalar in text.unicodeScalars {
            if !isReadableByLibYAML(scalar) {
                return (scalar, tracker.location)
            }
            tracker.advance(over: scalar)
        }
        return nil
    }

    // MARK: UTF-8

    /// Decodes strict UTF-8 and strips one leading BOM. On failure returns the location of the
    /// first invalid byte sequence.
    enum UTF8Decoding {
        case text(String)
        case invalid(at: LineColumn)
    }

    static func decodeUTF8(_ data: Data) -> UTF8Decoding {
        if var text = String(validating: data, as: UTF8.self) {
            if text.unicodeScalars.first == "\u{FEFF}" {
                text.unicodeScalars.removeFirst()
            }
            return .text(text)
        }
        var iterator = data.makeIterator()
        var decoder = UTF8()
        var tracker = LocationTracker()
        var isFirst = true
        while true {
            switch decoder.decode(&iterator) {
            case let .scalarValue(scalar):
                if !(isFirst && scalar == "\u{FEFF}") {
                    tracker.advance(over: scalar)
                }
                isFirst = false
            case .emptyInput:
                // Unreachable: validation failed above, so an error must come first.
                return .invalid(at: tracker.location)
            case .error:
                return .invalid(at: tracker.location)
            }
        }
    }

    // MARK: Message formatting

    static func codePoint(_ value: UInt32) -> String {
        let hex = String(value, radix: 16, uppercase: true)
        return "U+" + String(repeating: "0", count: max(0, 4 - hex.count)) + hex
    }

    static func octal(_ mode: UInt16) -> String {
        let digits = String(mode & 0o7777, radix: 8)
        return String(repeating: "0", count: max(0, 4 - digits.count)) + digits
    }

    /// Last component of a dotted key path.
    static func leaf(_ path: String) -> String {
        path.split(separator: ".").last.map(String.init) ?? path
    }

    /// Short, single-line rendering of user text for messages.
    static func preview(_ raw: String, limit: Int = 40) -> String {
        var scalars = String.UnicodeScalarView()
        var count = 0
        for scalar in raw.unicodeScalars {
            if count == limit {
                scalars.append("…")
                break
            }
            scalars.append(isDisallowedInField(scalar) ? "\u{FFFD}" : scalar)
            count += 1
        }
        return String(scalars)
    }
}
