import Foundation

/// Parses IMAP `date-time` (`INTERNALDATE`), e.g. `14-Oct-2026 10:19:00 +0900` or ` 4-Oct-2026 ...`.
///
/// Month names are the fixed English (POSIX) abbreviations regardless of the user's locale; the
/// conversion is pure arithmetic (proleptic Gregorian calendar), so no `DateFormatter` is involved.
enum IMAPInternalDate {
    private static let months = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"]

    static func parse(_ text: String) -> Date? {
        var scanner = Scanner(Array(text.utf8))
        scanner.skipSpaces()
        guard let day = scanner.number(digits: 1...2), scanner.consume("-"),
              let month = scanner.month(), scanner.consume("-"),
              let year = scanner.number(digits: 4...4), scanner.skipSpaces(),
              let hour = scanner.number(digits: 2...2), scanner.consume(":"),
              let minute = scanner.number(digits: 2...2), scanner.consume(":"),
              let second = scanner.number(digits: 2...2), scanner.skipSpaces(),
              let sign = scanner.sign(),
              let zone = scanner.number(digits: 4...4)
        else { return nil }
        scanner.skipSpaces()
        guard scanner.isAtEnd else { return nil }

        let zoneHours = zone / 100
        let zoneMinutes = zone % 100
        guard (1...daysInMonth(month, year: year)).contains(day),
              (0...23).contains(hour), (0...59).contains(minute), (0...60).contains(second),
              (0...23).contains(zoneHours), (0...59).contains(zoneMinutes)
        else { return nil }

        let offset = sign * (zoneHours * 3600 + zoneMinutes * 60)
        let seconds = daysFromCivil(year: year, month: month, day: day) * 86_400
            + hour * 3600 + minute * 60 + second - offset
        return Date(timeIntervalSince1970: TimeInterval(seconds))
    }

    private static func daysInMonth(_ month: Int, year: Int) -> Int {
        switch month {
        case 2: (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 ? 29 : 28
        case 4, 6, 9, 11: 30
        default: 31
        }
    }

    /// Days since 1970-01-01 (H. Hinnant's `days_from_civil`).
    private static func daysFromCivil(year: Int, month: Int, day: Int) -> Int {
        let y = month <= 2 ? year - 1 : year
        let era = (y >= 0 ? y : y - 399) / 400
        let yearOfEra = y - era * 400
        let dayOfYear = (153 * (month + (month > 2 ? -3 : 9)) + 2) / 5 + day - 1
        let dayOfEra = yearOfEra * 365 + yearOfEra / 4 - yearOfEra / 100 + dayOfYear
        return era * 146_097 + dayOfEra - 719_468
    }

    private struct Scanner {
        let bytes: [UInt8]
        var index = 0

        init(_ bytes: [UInt8]) {
            self.bytes = bytes
        }

        var isAtEnd: Bool { index == bytes.count }

        @discardableResult
        mutating func skipSpaces() -> Bool {
            let start = index
            while index < bytes.count, bytes[index] == 0x20 { index += 1 }
            return index > start
        }

        mutating func consume(_ character: Unicode.Scalar) -> Bool {
            guard index < bytes.count, bytes[index] == UInt8(character.value) else { return false }
            index += 1
            return true
        }

        mutating func number(digits: ClosedRange<Int>) -> Int? {
            let start = index
            var value = 0
            while index < bytes.count, index - start < digits.upperBound, IMAPByte.isDigit(bytes[index]) {
                value = value * 10 + Int(bytes[index] - IMAPByte.zero)
                index += 1
            }
            guard digits.contains(index - start) else {
                index = start
                return nil
            }
            return value
        }

        mutating func month() -> Int? {
            guard index + 3 <= bytes.count else { return nil }
            let name = String(decoding: bytes[index..<(index + 3)], as: UTF8.self).uppercased()
            guard let position = IMAPInternalDate.months.firstIndex(of: name) else { return nil }
            index += 3
            return position + 1
        }

        mutating func sign() -> Int? {
            if consume("+") { return 1 }
            if consume("-") { return -1 }
            return nil
        }
    }
}
