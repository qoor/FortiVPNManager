import Foundation
import Testing

/// The Log tab's lines: what it keeps, and Copy All against the shared fixture
/// `contract/tests/fixtures/app-internal/log_copy_all.json`, whose `copyAll` lines are also the log
/// file's lines (each followed by a line break there; joined by line breaks in Copy All).
@MainActor
struct LogLinesTests {
    private struct CopyAllFixture: Decodable {
        var timeZone: String
        var message: AppInternal.FromRuntime
        var copyAll: [String]
    }

    @Test func copyAllWritesTheLogFileFormat() throws {
        let fixture: CopyAllFixture = try AppFixtures.decoded("log_copy_all.json")
        guard case let .logLines(lines) = fixture.message else {
            Issue.record("the fixture holds no log lines")
            return
        }
        let log = LogLines()
        log.append(lines)
        let timeZone = try #require(TimeZone(identifier: fixture.timeZone))
        #expect(log.copyAllText(timeZone: timeZone) == fixture.copyAll.joined(separator: "\n"))
    }

    @Test func keepsTheNewestLinesWithinTheLineLimit() {
        let log = LogLines()
        log.append((0..<(LogLines.maxLines + 3)).map { line("\($0)") })
        #expect(log.entries.count == LogLines.maxLines)
        #expect(log.entries.first?.line.text == "3")
        #expect(log.entries.last?.line.text == "\(LogLines.maxLines + 2)")
    }

    @Test func keepsTheNewestLinesWithinTheSizeLimit() {
        let log = LogLines()
        // Each line costs its UTF-8 length plus one for its line break: 1 KiB.
        let text = String(repeating: "x", count: 1023)
        log.append((0..<(2 * 1024 + 2)).map { _ in line(text) })
        #expect(log.entries.count == 2 * 1024)
    }

    @Test func clearEmptiesTheTabAndKeepsCounting() {
        let log = LogLines()
        log.append([line("a"), line("b")])
        let lastID = log.entries.last?.id
        log.clear()
        #expect(log.entries.isEmpty)
        #expect(log.clearCount == 1)
        #expect(log.copyAllText(timeZone: .gmt).isEmpty)
        log.append([line("c")])
        #expect(log.entries.map(\.line.text) == ["c"])
        #expect(log.entries.first.map(\.id) ?? 0 > lastID ?? 0)
    }

    @Test func entriesAfterAnIDAreTheNewerOnes() {
        let log = LogLines()
        log.append([line("a"), line("b"), line("c")])
        let first = log.entries[0].id
        #expect(log.entries(after: first).map(\.line.text) == ["b", "c"])
        #expect(log.entries(after: nil).count == 3)
    }

    @Test func universalTimeEndsWithZ() {
        let line = AppInternal.LogLine(time: Date(timeIntervalSince1970: 1_762_412_345.25), source: .note, text: "x")
        #expect(LogLineFormat.fileLine(line, timeZone: .gmt) == "2025-11-06T06:59:05.250Z [▸] x")
    }

    private func line(_ text: String) -> AppInternal.LogLine {
        AppInternal.LogLine(time: Date(timeIntervalSince1970: 1_762_412_345), source: .stdout, text: text)
    }
}

/// The JSON in contract/tests/fixtures/app-internal, decoded as on the stream.
enum AppFixtures {
    static func data(_ name: String) throws -> Data {
        let components = URL(filePath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        return try Data(contentsOf: components.appending(path: "contract/tests/fixtures/app-internal/\(name)"))
    }

    static func decoded<T: Decodable>(_ name: String) throws -> T {
        try MessageCoding.decoder().decode(T.self, from: data(name))
    }
}
