import Foundation

/// IMAP mailbox-name encoding (RFC 3501 §5.1.3): printable ASCII as-is, `&` as `&-`, everything
/// else as `&<modified base64 of UTF-16BE>-` (`,` instead of `/`, no padding).
enum ModifiedUTF7 {
    /// Decodes a wire mailbox name; `nil` if it is not valid modified UTF-7.
    static func decode(_ wire: String) -> String? {
        let bytes = Array(wire.utf8)
        var result = ""
        var index = 0
        while index < bytes.count {
            let byte = bytes[index]
            guard (0x20...0x7E).contains(byte) else { return nil }
            guard byte == 0x26 else { // "&"
                result.unicodeScalars.append(Unicode.Scalar(byte))
                index += 1
                continue
            }
            guard let dash = bytes[(index + 1)...].firstIndex(of: 0x2D) else { return nil } // "-"
            if dash == index + 1 {
                result.append("&")
            } else {
                guard let decoded = decodeBase64Run(bytes[(index + 1)..<dash]) else { return nil }
                result.append(decoded)
            }
            index = dash + 1
        }
        return result
    }

    /// Encodes a mailbox name into its wire form.
    static func encode(_ name: String) -> String {
        var result = ""
        var pending: [UInt16] = []
        func flush() {
            guard !pending.isEmpty else { return }
            var data = Data(capacity: pending.count * 2)
            for unit in pending {
                data.append(UInt8(unit >> 8))
                data.append(UInt8(unit & 0xFF))
            }
            let base64 = data.base64EncodedString()
                .replacingOccurrences(of: "=", with: "")
                .replacingOccurrences(of: "/", with: ",")
            result += "&" + base64 + "-"
            pending.removeAll()
        }
        for scalar in name.unicodeScalars {
            if (0x20...0x7E).contains(scalar.value) {
                flush()
                result += scalar == "&" ? "&-" : String(scalar)
            } else {
                pending.append(contentsOf: String(scalar).utf16)
            }
        }
        flush()
        return result
    }

    private static func decodeBase64Run(_ run: ArraySlice<UInt8>) -> String? {
        let isModifiedBase64 = run.allSatisfy { byte in
            (0x41...0x5A).contains(byte) || (0x61...0x7A).contains(byte) || (0x30...0x39).contains(byte)
                || byte == 0x2B || byte == 0x2C // "+" ","
        }
        guard isModifiedBase64 else { return nil }
        var text = String(decoding: run, as: UTF8.self).replacingOccurrences(of: ",", with: "/")
        while text.utf8.count % 4 != 0 { text.append("=") }
        guard let data = Data(base64Encoded: text), !data.isEmpty, data.count.isMultiple(of: 2) else { return nil }
        var units: [UInt16] = []
        units.reserveCapacity(data.count / 2)
        var iterator = data.makeIterator()
        while let high = iterator.next(), let low = iterator.next() {
            units.append(UInt16(high) << 8 | UInt16(low))
        }
        var decoded = ""
        var unitIterator = units.makeIterator()
        var decoder = UTF16()
        loop: while true {
            switch decoder.decode(&unitIterator) {
            case .scalarValue(let scalar): decoded.unicodeScalars.append(scalar)
            case .emptyInput: break loop
            case .error: return nil
            }
        }
        return decoded
    }
}
