import Foundation
import Network
import os
import FortiCore
import FortiShared

/// Why the loopback listener ended without an authorization code.
nonisolated enum LoopbackCallbackError: Error, Sendable, Equatable {
    /// The listener could not be created or failed (text for logs).
    case listenerFailed(String)
    case timedOut
    case cancelled
    /// Google redirected with `error=` (e.g. `access_denied`); the value is sanitized.
    case denied(String)
}

/// The OAuth redirect target of the Google sign-in (RFC 8252 §7.3): an HTTP listener on
/// `127.0.0.1`, on a port the system picks, that accepts exactly one valid callback.
///
/// A request with the expected `state` and a `code` (or an `error`) ends the wait and gets a small
/// HTML page; other requests (wrong `state`, `/favicon.ico`, ...) are answered and ignored. The
/// actor's executor is the listener's serial queue, so the `@Sendable` Network.framework callbacks
/// enter it synchronously through `assumeIsolated`.
actor LoopbackCallbackServer {
    /// Simultaneous browser connections served; more are refused.
    nonisolated static let maxConnections = 16
    /// A connection that has not sent a complete request head by then is closed (browser preconnects).
    nonisolated static let connectionTimeout: TimeInterval = 15

    private struct Client {
        let connection: NWConnection
        var buffer = Data()
        /// A response is being sent; the connection closes when it is out.
        var responding = false
    }

    private let queue = DispatchSerialQueue(label: "com.qoor.FortiVPNManager.oauth-loopback")
    private let expectedState: String
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "oauth")
    private var listener: NWListener?
    private var startWaiter: CheckedContinuation<Result<UInt16, LoopbackCallbackError>, Never>?
    private var callbackWaiter: CheckedContinuation<Result<String, LoopbackCallbackError>, Never>?
    private var outcome: Result<String, LoopbackCallbackError>?
    private var clients: [ObjectIdentifier: Client] = [:]
    private var timerGeneration: UInt64 = 0

    nonisolated var unownedExecutor: UnownedSerialExecutor { queue.asUnownedSerialExecutor() }

    /// `expectedState` is the `state` sent in the authorization URL.
    init(expectedState: String) {
        self.expectedState = expectedState
    }

    // MARK: - API

    /// Starts listening on `127.0.0.1` and returns the port once the listener is ready.
    func start() async throws(LoopbackCallbackError) -> UInt16 {
        guard listener == nil, outcome == nil else { throw .listenerFailed("already started") }
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: .ipv4(.loopback), port: .any)
        parameters.acceptLocalOnly = true
        let listener: NWListener
        do {
            listener = try NWListener(using: parameters)
        } catch {
            throw .listenerFailed(String(describing: error))
        }
        self.listener = listener
        listener.stateUpdateHandler = { @Sendable [weak self] state in
            self?.assumeIsolated { $0.listenerStateChanged(state) }
        }
        listener.newConnectionHandler = { @Sendable [weak self] connection in
            guard let self else {
                connection.cancel()
                return
            }
            self.assumeIsolated { $0.accept(connection) }
        }
        let result = await withCheckedContinuation { (continuation: CheckedContinuation<Result<UInt16, LoopbackCallbackError>, Never>) in
            startWaiter = continuation
            listener.start(queue: queue)
        }
        return try result.get()
    }

    /// Waits for the one valid callback and returns its authorization code. Ends with `.timedOut`
    /// after `timeout`, with `.cancelled` on `cancel()` or task cancellation. Stops the listener.
    func waitForCallback(timeout: TimeInterval) async throws(LoopbackCallbackError) -> String {
        let result = await withTaskCancellationHandler {
            await withCheckedContinuation { (continuation: CheckedContinuation<Result<String, LoopbackCallbackError>, Never>) in
                if let outcome {
                    continuation.resume(returning: outcome)
                } else {
                    callbackWaiter = continuation
                    armTimeout(timeout)
                }
            }
        } onCancel: { [weak self] in
            self?.cancel()
        }
        return try result.get()
    }

    /// Ends the wait with `.cancelled` and closes the listener. Callable from any context.
    nonisolated func cancel() {
        queue.async { @Sendable [weak self] in
            self?.assumeIsolated { $0.finish(.failure(.cancelled)) }
        }
    }

    // MARK: - Listener (on `queue`)

    private func listenerStateChanged(_ state: NWListener.State) {
        switch state {
        case .ready:
            guard let port = listener?.port?.rawValue else {
                finish(.failure(.listenerFailed("no port")))
                return
            }
            logger.info("OAuth loopback listener ready on 127.0.0.1:\(port, privacy: .public)")
            startWaiter?.resume(returning: .success(port))
            startWaiter = nil
        case .failed(let error), .waiting(let error):
            finish(.failure(.listenerFailed(String(describing: error))))
        case .cancelled:
            finish(.failure(.cancelled))
        default:
            break
        }
    }

    private func accept(_ connection: NWConnection) {
        guard outcome == nil, clients.count < Self.maxConnections else {
            connection.cancel()
            return
        }
        let id = ObjectIdentifier(connection)
        clients[id] = Client(connection: connection)
        connection.stateUpdateHandler = { @Sendable [weak self] state in
            self?.assumeIsolated { $0.connectionStateChanged(id, state) }
        }
        connection.start(queue: queue)
        receive(id)
        queue.asyncAfter(deadline: .now() + Self.connectionTimeout) { @Sendable [weak self] in
            self?.assumeIsolated { $0.connectionTimedOut(id) }
        }
    }

    private func connectionStateChanged(_ id: ObjectIdentifier, _ state: NWConnection.State) {
        switch state {
        case .failed, .cancelled:
            drop(id)
        default:
            break
        }
    }

    private func connectionTimedOut(_ id: ObjectIdentifier) {
        guard let client = clients[id], !client.responding else { return }
        drop(id)
    }

    private func receive(_ id: ObjectIdentifier) {
        guard let connection = clients[id]?.connection else { return }
        connection.receive(minimumIncompleteLength: 1, maximumLength: 4096) {
            @Sendable [weak self] data, _, isComplete, error in
            self?.assumeIsolated { $0.received(id, data: data, isComplete: isComplete, error: error) }
        }
    }

    private func received(_ id: ObjectIdentifier, data: Data?, isComplete: Bool, error: NWError?) {
        guard var client = clients[id], !client.responding else { return }
        if let data { client.buffer.append(data) }
        clients[id] = client
        switch LoopbackHTTPRequest.parse(client.buffer) {
        case .incomplete:
            if error != nil || isComplete {
                drop(id)
            } else {
                receive(id)
            }
        case .malformed:
            respond(id, status: 400, reason: "Bad Request", message: Self.invalidRequestMessage)
        case let .request(request):
            handle(id, request)
        }
    }

    private func handle(_ id: ObjectIdentifier, _ request: LoopbackHTTPRequest) {
        guard outcome == nil else {
            respond(id, status: 400, reason: "Bad Request", message: Self.invalidRequestMessage)
            return
        }
        switch OAuthLoopbackCallback.decide(request, expectedState: expectedState) {
        case let .authorized(code):
            respond(id, status: 200, reason: "OK", message: Self.successMessage)
            finish(.success(code))
        case let .denied(error):
            respond(id, status: 200, reason: "OK", message: Self.deniedMessage)
            finish(.failure(.denied(error)))
        case .rejected:
            logger.notice("OAuth loopback: ignored a request without the expected state or code")
            respond(id, status: 400, reason: "Bad Request", message: Self.invalidRequestMessage)
        case .notFound:
            respond(id, status: 404, reason: "Not Found", message: "Not Found")
        }
    }

    private func respond(_ id: ObjectIdentifier, status: Int, reason: String, message: String) {
        guard var client = clients[id], !client.responding else { return }
        client.responding = true
        clients[id] = client
        let page = LoopbackHTTPResponse.html(status: status, reason: reason, title: "FortiVPNManager", message: message)
        client.connection.send(content: page, contentContext: .finalMessage, isComplete: true, completion: .contentProcessed {
            @Sendable [weak self] _ in
            self?.assumeIsolated { $0.drop(id) }
        })
    }

    private func drop(_ id: ObjectIdentifier) {
        guard let client = clients.removeValue(forKey: id) else { return }
        client.connection.stateUpdateHandler = nil
        client.connection.cancel()
    }

    // MARK: - Ending

    private func armTimeout(_ timeout: TimeInterval) {
        timerGeneration &+= 1
        let generation = timerGeneration
        queue.asyncAfter(deadline: .now() + timeout) { @Sendable [weak self] in
            self?.assumeIsolated { $0.timeoutFired(generation) }
        }
    }

    private func timeoutFired(_ generation: UInt64) {
        guard generation == timerGeneration else { return }
        finish(.failure(.timedOut))
    }

    /// Records the outcome once, stops accepting connections and wakes the waiters. Connections that
    /// are still sending their page close when it is out; the rest close now.
    private func finish(_ result: Result<String, LoopbackCallbackError>) {
        guard outcome == nil else { return }
        outcome = result
        timerGeneration &+= 1
        if let listener {
            listener.stateUpdateHandler = nil
            listener.newConnectionHandler = nil
            listener.cancel()
        }
        for (id, client) in clients where !client.responding {
            drop(id)
        }
        if let startWaiter {
            self.startWaiter = nil
            switch result {
            case let .failure(error): startWaiter.resume(returning: .failure(error))
            case .success: startWaiter.resume(returning: .failure(.cancelled))
            }
        }
        if let callbackWaiter {
            self.callbackWaiter = nil
            callbackWaiter.resume(returning: result)
        }
    }

    // MARK: - Pages

    private nonisolated static var successMessage: String {
        String(localized: "FortiVPNManager: you can close this window.")
    }

    private nonisolated static var deniedMessage: String {
        String(localized: "FortiVPNManager: Google sign-in was not completed. You can close this window.")
    }

    private nonisolated static var invalidRequestMessage: String {
        String(localized: "FortiVPNManager: this sign-in response is not valid. Start the sign-in again in Settings.")
    }
}
