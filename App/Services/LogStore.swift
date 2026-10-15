import Foundation
import FortiShared
import Observation

/// One line of the Log tab. `id` is local, strictly increasing and never reused (helper `seq`
/// restarts with the helper process).
struct LogEntry: Identifiable, Sendable, Equatable {
    let id: UInt64
    let line: OutputLine
}

/// In-memory ring buffer behind the Log tab (DESIGN §5.6): the newest 5,000 lines / 2 MB of raw
/// openfortivpn, pppd and helper output, in arrival order (= helper `seq` order, one FIFO channel).
/// Every appended batch is also handed to `LogFileWriter`, in order.
@Observable
final class LogStore {
    static let maxLines = 5_000
    static let maxBytes = 2 * 1024 * 1024

    /// Oldest first. Text views append `entries(after:)` incrementally.
    private(set) var entries: [LogEntry] = []
    /// Bumped by `clearView()`; a text view that sees a new value starts over.
    private(set) var clearCount = 0

    @ObservationIgnored private var nextID: UInt64 = 1
    @ObservationIgnored private var byteCount = 0
    @ObservationIgnored private let fileQueue: AsyncStream<FileCommand>.Continuation
    @ObservationIgnored let fileURL: URL

    private nonisolated enum FileCommand: Sendable {
        case write([OutputLine])
        case flush(CheckedContinuation<Void, Never>)
    }

    init(writer: LogFileWriter = LogFileWriter()) {
        fileURL = writer.fileURL
        let (stream, continuation) = AsyncStream.makeStream(of: FileCommand.self, bufferingPolicy: .unbounded)
        fileQueue = continuation
        // One consumer keeps file writes in arrival order.
        Task.detached(priority: .utility) {
            for await command in stream {
                switch command {
                case let .write(lines): await writer.write(lines)
                case let .flush(done): done.resume()
                }
            }
        }
    }

    func append(_ lines: [OutputLine]) {
        guard !lines.isEmpty else { return }
        fileQueue.yield(.write(lines))

        var newEntries: [LogEntry] = []
        newEntries.reserveCapacity(lines.count)
        for line in lines {
            newEntries.append(LogEntry(id: nextID, line: line))
            nextID += 1
            byteCount += Self.cost(of: line)
        }
        entries.append(contentsOf: newEntries)
        trim()
    }

    /// Entries newer than `id` (all entries when `id` is `nil` or older than the buffer).
    func entries(after id: UInt64?) -> ArraySlice<LogEntry> {
        guard let id, let last = entries.lastIndex(where: { $0.id <= id }) else { return entries[...] }
        return entries[(last + 1)...]
    }

    /// "Clear": empties the view only; the log file keeps everything.
    func clearView() {
        entries.removeAll()
        byteCount = 0
        clearCount += 1
    }

    /// All lines in the file format, for "Copy All".
    var allText: String {
        entries.map { LogLineFormat.fileLine($0.line) }.joined(separator: "\n")
    }

    /// Waits until every appended line reached the log file (used before the app terminates).
    func flushToFile() async {
        await withCheckedContinuation { continuation in
            fileQueue.yield(.flush(continuation))
        }
    }

    private func trim() {
        var drop = 0
        while entries.count - drop > Self.maxLines || (byteCount > Self.maxBytes && drop < entries.count) {
            byteCount -= Self.cost(of: entries[drop].line)
            drop += 1
        }
        if drop > 0 { entries.removeFirst(drop) }
    }

    private static func cost(of line: OutputLine) -> Int {
        line.text.utf8.count + 1
    }
}

/// `<ISO 8601 local time> [out|err|▸] <text>`, the format of `openfortivpn.log`.
nonisolated enum LogLineFormat {
    /// e.g. `2026-10-15T14:03:22.123+09:00`.
    private static let timestamp = Date.ISO8601FormatStyle(
        timeZoneSeparator: .colon, includingFractionalSeconds: true, timeZone: .current
    )

    static func fileLine(_ line: OutputLine) -> String {
        "\(line.date.formatted(timestamp)) [\(tag(line.source))] \(line.text)"
    }

    static func tag(_ source: OutputSource) -> String {
        switch source {
        case .stdout: "out"
        case .stderr: "err"
        case .helper: "▸"
        }
    }
}
