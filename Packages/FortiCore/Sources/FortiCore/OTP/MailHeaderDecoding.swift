import Foundation

/// Minimal RFC 5322 header-block reader (only what OTP matching needs).
enum MailHeader {
    /// The first field named `name` (case-insensitive): unfolded, trimmed, and RFC 2047-decoded.
    static func fieldValue(_ name: String, in block: Data) -> String? {
        rawFieldValue(name, in: block).map(RFC2047.decode)
    }

    /// The first field named `name`, unfolded and trimmed but not decoded.
    static func rawFieldValue(_ name: String, in block: Data) -> String? {
        // Header bytes should be ASCII, but raw 8-bit UTF-8 subjects occur; Latin-1 never fails.
        let text = String(data: block, encoding: .utf8)
            ?? String(String.UnicodeScalarView(block.map { Unicode.Scalar($0) }))
        let lines = text.split(omittingEmptySubsequences: false) { $0 == "\r\n" || $0 == "\n" || $0 == "\r" }

        var current: (name: Substring, value: String)?
        for line in lines {
            if line.isEmpty { break } // End of the header block.
            if line.first == " " || line.first == "\t" {
                // Folded continuation: unfolding removes only the line break (RFC 5322 §2.2.3).
                current?.value += line
                continue
            }
            if let field = current, field.name.caseInsensitiveCompare(name) == .orderedSame {
                return field.value.trimmingCharacters(in: .whitespaces)
            }
            if let colon = line.firstIndex(of: ":") {
                let fieldName = line[..<colon].trimmingCharacters(in: .whitespaces)
                current = (Substring(fieldName), String(line[line.index(after: colon)...]))
            } else {
                current = nil
            }
        }
        if let field = current, field.name.caseInsensitiveCompare(name) == .orderedSame {
            return field.value.trimmingCharacters(in: .whitespaces)
        }
        return nil
    }
}

/// RFC 2047 encoded-word decoding (`=?charset?B|Q?text?=`), applied only when `=?` is present.
///
/// Whitespace between adjacent encoded-words is dropped, and adjacent words in the same charset are
/// joined before conversion so multi-byte characters split across words still decode. Words that
/// are malformed or use an unknown charset are left verbatim.
enum RFC2047 {
    static func decode(_ text: String) -> String {
        guard text.contains("=?") else { return text }
        let bytes = Array(text.utf8)
        var output = ""
        var literalStart = 0
        var pending: Run?
        var index = 0
        while index + 1 < bytes.count {
            guard bytes[index] == 0x3D, bytes[index + 1] == 0x3F, let word = parseWord(bytes, at: index) else {
                index += 1
                continue
            }
            let gap = bytes[literalStart..<index]
            let gapIsWhitespace = gap.allSatisfy { $0 == 0x20 || $0 == 0x09 || $0 == 0x0D || $0 == 0x0A }
            if var run = pending, gapIsWhitespace {
                if run.encoding == word.encoding {
                    run.decoded += word.decoded
                    run.end = word.end
                    pending = run
                } else {
                    output += run.rendered(source: bytes)
                    pending = word
                }
            } else {
                if let run = pending { output += run.rendered(source: bytes) }
                output += String(decoding: gap, as: UTF8.self)
                pending = word
            }
            index = word.end
            literalStart = index
        }
        if let run = pending { output += run.rendered(source: bytes) }
        output += String(decoding: bytes[literalStart...], as: UTF8.self)
        return output
    }

    /// Maps an IANA charset name (`UTF-8`, `EUC-KR`, `ISO-8859-1`, `ks_c_5601-1987`, ...) to an encoding.
    static func encoding(forCharset charset: String) -> String.Encoding? {
        let cfEncoding = CFStringConvertIANACharSetNameToEncoding(charset as CFString)
        if cfEncoding != kCFStringEncodingInvalidId {
            return String.Encoding(rawValue: CFStringConvertEncodingToNSStringEncoding(cfEncoding))
        }
        // Common non-IANA spelling.
        return charset.lowercased() == "utf8" ? .utf8 : nil
    }

    /// One or more adjacent encoded-words sharing a charset.
    private struct Run {
        var start: Int
        var end: Int
        var encoding: String.Encoding
        var decoded: [UInt8]

        func rendered(source: [UInt8]) -> String {
            String(bytes: decoded, encoding: encoding) ?? String(decoding: source[start..<end], as: UTF8.self)
        }
    }

    private static func parseWord(_ bytes: [UInt8], at start: Int) -> Run? {
        var index = start + 2
        let charsetStart = index
        while index < bytes.count, bytes[index] != 0x3F {
            guard (0x21...0x7E).contains(bytes[index]) else { return nil }
            index += 1
        }
        guard index < bytes.count, index > charsetStart else { return nil }
        var charset = String(decoding: bytes[charsetStart..<index], as: UTF8.self)
        if let star = charset.firstIndex(of: "*") { charset = String(charset[..<star]) } // RFC 2231 language
        index += 1 // "?"
        guard index + 1 < bytes.count, bytes[index + 1] == 0x3F else { return nil }
        let method = bytes[index] | 0x20 // lowercase
        guard method == 0x62 || method == 0x71 else { return nil } // "b" / "q"
        index += 2
        let textStart = index
        while index + 1 < bytes.count, !(bytes[index] == 0x3F && bytes[index + 1] == 0x3D) {
            guard (0x21...0x7E).contains(bytes[index]), bytes[index] != 0x3F else { return nil }
            index += 1
        }
        guard index + 1 < bytes.count else { return nil }
        let encodedText = bytes[textStart..<index]
        let end = index + 2
        guard let encoding = encoding(forCharset: charset),
              let decoded = method == 0x62 ? decodeB(encodedText) : decodeQ(encodedText)
        else { return nil }
        return Run(start: start, end: end, encoding: encoding, decoded: decoded)
    }

    private static func decodeB(_ text: ArraySlice<UInt8>) -> [UInt8]? {
        var base64 = String(decoding: text, as: UTF8.self)
        guard base64.utf8.count % 4 != 1 else { return nil }
        while base64.utf8.count % 4 != 0 { base64.append("=") }
        return Data(base64Encoded: base64).map { Array($0) }
    }

    private static func decodeQ(_ text: ArraySlice<UInt8>) -> [UInt8]? {
        var result: [UInt8] = []
        var index = text.startIndex
        while index < text.endIndex {
            let byte = text[index]
            switch byte {
            case 0x5F: // "_"
                result.append(0x20)
                index += 1
            case 0x3D: // "="
                guard index + 2 < text.endIndex,
                      let high = hexValue(text[index + 1]), let low = hexValue(text[index + 2])
                else { return nil }
                result.append(high << 4 | low)
                index += 3
            default:
                result.append(byte)
                index += 1
            }
        }
        return result
    }

    private static func hexValue(_ byte: UInt8) -> UInt8? {
        switch byte {
        case 0x30...0x39: byte - 0x30
        case 0x41...0x46: byte - 0x41 + 10
        case 0x61...0x66: byte - 0x61 + 10
        default: nil
        }
    }
}
