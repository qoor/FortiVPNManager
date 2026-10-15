import Darwin
import Dispatch
import FortiCore
import FortiShared
import Foundation

/// Runs and watches one openfortivpn attempt for the root helper (DESIGN §5.2: input injection,
/// output handling, watchdog, RouteRepair).
///
/// - Spawns the executable with `ProcessRunner` and reports `.spawned`; a spawn failure is
///   reported as `.exited` with `furthestMilestone == nil` and `.spawnFailed(errno:)`.
/// - Reads stdout and stderr on one private serial queue. stdout bytes go straight to
///   `PromptDetector` (prompts have no newline) and, like stderr, through a `LineAssembler` to
///   `OpenFortiVPNOutputParser`. Each milestone is reported once, in order.
/// - Writes the password (plus LF) once, for the first password prompt before
///   `Connected to gateway.`, then zeroes its copy. `submitOTP` writes the code once while
///   `otpPrompted` and closes stdin.
/// - Arms one watchdog per phase (`WatchdogLimits`); an expired watchdog stops the run with
///   `.watchdog`. `stop` sends SIGTERM to openfortivpn only (its children such as `/sbin/route`
///   must finish restoring routes) and, after `WatchdogLimits.terminate`, SIGKILL to it and its
///   process group.
/// - On exit, kills leftovers in the child's process group, waits up to
///   `SupervisorTiming.pipeDrainLimit` for pipe EOF, flushes the line assemblers, runs
///   `routeRepair` if routes were changed but not restored, delivers all pending output and only
///   then reports `.exited(ExitFacts)`.
///
/// Output lines are numbered consecutively from `firstOutputSeq` and delivered in batches (every
/// `SupervisorTiming.outputBatchInterval`, and before every event). `onEvent`, `onOutput` and
/// `routeRepair` run on the private queue, never inside a call to a public method, and may call
/// back into the supervisor. `onEvent`/`onOutput` must not block; `routeRepair` runs once the
/// child is gone and may take a moment.
public final class TunnelSupervisor: @unchecked Sendable {
    public let attemptID: AttemptID

    private let executable: ExecutableSpec
    private let limits: WatchdogLimits
    private let timing: SupervisorTiming
    private let clock: any SupervisorClock
    private let routeRepair: (@Sendable () -> RouteRepairOutcome)?
    private let onEvent: @Sendable (HelperEvent) -> Void
    private let onOutput: @Sendable ([OutputLine]) -> Void
    private let queue: DispatchQueue
    private let queueKey = DispatchSpecificKey<Bool>()

    // Everything below is confined to `queue`.

    private enum State {
        case idle
        /// The child is alive.
        case running
        /// The child was reaped; waiting for pipe EOF.
        case exited
        case finished
    }

    private enum TimerKind: Hashable {
        case watchdog
        case kill
        case outputBatch
        case lineFlush
        case pipeDrain
    }

    private struct ArmedTimer {
        var generation: UInt64
        var deadline: ContinuousClock.Instant
        var timer: any SupervisorTimer
    }

    private enum Delivery {
        case output([OutputLine])
        case event(HelperEvent)
    }

    private var state = State.idle
    private var pid: pid_t = 0
    private var stdinFD: Int32 = -1
    private var readFDs: [OutputSource: Int32] = [:]
    private var readSources: [OutputSource: any DispatchSourceRead] = [:]
    private var exitSource: (any DispatchSourceProcess)?
    private var exitStatus: ProcessExitStatus?
    private var exitedAt: ContinuousClock.Instant?

    /// Password followed by LF; zeroed once written (or when the run ends).
    private var passwordLine: [UInt8]
    private var passwordHandled = false
    private var otpSubmitted = false

    private var promptDetector = PromptDetector()
    private var stdoutLines: LineAssembler
    private var stderrLines: LineAssembler
    private var parser = OpenFortiVPNOutputParser()

    private var furthest: Milestone?
    private var certRejectedAt: Milestone?
    private var authFailLineSeen = false
    private var tunnelModeRejected = false
    private var tunnelUpAt: ContinuousClock.Instant?
    private var stopInitiator = StopInitiator.none
    private var stopRequested = false
    private var watchdogPhase: WatchdogPhase?
    private var watchdogFired: WatchdogPhase?

    private var timers: [TimerKind: ArmedTimer] = [:]
    private var timerGeneration: UInt64 = 0

    private var nextSeq: UInt64
    private var batch: [OutputLine] = []
    private var outbox: [Delivery] = []
    private var deliveryScheduled = false

    /// - Parameters:
    ///   - password: Validated with `ConfigRenderer.validatePassword`. The supervisor keeps only a
    ///     byte copy that it zeroes after writing; the caller should drop its `String`.
    ///   - firstOutputSeq: `seq` of the first output line; continue a helper-wide sequence with
    ///     the previous run's `ExitFacts.lastOutputSeq + 1`.
    ///   - routeRepair: Called on exit when openfortivpn changed routes but did not finish
    ///     restoring them; `.repaired` sets `ExitFacts.routeRepairPerformed`.
    /// - Throws: `.invalidRequest` if the password or `executable.arguments` is unusable.
    public init(
        attemptID: AttemptID,
        executable: ExecutableSpec,
        password: String,
        limits: WatchdogLimits = WatchdogLimits(),
        timing: SupervisorTiming = SupervisorTiming(),
        clock: any SupervisorClock = SystemSupervisorClock(),
        firstOutputSeq: UInt64 = 1,
        routeRepair: (@Sendable () -> RouteRepairOutcome)? = nil,
        onEvent: @escaping @Sendable (HelperEvent) -> Void,
        onOutput: @escaping @Sendable ([OutputLine]) -> Void
    ) throws(HelperError) {
        do {
            try ConfigRenderer.validatePassword(password)
        } catch {
            throw HelperError(.invalidRequest, error.description)
        }
        guard !executable.arguments.isEmpty else {
            throw HelperError(.invalidRequest, "missing argv[0]")
        }
        self.attemptID = attemptID
        self.executable = executable
        self.limits = limits
        self.timing = timing
        self.clock = clock
        self.routeRepair = routeRepair
        self.onEvent = onEvent
        self.onOutput = onOutput
        self.nextSeq = firstOutputSeq
        var line: [UInt8] = []
        // Reserve room for the LF so appending it cannot leave an unzeroed copy behind.
        line.reserveCapacity(password.utf8.count + 1)
        line.append(contentsOf: password.utf8)
        line.append(0x0A)
        self.passwordLine = line
        self.stdoutLines = LineAssembler(source: .stdout, idleFlush: timing.lineIdleFlush)
        self.stderrLines = LineAssembler(source: .stderr, idleFlush: timing.lineIdleFlush)
        self.queue = DispatchQueue(label: "com.qoor.FortiVPNManager.TunnelSupervisor.\(attemptID.uuidString)")
        queue.setSpecific(key: queueKey, value: true)
    }

    deinit {
        Self.zero(&passwordLine)
    }

    // MARK: - Public API

    /// Whether `submitOTP` accepts `code`: 6 to 8 ASCII digits.
    public static func isValidOTP(_ code: String) -> Bool {
        (6...8).contains(code.utf8.count) && code.utf8.allSatisfy { (UInt8(ascii: "0")...UInt8(ascii: "9")).contains($0) }
    }

    /// Spawns the child. Events follow asynchronously: `.spawned`, or `.exited` if the spawn failed.
    /// - Throws: `.wrongPhase` if called more than once.
    public func start() throws(HelperError) {
        let error: HelperError? = perform {
            guard state == .idle else {
                return HelperError(.wrongPhase, "attempt already started")
            }
            launch()
            return nil
        }
        if let error { throw error }
    }

    /// Writes `code` and LF to openfortivpn's stdin and closes it.
    /// - Throws: `.invalidRequest` unless `code` is 6-8 ASCII digits; `.wrongPhase` unless
    ///   openfortivpn is waiting for the OTP (furthest milestone `otpPrompted`, not stopping)
    ///   and no OTP was submitted yet.
    public func submitOTP(_ code: String) throws(HelperError) {
        guard Self.isValidOTP(code) else {
            throw HelperError(.invalidRequest, "OTP must be 6 to 8 ASCII digits")
        }
        let error: HelperError? = perform { writeOTP(code) }
        if let error { throw error }
    }

    /// Asks the child to exit: SIGTERM now, SIGKILL (to it and its process group) after
    /// `WatchdogLimits.terminate`. Records `.helper(reason)` as the stop initiator. Does nothing
    /// if the child is not running or a stop is already in progress (the first reason wins).
    /// Completion is reported by `.exited`.
    public func stop(reason: StopReason) {
        perform { requestStop(reason) }
    }

    /// pid of the child while it is running (not yet reaped).
    public var processID: pid_t? {
        perform { state == .running ? pid : nil }
    }

    /// True once `.exited` has been queued for delivery.
    public var isFinished: Bool {
        perform { state == .finished }
    }

    /// The furthest milestone reached so far (`nil` before the spawn).
    public var furthestMilestone: Milestone? {
        perform { furthest }
    }

    // MARK: - Queue confinement

    /// Runs `body` on the queue (inline when already on it), then schedules delivery of whatever
    /// it queued. Callbacks therefore never run inside a public method call.
    private func perform<T>(_ body: () -> T) -> T {
        if DispatchQueue.getSpecific(key: queueKey) == true {
            let result = body()
            scheduleDelivery()
            return result
        }
        return queue.sync {
            let result = body()
            scheduleDelivery()
            return result
        }
    }

    private func scheduleDelivery() {
        guard !outbox.isEmpty, !deliveryScheduled else { return }
        deliveryScheduled = true
        queue.async { [self] in
            deliveryScheduled = false
            let items = outbox
            outbox.removeAll()
            for item in items {
                switch item {
                case .output(let lines): onOutput(lines)
                case .event(let event): onEvent(event)
                }
            }
        }
    }

    // MARK: - Spawn

    private func launch() {
        state = .running
        let process: SpawnedProcess
        do {
            process = try ProcessRunner.spawn(
                executable: executable.path,
                arguments: executable.arguments,
                environment: executable.environment
            )
        } catch {
            state = .finished
            Self.zero(&passwordLine)
            meta("‹spawn failed: \(executable.path): \(error)›")
            emit(.exited(ExitFacts(
                furthestMilestone: nil,
                stopInitiator: .none,
                exitStatus: error.exitStatus,
                lastOutputSeq: nextSeq &- 1
            )))
            return
        }
        pid = process.pid
        stdinFD = process.stdinWrite
        let pid = process.pid
        exitSource = ProcessRunner.monitorExit(
            of: pid,
            on: queue,
            // Leftovers in the group (e.g. /sbin/route) would keep the output pipes open. pppd has
            // its own session (forkpty) and is not affected.
            beforeReap: { _ = kill(-pid, SIGKILL) },
            handler: { [self] status in perform { processExited(status) } }
        )
        for (source, fd) in [(OutputSource.stdout, process.stdoutRead), (.stderr, process.stderrRead)] {
            _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
            readFDs[source] = fd
            let readSource = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
            readSource.setEventHandler { [self] in perform { readChunk(from: source) } }
            readSource.setCancelHandler { close(fd) }
            readSources[source] = readSource
            readSource.resume()
        }
        meta("‹attempt started: pid \(pid), \(executable.arguments.joined(separator: " "))›")
        advance(to: .spawned)
    }

    // MARK: - Output

    private func readChunk(from source: OutputSource) {
        guard let fd = readFDs[source] else { return }
        var buffer = [UInt8](repeating: 0, count: 65_536)
        let (count, error) = buffer.withUnsafeMutableBytes { bytes in
            let count = read(fd, bytes.baseAddress, bytes.count)
            return (count, errno)
        }
        if count > 0 {
            ingest(buffer[..<count], from: source)
        } else if count == 0 || (error != EAGAIN && error != EINTR) {
            endOfStream(source)
        }
    }

    private func ingest(_ bytes: ArraySlice<UInt8>, from source: OutputSource) {
        let now = clock.now
        switch source {
        case .stdout:
            // Line by line, so a prompt is handled after the lines printed before it.
            var start = bytes.startIndex
            while start < bytes.endIndex {
                let end = bytes[start...].firstIndex(of: 0x0A).map { $0 + 1 } ?? bytes.endIndex
                let segment = bytes[start..<end]
                handle(stdoutLines.append(segment, now: now), from: .stdout)
                for prompt in promptDetector.append(segment) {
                    handle(prompt)
                }
                start = end
            }
        case .stderr:
            handle(stderrLines.append(bytes, now: now), from: .stderr)
        case .helper:
            break
        }
        armLineFlush()
    }

    private func handle(_ lines: [AssembledLine], from source: OutputSource) {
        for line in lines {
            appendOutput(line.text, source: source, isPartial: line.isPartial)
            for signal in parser.parse(line.text, source: source) {
                handle(signal)
            }
        }
    }

    private func handle(_ signal: OpenFortiVPNSignal) {
        switch signal {
        case .connectedToGateway:
            advance(to: .connectedToGateway)
        case .authenticated:
            advance(to: .authenticated)
        case .tunnelUp:
            advance(to: .tunnelUp)
        case .authenticationFailed:
            authFailLineSeen = true
        case .tunnelModeRejected:
            tunnelModeRejected = true
        case .certificateRejected:
            certRejectedAt = certRejectedAt ?? furthest
        default:
            break
        }
    }

    private func appendOutput(_ text: String, source: OutputSource, isPartial: Bool = false) {
        batch.append(OutputLine(seq: nextSeq, date: clock.date, attemptID: attemptID, source: source, text: text, isPartial: isPartial))
        nextSeq &+= 1
        if timers[.outputBatch] == nil {
            arm(.outputBatch, at: clock.now + timing.outputBatchInterval)
        }
    }

    /// A helper meta line (DESIGN §5.2: attempt start, input written, signals, exit).
    private func meta(_ text: String) {
        appendOutput(text, source: .helper)
    }

    private func flushBatch() {
        cancelTimer(.outputBatch)
        guard !batch.isEmpty else { return }
        outbox.append(.output(batch))
        batch.removeAll()
    }

    private func emit(_ kind: HelperEvent.Kind) {
        flushBatch()
        outbox.append(.event(HelperEvent(attemptID: attemptID, date: clock.date, kind: kind)))
    }

    private func armLineFlush() {
        let deadlines = [stdoutLines.idleFlushDeadline, stderrLines.idleFlushDeadline].compactMap { $0 }
        if let deadline = deadlines.min() {
            if timers[.lineFlush]?.deadline != deadline {
                arm(.lineFlush, at: deadline)
            }
        } else {
            cancelTimer(.lineFlush)
        }
    }

    private func flushIdleLines() {
        let now = clock.now
        handle(stdoutLines.flushIfIdle(now: now), from: .stdout)
        handle(stderrLines.flushIfIdle(now: now), from: .stderr)
        armLineFlush()
    }

    private func endOfStream(_ source: OutputSource) {
        readSources.removeValue(forKey: source)?.cancel()
        readFDs.removeValue(forKey: source)
        finishIfDrained()
    }

    // MARK: - Milestones and input

    private func advance(to milestone: Milestone) {
        if let furthest, milestone <= furthest { return }
        furthest = milestone
        var tunnelIP: String?
        if milestone == .tunnelUp {
            tunnelUpAt = clock.now
            tunnelIP = parser.tunnelAddresses?.ip
        }
        emit(.milestone(milestone, tunnelIP: tunnelIP))
        armWatchdog()
    }

    private func handle(_ prompt: PromptDetector.Prompt) {
        switch prompt {
        case .passwordPrompt:
            guard !passwordHandled, let furthest, furthest < .connectedToGateway, state == .running, !stopRequested else {
                meta("‹password prompt ignored: already answered or not expected at this stage›")
                return
            }
            passwordHandled = true
            let written = writeToStdin(passwordLine)
            Self.zero(&passwordLine)
            if written {
                meta("‹password sent›")
            }
        case .otpPrompt:
            advance(to: .otpPrompted)
        }
    }

    private func writeOTP(_ code: String) -> HelperError? {
        guard state == .running, !stopRequested else {
            return HelperError(.wrongPhase, "openfortivpn is not running")
        }
        guard !otpSubmitted else {
            return HelperError(.wrongPhase, "an OTP was already submitted for this attempt")
        }
        guard furthest == .otpPrompted else {
            return HelperError(.wrongPhase, "openfortivpn is not waiting for an OTP")
        }
        otpSubmitted = true
        var line = Array(code.utf8)
        line.append(0x0A)
        let written = writeToStdin(line)
        Self.zero(&line)
        closeStdin()
        guard written else {
            return HelperError(.wrongPhase, "openfortivpn exited before the OTP could be written")
        }
        meta("‹OTP sent›")
        advance(to: .otpSubmitted)
        return nil
    }

    /// Writes all of `bytes` (at most a few hundred, so the pipe never fills). On failure (EPIPE:
    /// the child is exiting) closes stdin and returns false.
    private func writeToStdin(_ bytes: [UInt8]) -> Bool {
        guard stdinFD >= 0 else { return false }
        var offset = 0
        while offset < bytes.count {
            let (count, error) = bytes.withUnsafeBytes { buffer in
                let count = write(stdinFD, buffer.baseAddress! + offset, buffer.count - offset)
                return (count, errno)
            }
            if count > 0 {
                offset += count
            } else if count < 0 && error == EINTR {
                continue
            } else {
                closeStdin()
                if error == EPIPE {
                    meta("‹input failed: openfortivpn exited before reading it (EPIPE)›")
                } else {
                    meta("‹input failed: \(SpawnError(code: error))›")
                }
                return false
            }
        }
        return true
    }

    private func closeStdin() {
        guard stdinFD >= 0 else { return }
        close(stdinFD)
        stdinFD = -1
    }

    // MARK: - Watchdog and stop

    private func armWatchdog() {
        guard state == .running, !stopRequested, let furthest, let watchdog = limits.watchdog(after: furthest) else {
            watchdogPhase = nil
            cancelTimer(.watchdog)
            return
        }
        watchdogPhase = watchdog.phase
        arm(.watchdog, at: clock.now + watchdog.limit)
    }

    private func watchdogExpired() {
        guard state == .running, !stopRequested, let phase = watchdogPhase else { return }
        watchdogFired = phase
        let limit = limits.watchdog(after: furthest ?? .spawned)?.limit ?? .zero
        meta("‹watchdog: \(phase.rawValue) did not finish within \(Self.format(limit))›")
        requestStop(.watchdog)
    }

    private func requestStop(_ reason: StopReason) {
        guard state == .running, !stopRequested else { return }
        stopRequested = true
        stopInitiator = .helper(reason)
        watchdogPhase = nil
        cancelTimer(.watchdog)
        _ = kill(pid, SIGTERM)
        meta("‹sent SIGTERM (reason: \(reason.rawValue))›")
        arm(.kill, at: clock.now + limits.terminate)
    }

    private func killExpired() {
        guard state == .running else { return }
        watchdogFired = watchdogFired ?? .terminate
        meta("‹sent SIGKILL: still running \(Self.format(limits.terminate)) after SIGTERM›")
        // openfortivpn first, so its status is SIGKILL even if it would otherwise react to a
        // group member dying first; then whatever else is in its group (e.g. /sbin/route).
        _ = kill(pid, SIGKILL)
        _ = kill(-pid, SIGKILL)
    }

    // MARK: - Exit

    private func processExited(_ status: ProcessExitStatus) {
        guard state == .running else { return }
        state = .exited
        exitStatus = status
        exitedAt = clock.now
        exitSource?.cancel()
        exitSource = nil
        watchdogPhase = nil
        cancelTimer(.watchdog)
        cancelTimer(.kill)
        closeStdin()
        if !readFDs.isEmpty {
            arm(.pipeDrain, at: clock.now + timing.pipeDrainLimit)
        }
        finishIfDrained()
    }

    private func finishIfDrained() {
        guard state == .exited, readFDs.isEmpty else { return }
        finish(pipesTimedOut: false)
    }

    /// The child is gone but something (pppd) still holds a pipe: take what is buffered and stop waiting.
    private func pipeDrainExpired() {
        guard state == .exited else { return }
        for source in [OutputSource.stdout, .stderr] {
            while let fd = readFDs[source] {
                var buffer = [UInt8](repeating: 0, count: 65_536)
                let count = buffer.withUnsafeMutableBytes { read(fd, $0.baseAddress, $0.count) }
                guard count > 0 else { break }
                ingest(buffer[..<count], from: source)
            }
        }
        finish(pipesTimedOut: true)
    }

    private func finish(pipesTimedOut: Bool) {
        guard state == .exited, let exitStatus, let exitedAt else { return }
        state = .finished
        for source in readSources.values {
            source.cancel()
        }
        readSources.removeAll()
        readFDs.removeAll()
        for kind in Array(timers.keys) where kind != .outputBatch {
            cancelTimer(kind)
        }
        Self.zero(&passwordLine)

        handle(stdoutLines.finish(), from: .stdout)
        handle(stderrLines.finish(), from: .stderr)
        if pipesTimedOut {
            meta("‹output pipes still open after \(Self.format(timing.pipeDrainLimit)); not waiting any longer›")
        }
        meta("‹exited: \(Self.describe(exitStatus))›")

        var routeRepairPerformed = false
        if parser.routesTouched, !parser.routesRestored, let routeRepair {
            let outcome = routeRepair()
            routeRepairPerformed = outcome == .repaired
            switch outcome {
            case .repaired: meta("‹routes repaired›")
            case .notNeeded: meta("‹routes checked: no repair needed›")
            case .failed: meta("‹route repair failed — see the Helper log›")
            }
        }

        emit(.exited(ExitFacts(
            furthestMilestone: furthest,
            stopInitiator: stopInitiator,
            watchdogFired: watchdogFired,
            certRejectedDigests: parser.rejectedCertificateDigests,
            certRejectedAt: certRejectedAt,
            authFailLineSeen: authFailLineSeen,
            tunnelModeRejected: tunnelModeRejected,
            upDuration: tunnelUpAt.map { (exitedAt - $0).timeInterval },
            exitStatus: exitStatus,
            routeRepairPerformed: routeRepairPerformed,
            lastOutputSeq: nextSeq &- 1
        )))
    }

    // MARK: - Timers

    private func arm(_ kind: TimerKind, at deadline: ContinuousClock.Instant) {
        timers[kind]?.timer.cancel()
        timerGeneration &+= 1
        let generation = timerGeneration
        // Weak: a cancelled dispatch timer keeps its closure until the original deadline.
        let timer = clock.schedule(at: deadline, on: queue) { [weak self] in
            guard let self else { return }
            perform { timerFired(kind, generation: generation) }
        }
        timers[kind] = ArmedTimer(generation: generation, deadline: deadline, timer: timer)
    }

    private func cancelTimer(_ kind: TimerKind) {
        timers.removeValue(forKey: kind)?.timer.cancel()
    }

    private func timerFired(_ kind: TimerKind, generation: UInt64) {
        guard let armed = timers[kind], armed.generation == generation else { return }
        guard clock.now >= armed.deadline else {
            // Fired early (dispatch time vs. continuous time); never act before the deadline.
            arm(kind, at: armed.deadline)
            return
        }
        timers[kind] = nil
        switch kind {
        case .watchdog: watchdogExpired()
        case .kill: killExpired()
        case .outputBatch: flushBatch()
        case .lineFlush: flushIdleLines()
        case .pipeDrain: pipeDrainExpired()
        }
    }

    // MARK: - Helpers

    private static func zero(_ bytes: inout [UInt8]) {
        bytes.withUnsafeMutableBytes { buffer in
            guard let base = buffer.baseAddress else { return }
            _ = memset_s(base, buffer.count, 0, buffer.count)
        }
        bytes.removeAll()
    }

    /// `30s`, `0.25s`.
    static func format(_ duration: Duration) -> String {
        let seconds = duration.timeInterval
        if seconds == seconds.rounded() {
            return "\(Int(seconds))s"
        }
        var text = String(format: "%.3f", seconds)
        while text.hasSuffix("0") {
            text.removeLast()
        }
        return "\(text)s"
    }

    static func describe(_ status: ProcessExitStatus) -> String {
        switch status {
        case .exited(let code):
            "code \(code)"
        case .signaled(let signal):
            "signal \(signal)\(signalName(signal).map { " (\($0))" } ?? "")"
        case .spawnFailed(let code):
            "spawn failed (\(SpawnError(code: code)))"
        case .unknown:
            "unknown"
        }
    }

    private static func signalName(_ signal: Int32) -> String? {
        switch signal {
        case SIGHUP: "SIGHUP"
        case SIGINT: "SIGINT"
        case SIGQUIT: "SIGQUIT"
        case SIGABRT: "SIGABRT"
        case SIGKILL: "SIGKILL"
        case SIGSEGV: "SIGSEGV"
        case SIGPIPE: "SIGPIPE"
        case SIGTERM: "SIGTERM"
        default: nil
        }
    }
}
