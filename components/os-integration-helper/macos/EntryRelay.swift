import Dispatch
import Foundation
import os
import Synchronization

/// Connects the XPC side of the Helper to the Helper entry (Rust): announces app connections, passes
/// app requests through unread, and carries out what the entry sends back. The process has one
/// entry, so this is process-wide.
enum EntryRelay {
    /// Where the entry's answer to one request goes.
    enum Reply: Sendable {
        /// connect, submitOTP and disconnect; `nil` is the empty (success) reply.
        case optionalData(@Sendable (Data?) -> Void)
        /// status.
        case data(@Sendable (Data) -> Void)
        /// exitWhenIdle.
        case flag(@Sendable (Bool) -> Void)
    }

    private struct State {
        var clients: [UInt64: ClientLink] = [:]
        var lastClientID: UInt64 = 0
        var replies: [UInt64: Reply] = [:]
        var lastRequestID: UInt64 = 0
        /// Cleared on SIGTERM and when the entry says to exit.
        var accepting = true
        var replacementReported = false
    }

    private static let state = Mutex(State())
    /// The first use starts the entry. `receive` may run before that use returns; its own uses of
    /// `stream` then wait until the initialization has finished.
    private static let stream = HelperEntry.start { receive($0) }
    private static let ready = DispatchSemaphore(value: 0)
    /// A slow `/sbin/route` must not hold up the entry's other messages.
    private static let routeRepairQueue = DispatchQueue(label: "com.qoor.FortiVPNManager.Helper.route-repair")
    /// How long exiting waits for an app that stops reading.
    private static let drainLimit: DispatchTimeInterval = .seconds(1)

    /// Starts the entry and returns once app connections may be accepted: the entry first cleans up
    /// after an earlier Helper that died during a run.
    static func start() {
        _ = stream
        ready.wait()
    }

    /// Registers and announces a connection that passed the signature check, or returns `nil` once
    /// the Helper is on its way out. A replacement of the executable is announced first, so the
    /// entry can refuse the connection.
    static func admit(_ connection: NSXPCConnection, executableReplaced: Bool) -> ClientLink? {
        // Sent under the lock, so a replacement the watcher reports meanwhile cannot come after it.
        state.withLock { state in
            guard state.accepting else { return nil }
            if executableReplaced { reportReplacement(&state) }
            state.lastClientID += 1
            let client = ClientLink(id: state.lastClientID, connection: connection)
            state.clients[client.id] = client
            stream.send(.appConnected(app: client.id, uid: client.uid))
            return client
        }
    }

    /// The connection is gone, whether the app quit, crashed or was killed.
    static func disconnected(_ client: ClientLink) {
        state.withLock { _ = $0.clients.removeValue(forKey: client.id) }
        stream.send(.appDisconnected(app: client.id))
    }

    /// The self-update watcher saw the executable replaced.
    static func executableReplaced() {
        state.withLock { reportReplacement(&$0) }
    }

    /// SIGTERM: accept no more connections; the entry ends the run and says when to exit.
    static func terminationRequested() {
        HelperLog.helper.notice("SIGTERM received")
        state.withLock { $0.accepting = false }
        stream.send(.terminationRequested)
    }

    /// Passes an app request to the entry; `reply` gets its answer.
    static func forward(_ request: HelperInternal.AppRequest, from client: ClientLink, reply: Reply) {
        let id = state.withLock { state in
            state.lastRequestID += 1
            state.replies[state.lastRequestID] = reply
            return state.lastRequestID
        }
        stream.send(.request(id: id, app: client.id, request: request))
    }

    private static func reportReplacement(_ state: inout State) {
        guard !state.replacementReported else { return }
        state.replacementReported = true
        stream.send(.executableReplaced)
    }

    /// Runs on the stream's reader thread, one message at a time, so callbacks reach each app in the
    /// order the entry sent them.
    private static func receive(_ message: HelperInternal.FromEntry) {
        switch message {
        case .ready:
            ready.signal()
        case .reply(let id, let json):
            let data = json.map { Data($0.utf8) }
            switch takeReply(id) {
            case .optionalData(let reply):
                reply(data)
            case .data(let reply):
                guard let data else { fatalError("the Helper entry answered request \(id) without data") }
                reply(data)
            case .flag:
                fatalError("the Helper entry answered request \(id) with data instead of a flag")
            }
        case .replyFlag(let id, let value):
            guard case .flag(let reply) = takeReply(id) else {
                fatalError("the Helper entry answered request \(id) with a flag instead of data")
            }
            reply(value)
        case .sendEvent(let app, let json):
            client(app)?.send(event: Data(json.utf8))
        case .sendOutput(let app, let json):
            client(app)?.send(output: Data(json.utf8))
        case .repairRoutes(let gateway):
            routeRepairQueue.async {
                stream.send(.routesRepaired(outcome: RouteRepair.repair(gatewayIP: gateway)))
            }
        case .exitAfterDraining:
            exitAfterDraining()
        case .exitNow:
            exit(EXIT_SUCCESS)
        case .log(let level, let category, let message):
            let logger = switch category {
            case .helper: HelperLog.helper
            case .routes: HelperLog.routes
            }
            let type: OSLogType = switch level {
            case .info: .info
            case .notice: .default
            case .error: .error
            }
            logger.log(level: type, "\(message, privacy: .public)")
        }
    }

    /// `nil` once the app is gone: the entry may still send to it until it reads `appDisconnected`.
    private static func client(_ id: UInt64) -> ClientLink? {
        state.withLock { $0.clients[id] }
    }

    private static func takeReply(_ id: UInt64) -> Reply {
        guard let reply = state.withLock({ $0.replies.removeValue(forKey: id) }) else {
            fatalError("the Helper entry answered request \(id), which is not pending")
        }
        return reply
    }

    /// Exits once everything sent to the apps has left, or after `drainLimit`.
    private static func exitAfterDraining() {
        let clients = state.withLock { state in
            state.accepting = false
            return Array(state.clients.values)
        }
        let drained = DispatchGroup()
        for client in clients {
            drained.enter()
            client.afterPendingSends { drained.leave() }
        }
        drained.notify(queue: .global()) { exit(EXIT_SUCCESS) }
        DispatchQueue.global().asyncAfter(deadline: .now() + drainLimit) { exit(EXIT_SUCCESS) }
    }
}
