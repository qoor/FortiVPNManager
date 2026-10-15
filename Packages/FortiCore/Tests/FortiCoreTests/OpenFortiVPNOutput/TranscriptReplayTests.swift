import FortiCore
import FortiShared
import Foundation
import Testing

/// Replays the synthetic transcripts in `Fixtures/openfortivpn` through `PromptDetector`,
/// `LineAssembler` and `OpenFortiVPNOutputParser`, wired the way the helper uses them, under many
/// chunkings and timings. The fixtures are reconstructed from openfortivpn 1.24.1 and Apple pppd
/// source strings; replace them with real Phase 0b captures when available.
@Suite struct TranscriptReplayTests {
    static let digest = "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11"

    enum Scenario: String, CaseIterable, Sendable {
        case successTokenInfo = "synthetic-success-tokeninfo"
        case successHTMLForm = "synthetic-success-htmlform"
        case certRejected = "synthetic-cert-rejected"
        case authFailed = "synthetic-auth-failed"
        case serverTerminated = "synthetic-server-terminated"
        case sigtermTeardown = "synthetic-sigterm-teardown"

        var expectedPrompts: [PromptDetector.Prompt] {
            switch self {
            case .successTokenInfo, .serverTerminated, .sigtermTeardown: [.passwordPrompt, .otpPrompt(.tokenInfo)]
            case .successHTMLForm: [.passwordPrompt, .otpPrompt(.htmlForm)]
            case .certRejected, .authFailed: [.passwordPrompt]
            }
        }

        private static let tokenInfoUp: [OpenFortiVPNSignal] = [
            .connectedToGateway,
            .authenticated,
            .addressesAssigned(TunnelAddresses(ip: "10.212.134.200", dnsServers: ["10.0.0.53", "10.0.0.54"], dnsSuffix: "corp.example.com")),
            .interfaceUp(name: "ppp0"),
            .routesTouched,
            .nameserversAdded,
            .tunnelUp,
        ]

        private static let teardown: [OpenFortiVPNSignal] = [
            .interfaceDown(name: "ppp0"),
            .routesRestored,
            .nameserversRemoved,
            .pppdExited(.exitStatus(16)),
        ]

        var expectedStdoutSignals: [OpenFortiVPNSignal] {
            switch self {
            case .successTokenInfo:
                Self.tokenInfoUp
            case .successHTMLForm:
                [
                    .connectedToGateway,
                    .authenticated,
                    .addressesAssigned(TunnelAddresses(ip: "10.212.134.201", dnsServers: ["10.0.0.53"])),
                    .interfaceUp(name: "ppp0"),
                    .routesTouched,
                    .nameserversAdded,
                    .tunnelUp,
                ]
            case .certRejected:
                [.certificateRejected(digest: TranscriptReplayTests.digest)]
            case .authFailed:
                [.connectedToGateway, .authenticationFailed]
            case .serverTerminated, .sigtermTeardown:
                Self.tokenInfoUp + Self.teardown
            }
        }

        var expectedStderrSignals: [OpenFortiVPNSignal] {
            switch self {
            case .serverTerminated, .sigtermTeardown: [.pppdNotice(.modemHangup), .pppdNotice(.connectionTerminated)]
            default: []
            }
        }

        var expectedDigests: [String] {
            self == .certRejected ? [TranscriptReplayTests.digest] : []
        }

        var expectedRoutesTouched: Bool {
            self != .certRejected && self != .authFailed
        }

        var expectedRoutesRestored: Bool {
            self == .serverTerminated || self == .sigtermTeardown
        }
    }

    /// Where read() boundaries fall, on top of the points where openfortivpn/pppd really pause.
    enum Chunking: Sendable, CustomTestStringConvertible {
        case whole
        case every(Int)
        case random(seed: UInt64)

        static let all: [Chunking] = [.whole] + [1, 2, 3, 5, 8, 13, 64, 4_096].map { .every($0) }
            + (1...12).map { .random(seed: $0) }

        func cuts(length: Int) -> Set<Int> {
            switch self {
            case .whole:
                return []
            case .every(let size):
                return Set(stride(from: size, to: length, by: size))
            case .random(let seed):
                var rng = SplitMix64(seed: seed)
                var cuts = Set<Int>()
                var position = 0
                while true {
                    position += 1 + Int(rng.next() % 24)
                    guard position < length else { return cuts }
                    cuts.insert(position)
                }
            }
        }

        var testDescription: String {
            switch self {
            case .whole: "whole"
            case .every(let size): "every \(size) bytes"
            case .random(let seed): "random #\(seed)"
            }
        }
    }

    /// Whether the helper answers a prompt (and pppd finishes a line) before the 250 ms idle flush.
    enum Timing: CaseIterable, Sendable {
        case quick
        case slow
    }

    struct Replay {
        var promptsPerPause: [[PromptDetector.Prompt]] = []
        var stdoutLines: [AssembledLine] = []
        var stderrLines: [AssembledLine] = []
        var stdoutSignals: [OpenFortiVPNSignal] = []
        var stderrSignals: [OpenFortiVPNSignal] = []
        var parser = OpenFortiVPNOutputParser()

        var prompts: [PromptDetector.Prompt] { promptsPerPause.flatMap { $0 } }

        mutating func record(_ lines: [AssembledLine], from source: OutputSource) {
            for line in lines {
                let signals = parser.parse(line.text, source: source)
                if source == .stdout {
                    stdoutLines.append(line)
                    stdoutSignals += signals
                } else {
                    stderrLines.append(line)
                    stderrSignals += signals
                }
            }
        }
    }

    static func fixture(_ scenario: Scenario, _ stream: String) throws -> [UInt8] {
        let url = try #require(
            Bundle.module.url(forResource: "\(scenario.rawValue).\(stream)", withExtension: "txt", subdirectory: "Fixtures/openfortivpn")
        )
        return try Array(Data(contentsOf: url))
    }

    static func lines(_ bytes: [UInt8]) -> [[UInt8]] {
        var lines = bytes.split(separator: 0x0A, omittingEmptySubsequences: false).map(Array.init)
        if lines.last?.isEmpty == true {
            lines.removeLast()
        }
        return lines
    }

    /// Offsets right after each prompt (before its LF): openfortivpn blocks on stdin there.
    static func promptPauses(in stdout: [UInt8]) -> Set<Int> {
        var pauses = Set<Int>()
        var offset = 0
        for line in lines(stdout) {
            if isPrompt(String(decoding: line, as: UTF8.self)) {
                pauses.insert(offset + line.count)
            }
            offset += line.count + 1
        }
        return pauses
    }

    /// Apple pppd writes timestamp, message and LF with three write() calls (utils.c:755-763).
    static func pppdWriteBoundaries(in stderr: [UInt8]) -> Set<Int> {
        var boundaries = Set<Int>()
        var offset = 0
        for line in lines(stderr) {
            let text = Substring(String(decoding: line, as: UTF8.self))
            if let range = text.firstRange(of: " : ") {
                boundaries.insert(offset + text.utf8.distance(from: text.startIndex, to: range.upperBound))
            }
            boundaries.insert(offset + line.count)
            offset += line.count + 1
        }
        return boundaries
    }

    /// One scenario's streams plus the points where openfortivpn/pppd pause between writes.
    struct Transcript {
        var stdout: [UInt8]
        var stderr: [UInt8]
        var promptPauses: Set<Int>
        var pppdWrites: Set<Int>

        init(_ scenario: Scenario) throws {
            stdout = try TranscriptReplayTests.fixture(scenario, "stdout")
            stderr = try TranscriptReplayTests.fixture(scenario, "stderr")
            promptPauses = TranscriptReplayTests.promptPauses(in: stdout)
            pppdWrites = TranscriptReplayTests.pppdWriteBoundaries(in: stderr)
        }
    }

    struct Streams: OptionSet {
        let rawValue: Int
        static let stdout = Streams(rawValue: 1)
        static let stderr = Streams(rawValue: 2)
        static let both: Streams = [.stdout, .stderr]
    }

    static func replay(
        _ transcript: Transcript,
        stdoutCuts: Set<Int>,
        stderrCuts: Set<Int>,
        timing: Timing,
        streams: Streams = .both
    ) -> Replay {
        var replay = Replay()
        var now = ContinuousClock.now
        let pauseLength: Duration = timing == .slow ? .milliseconds(300) : .milliseconds(5)
        if streams.contains(.stdout) {
            replayStdout(transcript, cuts: stdoutCuts, pauseLength: pauseLength, now: &now, into: &replay)
        }
        if streams.contains(.stderr) {
            replayStderr(transcript, cuts: stderrCuts, pauseLength: pauseLength, now: &now, into: &replay)
        }
        return replay
    }

    private static func replayStdout(
        _ transcript: Transcript,
        cuts: Set<Int>,
        pauseLength: Duration,
        now: inout ContinuousClock.Instant,
        into replay: inout Replay
    ) {
        var detector = PromptDetector()
        var stdoutAssembler = LineAssembler(source: .stdout)
        let stdout = transcript.stdout
        var promptsSincePause: [PromptDetector.Prompt] = []
        for range in chunks(length: stdout.count, cuts: cuts.union(transcript.promptPauses)) {
            now += .milliseconds(1)
            let chunk = stdout[range]
            promptsSincePause += detector.append(chunk)
            replay.record(stdoutAssembler.append(chunk, now: now), from: .stdout)
            if transcript.promptPauses.contains(range.upperBound) {
                // Blocked on stdin until the helper writes the secret.
                replay.promptsPerPause.append(promptsSincePause)
                promptsSincePause = []
                now += pauseLength
                replay.record(stdoutAssembler.flushIfIdle(now: now), from: .stdout)
            }
        }
        replay.promptsPerPause.append(promptsSincePause)
        replay.record(stdoutAssembler.finish(), from: .stdout)
    }

    private static func replayStderr(
        _ transcript: Transcript,
        cuts: Set<Int>,
        pauseLength: Duration,
        now: inout ContinuousClock.Instant,
        into replay: inout Replay
    ) {
        var stderrAssembler = LineAssembler(source: .stderr)
        let stderr = transcript.stderr
        for range in chunks(length: stderr.count, cuts: cuts.union(transcript.pppdWrites)) {
            now += .milliseconds(1)
            replay.record(stderrAssembler.append(stderr[range], now: now), from: .stderr)
            if transcript.pppdWrites.contains(range.upperBound) {
                now += pauseLength
                replay.record(stderrAssembler.flushIfIdle(now: now), from: .stderr)
            }
        }
        replay.record(stderrAssembler.finish(), from: .stderr)
    }

    static func chunks(length: Int, cuts: Set<Int>) -> [Range<Int>] {
        let points = [0] + cuts.filter { $0 > 0 && $0 < length }.sorted() + [length]
        return zip(points, points.dropFirst()).compactMap { $0 < $1 ? $0..<$1 : nil }
    }

    // MARK: Tests

    @Test(arguments: Scenario.allCases, Chunking.all)
    func replaysTranscript(scenario: Scenario, chunking: Chunking) throws {
        let transcript = try Transcript(scenario)
        let stdoutLineTexts = Self.lines(transcript.stdout).map { String(decoding: $0, as: UTF8.self) }
        let stderrLineTexts = Self.lines(transcript.stderr).map { String(decoding: $0, as: UTF8.self) }

        for timing in Timing.allCases {
            let replay = Self.replay(
                transcript,
                stdoutCuts: chunking.cuts(length: transcript.stdout.count),
                stderrCuts: chunking.cuts(length: transcript.stderr.count),
                timing: timing
            )

            // Each prompt is reported before openfortivpn needs its answer, exactly once.
            #expect(replay.promptsPerPause == scenario.expectedPrompts.map { [$0] } + [[]], "\(timing)")
            #expect(replay.stdoutSignals == scenario.expectedStdoutSignals, "\(timing)")
            #expect(replay.stderrSignals == scenario.expectedStderrSignals, "\(timing)")
            #expect(replay.parser.rejectedCertificateDigests == scenario.expectedDigests)
            #expect(replay.parser.routesTouched == scenario.expectedRoutesTouched)
            #expect(replay.parser.routesRestored == scenario.expectedRoutesRestored)

            // Log tab: the original lines, a slow prompt shown as a partial line and its LF swallowed.
            #expect(replay.stdoutLines.map(\.text) == stdoutLineTexts, "\(timing)")
            let partialStdout = replay.stdoutLines.filter(\.isPartial).map(\.text)
            #expect(partialStdout == (timing == .slow ? stdoutLineTexts.filter(Self.isPrompt) : []), "\(timing)")
            #expect(replay.stderrLines.map(\.text).joined() == stderrLineTexts.joined(), "\(timing)")
            if timing == .quick {
                #expect(replay.stderrLines == stderrLineTexts.map { AssembledLine(text: $0, isPartial: false) })
            }
        }
    }

    /// Every single split point of each stream. Offsets are sharded so the cases run in parallel.
    @Test(arguments: Scenario.allCases, 0..<4)
    func replaysTranscriptSplitAtEveryOffset(scenario: Scenario, shard: Int) throws {
        let transcript = try Self.Transcript(scenario)
        let expectedPrompts = scenario.expectedPrompts
        let expectedStdout = scenario.expectedStdoutSignals
        let expectedStderr = scenario.expectedStderrSignals
        var failingStdoutOffsets: [Int] = []
        var failingStderrOffsets: [Int] = []
        for offset in stride(from: shard, through: transcript.stdout.count, by: 4) {
            let replay = Self.replay(transcript, stdoutCuts: [offset], stderrCuts: [], timing: .quick, streams: .stdout)
            if replay.prompts != expectedPrompts || replay.stdoutSignals != expectedStdout {
                failingStdoutOffsets.append(offset)
            }
        }
        for offset in stride(from: shard, through: transcript.stderr.count, by: 4) {
            let replay = Self.replay(transcript, stdoutCuts: [], stderrCuts: [offset], timing: .quick, streams: .stderr)
            if replay.stderrSignals != expectedStderr {
                failingStderrOffsets.append(offset)
            }
        }
        #expect(failingStdoutOffsets.isEmpty)
        #expect(failingStderrOffsets.isEmpty)
    }

    @Test(arguments: Scenario.allCases)
    func fixturesUseOnlyKnownLineShapes(scenario: Scenario) throws {
        // Guards the synthetic fixtures: log.c prefixes are exactly 8 bytes, every other stdout
        // line is a prompt or /sbin/route output, and stderr lines carry pppd's "%c : " stamp.
        let prefixes = ["INFO:   ", "WARN:   ", "ERROR:  ", "DEBUG:  "]
        let stdout = try Self.fixture(scenario, "stdout")
        let stderr = try Self.fixture(scenario, "stderr")
        for line in Self.lines(stdout).map({ String(decoding: $0, as: UTF8.self) }) {
            let known = prefixes.contains { line.hasPrefix($0) } || Self.isPrompt(line)
                || line.hasPrefix("add ") || line.hasPrefix("delete ")
            #expect(known, "\(line)")
        }
        for line in Self.lines(stderr).map({ String(decoding: $0, as: UTF8.self) }) {
            #expect(OpenFortiVPNOutputParserTimestampProbe.hasTimestamp(line), "\(line)")
        }
    }

    static func isPrompt(_ line: String) -> Bool {
        line == PromptDetector.passwordPromptText || line == PromptDetector.tokenInfoPromptText
            || line.hasPrefix(PromptDetector.htmlFormPromptLead)
    }
}

/// Deterministic generator for chunk boundaries.
private struct SplitMix64: RandomNumberGenerator {
    private var state: UInt64

    init(seed: UInt64) {
        state = seed
    }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}

/// Checks the pppd timestamp shape without depending on parser internals.
private enum OpenFortiVPNOutputParserTimestampProbe {
    static func hasTimestamp(_ line: String) -> Bool {
        let pattern = #/^[A-Z][a-z]{2} [A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2} \d{4} : /#
        return line.firstMatch(of: pattern) != nil
    }
}
