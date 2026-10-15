import Foundation
import Network
import os
import FortiCore
import FortiShared

/// Where an IMAP session connects. The app always uses `.gmail`; a plain-TCP endpoint exists for tests.
nonisolated struct IMAPEndpoint: Sendable {
    var host: NWEndpoint.Host
    var port: NWEndpoint.Port
    var usesTLS = true

    static let gmail = IMAPEndpoint(host: "imap.gmail.com", port: .imaps)
}

/// Why an IMAP connection or command failed. Every case except `.rejected` ends the connection.
nonisolated enum IMAPTransportError: Error, Sendable {
    /// DNS, TCP or TLS failure as reported by Network.framework.
    case network(String)
    /// No answer in time. The associated value names the step (localized "Connecting to the server", `"UID SEARCH"`, ...).
    case timedOut(String)
    /// `* BYE` outside LOGOUT, or the server closed the stream.
    case serverClosed(String?)
    /// Unparsable or out-of-sequence server data.
    case protocolViolation(String)
    /// The command could not be serialized (e.g. an empty username).
    case invalidCommand(IMAPCommandError)
    /// Tagged `NO`/`BAD`. The connection stays usable.
    case rejected(command: String, status: IMAPStatus, code: IMAPResponseCode?, text: String)
    /// Closed locally by `close()` or `logout()`.
    case closed
}

nonisolated extension IMAPTransportError: LocalizedError {
    var errorDescription: String? {
        switch self {
        case .network(let detail): String(localized: "Network error (\(detail))")
        case .timedOut(let step): String(localized: "\(step) timed out")
        case .serverClosed(let text):
            if let text {
                String(localized: "The server closed the connection: \(text)")
            } else {
                String(localized: "The server closed the connection")
            }
        case .protocolViolation(let detail): String(localized: "IMAP protocol error: \(detail)")
        case .invalidCommand(let error): String(localized: "Could not build the IMAP command (\(String(describing: error)))")
        case let .rejected(command, status, _, text): String(localized: "\(command) rejected (\(status.rawValue)): \(text)")
        case .closed: String(localized: "The connection was closed")
        }
    }
}

nonisolated extension Array where Element == IMAPResponse {
    /// UIDs from `* SEARCH` replies.
    var searchedUIDs: [UInt32] {
        flatMap { response -> [UInt32] in
            if case .search(let uids) = response { uids } else { [] }
        }
    }

    /// FETCH replies that carry UID, INTERNALDATE and a header block.
    var headerMessages: [FetchedHeaderMessage] {
        compactMap { response in
            if case .fetch(let fetch) = response { FetchedHeaderMessage(fetch) } else { nil }
        }
    }

    /// Values of every untagged `[UIDVALIDITY n]` status.
    var uidValidities: [UInt32] {
        compactMap { response in
            if case .untaggedStatus(_, .uidValidity(let value)?, _) = response { value } else { nil }
        }
    }
}

/// A minimal IMAP client connection over Network.framework (DESIGN.md §5.3).
///
/// One command is in flight at a time: `execute` sends a tagged command and, on its tagged `OK`, returns
/// the untagged responses received since the previous completion (a `NO`/`BAD` throws `.rejected`);
/// `idle(for:)` runs one IDLE → `+` → DONE → `OK` cycle.
/// The actor's executor is the connection's serial queue, so the `@Sendable` NWConnection callbacks
/// enter the actor synchronously through `assumeIsolated`. A timeout, transport error or unexpected
/// `BYE` fails the whole connection and every pending call.
actor IMAPTransport {
    nonisolated static let commandTimeout: Duration = .seconds(20)

    private enum Phase {
        /// Before `connect`.
        case initial
        /// TCP/TLS handshake or waiting for the server greeting.
        case connecting
        case open
        case finished(IMAPTransportError)
    }

    private enum IdleStage {
        /// `IDLE` sent, waiting for `+`; then idle for the given duration.
        case starting(Duration)
        case idling
        /// `DONE` sent, waiting for the tagged completion.
        case stopping
    }

    private struct PendingCommand {
        enum Kind {
            case regular
            case logout
            case idle(IdleStage)
        }

        let tag: String
        /// Log- and UI-safe command name (never carries arguments).
        let verb: String
        var kind: Kind
        let continuation: CheckedContinuation<[IMAPResponse], Error>
    }

    private let queue = DispatchSerialQueue(label: "com.qoor.FortiVPNManager.imap")
    private let connection: NWConnection
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "imap")
    private var phase = Phase.initial
    private var parser = IMAPResponseParser()
    private var tags = IMAPTagGenerator()
    private var greeting: CheckedContinuation<Void, Error>?
    private var pending: PendingCommand?
    private var untagged: [IMAPResponse] = []
    /// Set by `EXISTS` and `interruptIdle()`: ends the current IDLE, or makes the next `idle` return at once.
    private var wakeRequested = false
    private var timerGeneration: UInt64 = 0

    nonisolated var unownedExecutor: UnownedSerialExecutor { queue.asUnownedSerialExecutor() }

    init(endpoint: IMAPEndpoint) {
        connection = NWConnection(
            host: endpoint.host,
            port: endpoint.port,
            using: endpoint.usesTLS ? .tls : .tcp
        )
    }

    deinit {
        connection.cancel()
    }

    // MARK: - API

    /// Opens the connection and waits for the server greeting.
    func connect(timeout: Duration) async throws {
        guard case .initial = phase else { throw IMAPTransportError.protocolViolation(String(localized: "The connection was already started")) }
        phase = .connecting
        connection.stateUpdateHandler = { @Sendable [weak self] state in
            self?.assumeIsolated { $0.stateChanged(state) }
        }
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            greeting = continuation
            armTimer(timeout)
            connection.start(queue: queue)
        }
    }

    /// Sends `command` and returns the untagged responses received since the previous completion once
    /// the tagged `OK` arrives. Use `idle(for:)` instead of `.idle`.
    func execute(_ command: IMAPCommand, timeout: Duration = commandTimeout) async throws -> [IMAPResponse] {
        try await send(command, kind: command == .logout ? .logout : .regular, timeout: timeout)
    }

    /// Runs one IDLE cycle and returns the untagged responses received meanwhile. IDLE ends after
    /// `duration`, on `* n EXISTS` or on `interruptIdle()`; it is skipped entirely when one of those
    /// already happened since the last cycle.
    func idle(for duration: Duration) async throws -> [IMAPResponse] {
        try ensureOpen()
        if wakeRequested {
            wakeRequested = false
            defer { untagged.removeAll() }
            return untagged
        }
        return try await send(.idle, kind: .idle(.starting(duration)), timeout: Self.commandTimeout)
    }

    /// Ends the current IDLE early (or the next one before it starts). Callable from any context.
    nonisolated func interruptIdle() {
        queue.async { @Sendable [weak self] in
            self?.assumeIsolated { $0.wake() }
        }
    }

    /// Sends LOGOUT when the connection is open and idle (errors ignored), then closes it.
    func logout() async {
        if case .open = phase, pending == nil {
            _ = try? await execute(.logout)
        }
        close()
    }

    /// Cancels the connection at once; pending calls throw `.closed`.
    func close() {
        fail(.closed)
    }

    // MARK: - Commands

    private func send(
        _ command: IMAPCommand,
        kind: PendingCommand.Kind,
        timeout: Duration
    ) async throws -> [IMAPResponse] {
        try ensureOpen()
        guard pending == nil else { throw IMAPTransportError.protocolViolation(String(localized: "The previous command has not finished")) }
        let tag = tags.next()
        let bytes: Data
        do {
            bytes = try command.encoded(tag: tag)
        } catch {
            throw IMAPTransportError.invalidCommand(error)
        }
        let verb = Self.verb(of: command)
        logger.debug("→ \(verb, privacy: .public)")
        return try await withCheckedThrowingContinuation { continuation in
            pending = PendingCommand(tag: tag, verb: verb, kind: kind, continuation: continuation)
            armTimer(timeout)
            write(bytes)
        }
    }

    private func ensureOpen() throws {
        switch phase {
        case .open: return
        case .finished(let error): throw error
        case .initial, .connecting: throw IMAPTransportError.protocolViolation(String(localized: "The connection is not open"))
        }
    }

    private func write(_ bytes: Data) {
        connection.send(content: bytes, completion: .contentProcessed { @Sendable [weak self] error in
            guard let error else { return }
            self?.assumeIsolated { $0.fail(.network(String(describing: error))) }
        })
    }

    private nonisolated static func verb(of command: IMAPCommand) -> String {
        switch command {
        case .authenticatePlain: "AUTHENTICATE"
        case .list: "LIST"
        case .select: "SELECT"
        case .examine: "EXAMINE"
        case .uidSearch, .uidSearchSubject: "UID SEARCH"
        case .uidFetchHeaders: "UID FETCH"
        case .idle: "IDLE"
        case .uidMove: "UID MOVE"
        case .noop: "NOOP"
        case .logout: "LOGOUT"
        }
    }

    // MARK: - Connection callbacks (on `queue`)

    private func stateChanged(_ state: NWConnection.State) {
        switch state {
        case .ready:
            receive()
        case .waiting(let error), .failed(let error):
            // A fresh connection in `.waiting` has no route to the server; fail fast instead of
            // letting Network.framework retry until the login deadline.
            fail(.network(String(describing: error)))
        case .cancelled:
            fail(.closed)
        default:
            break
        }
    }

    private func receive() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) {
            @Sendable [weak self] data, _, isComplete, error in
            self?.assumeIsolated { $0.received(data, isComplete: isComplete, error: error) }
        }
    }

    private func received(_ data: Data?, isComplete: Bool, error: NWError?) {
        guard !isFinished else { return }
        if let data, !data.isEmpty {
            let responses: [IMAPResponse]
            do {
                responses = try parser.feed(data)
            } catch {
                fail(.protocolViolation(String(localized: "Could not parse the response (\(String(describing: error)))")))
                return
            }
            for response in responses {
                handle(response)
                if isFinished { return }
            }
        }
        if let error {
            fail(.network(String(describing: error)))
        } else if isComplete {
            fail(.serverClosed(nil))
        } else {
            receive()
        }
    }

    private func handle(_ response: IMAPResponse) {
        if case .connecting = phase {
            greeted(with: response)
            return
        }
        switch response {
        case let .tagged(tag, status, code, text):
            complete(tag: tag, status: status, code: code, text: text)
        case .continuation:
            continuationRequested()
        case .bye(_, let text):
            if case .logout? = pending?.kind {
                untagged.append(response)
            } else {
                fail(.serverClosed(text))
            }
        default:
            untagged.append(response)
            if case .exists = response { wake() }
        }
    }

    private func greeted(with response: IMAPResponse) {
        switch response {
        case .untaggedStatus(.ok, _, _):
            phase = .open
            disarmTimer()
            greeting?.resume()
            greeting = nil
        case .bye(_, let text):
            fail(.serverClosed(text))
        default:
            fail(.protocolViolation(String(localized: "Unexpected server greeting")))
        }
    }

    private func complete(tag: String, status: IMAPStatus, code: IMAPResponseCode?, text: String) {
        guard let command = pending, command.tag == tag else {
            fail(.protocolViolation(String(localized: "Response with an unknown tag")))
            return
        }
        pending = nil
        disarmTimer()
        let responses = untagged
        untagged.removeAll()
        if status == .ok {
            command.continuation.resume(returning: responses)
        } else {
            logger.notice("\(command.verb, privacy: .public) → \(status.rawValue, privacy: .public): \(text, privacy: .public)")
            command.continuation.resume(throwing: IMAPTransportError.rejected(
                command: command.verb, status: status, code: code, text: text
            ))
        }
        if case .logout = command.kind { close() }
    }

    private func continuationRequested() {
        guard var command = pending, case .idle(.starting(let duration)) = command.kind else {
            fail(.protocolViolation(String(localized: "Unexpected continuation request (+)")))
            return
        }
        command.kind = .idle(.idling)
        pending = command
        if wakeRequested {
            stopIdling()
        } else {
            armTimer(duration)
        }
    }

    private func wake() {
        wakeRequested = true
        if case .idle(.idling)? = pending?.kind { stopIdling() }
    }

    private func stopIdling() {
        guard var command = pending, case .idle(.idling) = command.kind else { return }
        command.kind = .idle(.stopping)
        pending = command
        wakeRequested = false
        write(IMAPCommand.idleDone)
        armTimer(Self.commandTimeout)
    }

    // MARK: - Timer and teardown

    private func armTimer(_ delay: Duration) {
        timerGeneration &+= 1
        let generation = timerGeneration
        queue.asyncAfter(deadline: .now() + delay / .seconds(1)) { @Sendable [weak self] in
            self?.assumeIsolated { $0.timerFired(generation) }
        }
    }

    private func disarmTimer() {
        timerGeneration &+= 1
    }

    private func timerFired(_ generation: UInt64) {
        guard generation == timerGeneration, !isFinished else { return }
        if case .idle(.idling)? = pending?.kind {
            stopIdling()
        } else {
            fail(.timedOut(pending?.verb ?? String(localized: "Connecting to the server")))
        }
    }

    private var isFinished: Bool {
        if case .finished = phase { true } else { false }
    }

    private func fail(_ error: IMAPTransportError) {
        guard !isFinished else { return }
        phase = .finished(error)
        disarmTimer()
        connection.cancel()
        untagged.removeAll()
        if case .closed = error {
            logger.debug("connection closed")
        } else {
            logger.error("IMAP connection failed: \(error.localizedDescription, privacy: .public)")
        }
        greeting?.resume(throwing: error)
        greeting = nil
        if let command = pending {
            pending = nil
            command.continuation.resume(throwing: error)
        }
    }
}
