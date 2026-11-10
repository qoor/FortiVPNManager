import Foundation
import Observation

/// One line of the Log pane. `id` is local, strictly increasing and never reused.
struct LogEntry: Identifiable, Equatable {
    let id: UInt64
    let line: AppInternal.LogLine
}

/// The lines the Log pane shows, oldest first: the newest ones the runtime sent, within a line and a
/// size limit. The log file keeps everything; that is the runtime's.
@MainActor
@Observable
final class LogLines {
    static let maxLines = 5_000
    static let maxBytes = 2 * 1024 * 1024

    private(set) var entries: [LogEntry] = []
    /// Bumped by `clear()`; a text view that sees a new value starts over.
    private(set) var clearCount = 0

    @ObservationIgnored private var nextID: UInt64 = 1
    @ObservationIgnored private var byteCount = 0

    func append(_ lines: [AppInternal.LogLine]) {
        guard !lines.isEmpty else { return }
        var added: [LogEntry] = []
        added.reserveCapacity(lines.count)
        for line in lines {
            added.append(LogEntry(id: nextID, line: line))
            nextID += 1
            byteCount += Self.cost(of: line)
        }
        entries.append(contentsOf: added)
        trim()
    }

    /// Entries newer than `id` (all entries when `id` is `nil` or older than the buffer).
    func entries(after id: UInt64?) -> ArraySlice<LogEntry> {
        guard let id, let last = entries.lastIndex(where: { $0.id <= id }) else { return entries[...] }
        return entries[(last + 1)...]
    }

    /// "Clear": empties the pane only.
    func clear() {
        entries.removeAll()
        byteCount = 0
        clearCount += 1
    }

    /// "Copy All": the lines in the log file's format, times in `timeZone`.
    func copyAllText(timeZone: TimeZone) -> String {
        entries.map { LogLineFormat.fileLine($0.line, timeZone: timeZone) }.joined(separator: "\n")
    }

    private func trim() {
        var drop = 0
        while entries.count - drop > Self.maxLines || (byteCount > Self.maxBytes && drop < entries.count) {
            byteCount -= Self.cost(of: entries[drop].line)
            drop += 1
        }
        if drop > 0 { entries.removeFirst(drop) }
    }

    private static func cost(of line: AppInternal.LogLine) -> Int {
        line.text.utf8.count + 1
    }
}

/// `<ISO 8601 local time with milliseconds> [out|err|▸] <text>`, the line format of the log file
/// (`openfortivpn.log`), such as `2026-10-15T14:03:22.123+09:00 [out] INFO:   Connected to gateway.`
enum LogLineFormat {
    static func fileLine(_ line: AppInternal.LogLine, timeZone: TimeZone) -> String {
        "\(timestamp(line.time, timeZone: timeZone)) [\(tag(line.source))] \(line.text)"
    }

    static func tag(_ source: AppInternal.LogSource) -> String {
        switch source {
        case .stdout: "out"
        case .stderr: "err"
        case .note: "▸"
        }
    }

    /// The milliseconds are written from the whole milliseconds the time was sent in: a `Date` keeps
    /// them as a binary fraction, and the formatter truncates it, so about half the times would come
    /// out one millisecond early and differ from the file.
    private static func timestamp(_ time: Date, timeZone: TimeZone) -> String {
        let millis = Int64((time.timeIntervalSince1970 * 1000).rounded())
        let (seconds, fraction) = millis.quotientAndRemainder(dividingBy: 1000)
        let whole = Date(timeIntervalSince1970: TimeInterval(seconds))
            .formatted(Date.ISO8601FormatStyle(timeZoneSeparator: .colon, timeZone: timeZone))
        // `yyyy-MM-ddTHH:mm:ss` is followed by the offset.
        let secondsEnd = whole.index(whole.startIndex, offsetBy: 19)
        let digits = String(fraction)
        return whole[..<secondsEnd] + "." + String(repeating: "0", count: 3 - digits.count) + digits + whole[secondsEnd...]
    }
}
