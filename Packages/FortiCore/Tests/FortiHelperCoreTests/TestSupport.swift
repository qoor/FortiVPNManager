import CryptoKit
import Darwin
import Dispatch
import FortiHelperCore
import FortiShared
import Foundation
import Synchronization
import Testing

// MARK: - Fixtures

enum Fixture: String {
    case success
    case serverCut = "server-cut"
    case certRejected = "cert-rejected"
    case authFailed = "auth-failed"
    case hang
    case earlyExit = "early-exit"

    var path: String {
        guard let url = Bundle.module.url(forResource: rawValue, withExtension: "sh", subdirectory: "Fixtures") else {
            fatalError("missing fixture \(rawValue).sh")
        }
        return url.path
    }
}

let fixturePATH = ["PATH": ExecutableSpec.helperPATH]
let rejectedDigest = "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11"
/// Leading/trailing spaces and non-ASCII must reach openfortivpn byte for byte.
let testPassword = " pässwörd 🔑 "
let testOTP = "042137"

/// Generous limits so a broken test fails instead of hanging; tests that exercise a watchdog
/// shrink the relevant limit.
let testLimits = WatchdogLimits(
    spawnToGateway: .seconds(5),
    gatewayToPromptOrAuth: .seconds(5),
    otpPromptToSubmit: .seconds(5),
    otpSubmitToAuth: .seconds(5),
    authToTunnel: .seconds(5),
    terminate: .seconds(2)
)
let testTiming = SupervisorTiming(outputBatchInterval: .milliseconds(20), pipeDrainLimit: .seconds(1), lineIdleFlush: .milliseconds(250))

func sha256Hex(_ text: String) -> String {
    SHA256.hash(data: Data(text.utf8)).map { String(format: "%02x", $0) }.joined()
}

/// A temporary file a fixture writes `key=value` lines to.
struct RecordFile: Sendable {
    let path: String

    init() {
        path = FileManager.default.temporaryDirectory.appendingPathComponent("fvm-helpercore-\(UUID().uuidString).txt").path
    }

    var values: [String: String] {
        guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { return [:] }
        var values: [String: String] = [:]
        for line in text.split(separator: "\n") {
            guard let equals = line.firstIndex(of: "=") else { continue }
            values[String(line[..<equals])] = String(line[line.index(after: equals)...])
        }
        return values
    }

    func remove() {
        unlink(path)
    }
}

// MARK: - Recording callbacks

/// What the supervisor delivered, in delivery order.
enum Delivered: Sendable {
    case event(HelperEvent)
    case output([OutputLine])
}

final class Recorder: Sendable {
    private let entries = Mutex<[Delivered]>([])
    /// Called after an event is recorded (on the supervisor's queue).
    private let reaction = Mutex<(@Sendable (HelperEvent) -> Void)?>(nil)

    func react(_ body: @escaping @Sendable (HelperEvent) -> Void) {
        reaction.withLock { $0 = body }
    }

    func record(_ event: HelperEvent) {
        entries.withLock { $0.append(.event(event)) }
        reaction.withLock { $0 }?(event)
    }

    func record(_ lines: [OutputLine]) {
        entries.withLock { $0.append(.output(lines)) }
    }

    var delivered: [Delivered] {
        entries.withLock { $0 }
    }

    var events: [HelperEvent] {
        delivered.compactMap { if case .event(let event) = $0 { event } else { nil } }
    }

    var batches: [[OutputLine]] {
        delivered.compactMap { if case .output(let lines) = $0 { lines } else { nil } }
    }

    var lines: [OutputLine] {
        batches.flatMap { $0 }
    }

    var texts: [String] {
        lines.map(\.text)
    }

    var milestones: [Milestone] {
        events.compactMap { if case .milestone(let milestone, _) = $0.kind { milestone } else { nil } }
    }

    var exitFacts: ExitFacts? {
        events.lazy.compactMap { if case .exited(let facts) = $0.kind { facts } else { nil } }.first
    }

    func reached(_ milestone: Milestone) -> Bool {
        milestones.contains(milestone)
    }

    /// Polls until `condition` holds; false on timeout.
    func wait(timeout: Duration = .seconds(5), until condition: (Recorder) -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while !condition(self) {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return true
    }

    func waitForExit(timeout: Duration = .seconds(5)) async -> ExitFacts? {
        _ = await wait(timeout: timeout) { $0.exitFacts != nil }
        return exitFacts
    }
}

// MARK: - Harness

/// A supervisor wired to a `Recorder`, running a fixture script (or any command).
struct Harness: Sendable {
    let supervisor: TunnelSupervisor
    let recorder: Recorder
    let record: RecordFile

    init(
        _ fixture: Fixture,
        _ arguments: [String] = [],
        password: String = testPassword,
        limits: WatchdogLimits = testLimits,
        timing: SupervisorTiming = testTiming,
        clock: any SupervisorClock = SystemSupervisorClock(),
        firstOutputSeq: UInt64 = 1,
        routeRepair: (@Sendable () -> RouteRepairOutcome)? = nil
    ) throws {
        let record = RecordFile()
        try self.init(
            executable: ExecutableSpec(path: "/bin/sh", arguments: ["/bin/sh", fixture.path, record.path] + arguments, environment: fixturePATH),
            record: record,
            password: password,
            limits: limits,
            timing: timing,
            clock: clock,
            firstOutputSeq: firstOutputSeq,
            routeRepair: routeRepair
        )
    }

    /// Runs `/bin/sh -c script`; `$1` is the record file.
    init(script: String, limits: WatchdogLimits = testLimits, timing: SupervisorTiming = testTiming) throws {
        let record = RecordFile()
        try self.init(
            executable: ExecutableSpec(path: "/bin/sh", arguments: ["/bin/sh", "-c", script, "sh", record.path], environment: fixturePATH),
            record: record,
            limits: limits,
            timing: timing
        )
    }

    init(
        executable: ExecutableSpec,
        record: RecordFile = RecordFile(),
        password: String = testPassword,
        limits: WatchdogLimits = testLimits,
        timing: SupervisorTiming = testTiming,
        clock: any SupervisorClock = SystemSupervisorClock(),
        firstOutputSeq: UInt64 = 1,
        routeRepair: (@Sendable () -> RouteRepairOutcome)? = nil
    ) throws {
        let recorder = Recorder()
        self.recorder = recorder
        self.record = record
        supervisor = try TunnelSupervisor(
            attemptID: AttemptID(),
            executable: executable,
            password: password,
            limits: limits,
            timing: timing,
            clock: clock,
            firstOutputSeq: firstOutputSeq,
            routeRepair: routeRepair,
            onEvent: { recorder.record($0) },
            onOutput: { recorder.record($0) }
        )
    }

    /// Submits `code` as soon as the OTP prompt is reported, from inside the event callback.
    func submitOTPWhenPrompted(_ code: String = testOTP) {
        let supervisor = supervisor
        recorder.react { event in
            if case .milestone(.otpPrompted, _) = event.kind {
                #expect(throws: Never.self) { try supervisor.submitOTP(code) }
            }
        }
    }

    /// Starts the supervisor and waits for the run to finish.
    func run(timeout: Duration = .seconds(5)) async throws -> ExitFacts {
        try supervisor.start()
        return try #require(await recorder.waitForExit(timeout: timeout), "no .exited within \(timeout)")
    }
}

// MARK: - Output invariants

/// Checks DESIGN §5.2 output rules for a finished run: consecutive seqs from `firstSeq`, every
/// line tagged with the attempt, all output delivered before `.exited`, which is last, and
/// `lastOutputSeq` matching the last delivered line.
func expectOutputInvariants(_ harness: Harness, firstSeq: UInt64 = 1, sourceLocation: SourceLocation = #_sourceLocation) {
    let recorder = harness.recorder
    let lines = recorder.lines
    #expect(lines.map(\.seq) == Array(firstSeq..<(firstSeq + UInt64(lines.count))), sourceLocation: sourceLocation)
    #expect(lines.allSatisfy { $0.attemptID == harness.supervisor.attemptID }, sourceLocation: sourceLocation)
    #expect(recorder.events.allSatisfy { $0.attemptID == harness.supervisor.attemptID }, sourceLocation: sourceLocation)
    #expect(recorder.batches.allSatisfy { !$0.isEmpty }, sourceLocation: sourceLocation)
    guard case .event(let last)? = recorder.delivered.last, case .exited(let facts) = last.kind else {
        Issue.record("the last delivery is not .exited", sourceLocation: sourceLocation)
        return
    }
    #expect(recorder.events.filter { if case .exited = $0.kind { true } else { false } }.count == 1, sourceLocation: sourceLocation)
    #expect(facts.lastOutputSeq == lines.last?.seq, sourceLocation: sourceLocation)
    #expect(lines.last?.source == .helper, sourceLocation: sourceLocation)
    #expect(lines.last?.text.hasPrefix("‹exited: ") == true || lines.last?.text.hasPrefix("‹route") == true, sourceLocation: sourceLocation)
    #expect(!lines.contains { $0.text.contains(testPassword) || $0.text.contains(testOTP) }, sourceLocation: sourceLocation)
}

/// Lines delivered before the first event matching `predicate`.
func linesBefore(_ recorder: Recorder, event predicate: (HelperEvent.Kind) -> Bool) -> [String] {
    var texts: [String] = []
    for entry in recorder.delivered {
        switch entry {
        case .output(let lines): texts += lines.map(\.text)
        case .event(let event) where predicate(event.kind): return texts
        case .event: break
        }
    }
    return texts
}

// MARK: - Manual clock

/// A `SupervisorClock` that only moves when the test calls `advance(by:)`.
final class ManualClock: SupervisorClock {
    private struct Pending {
        var id: Int
        var deadline: ContinuousClock.Instant
        var queue: DispatchQueue
        var action: @Sendable () -> Void
    }

    private struct State {
        var offset = Duration.zero
        var nextID = 0
        var pending: [Pending] = []
    }

    private let origin = ContinuousClock.now
    private let originDate = Date(timeIntervalSince1970: 1_800_000_000)
    private let state = Mutex(State())

    var now: ContinuousClock.Instant {
        origin + state.withLock { $0.offset }
    }

    var date: Date {
        let (seconds, attoseconds) = state.withLock { $0.offset }.components
        return originDate.addingTimeInterval(TimeInterval(seconds) + TimeInterval(attoseconds) / 1e18)
    }

    func schedule(at deadline: ContinuousClock.Instant, on queue: DispatchQueue, _ action: @escaping @Sendable () -> Void) -> any SupervisorTimer {
        let id = state.withLock { state in
            state.nextID += 1
            state.pending.append(Pending(id: state.nextID, deadline: deadline, queue: queue, action: action))
            return state.nextID
        }
        return Timer(clock: self, id: id)
    }

    var pendingTimerCount: Int {
        state.withLock { $0.pending.count }
    }

    /// Moves time forward and runs every timer that became due, in deadline order.
    func advance(by duration: Duration) {
        let due = state.withLock { state in
            state.offset += duration
            let now = origin + state.offset
            let due = state.pending.filter { $0.deadline <= now }.sorted { $0.deadline < $1.deadline }
            state.pending.removeAll { $0.deadline <= now }
            return due
        }
        for timer in due {
            timer.queue.async(execute: timer.action)
        }
    }

    private func cancel(_ id: Int) {
        state.withLock { $0.pending.removeAll { $0.id == id } }
    }

    private struct Timer: SupervisorTimer {
        let clock: ManualClock
        let id: Int

        func cancel() {
            clock.cancel(id)
        }
    }
}

// MARK: - Signals

/// Runs `body` with SIGTERM ignored by this process, restoring the previous disposition afterwards.
func withSIGTERMIgnored<T>(_ body: () async throws -> T) async rethrows -> T {
    let previous = signal(SIGTERM, SIG_IGN)
    defer { signal(SIGTERM, previous) }
    return try await body()
}

/// Runs `body` on a dispatch worker thread with SIGTERM blocked in that thread's signal mask.
func onBackgroundQueueBlockingSIGTERM(_ body: @escaping @Sendable () -> Void) async {
    await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
        DispatchQueue.global(qos: .utility).async {
            var block = sigset_t()
            sigemptyset(&block)
            sigaddset(&block, SIGTERM)
            var previous = sigset_t()
            pthread_sigmask(SIG_BLOCK, &block, &previous)
            body()
            pthread_sigmask(SIG_SETMASK, &previous, nil)
            continuation.resume()
        }
    }
}
