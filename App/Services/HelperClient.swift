import Foundation
import FortiShared
import os
import Synchronization

/// One message from the helper, in the order the helper sent it (DESIGN §5.1 ordering rules).
nonisolated enum HelperInbound: Sendable {
    case event(HelperEvent)
    case output([OutputLine])
    /// The current XPC connection was interrupted or invalidated. No `.exited` follows for its runs.
    case connectionLost
}

/// XPC client for the root helper. Both ends pin each other's signing identity (DESIGN §5.2).
///
/// Helper callbacks (`event`, `output`) and connection loss arrive on `inbound`, a single ordered
/// channel. Replies are not ordered with respect to that channel; a run has finished only when its
/// `.exited` event arrives.
final class HelperClient {
    enum ClientError: LocalizedError {
        case unsignedApp
        case connection(String)
        case decoding(String)
        case timeout

        var errorDescription: String? {
            switch self {
            case .unsignedApp: String(localized: "The app is not signed with a team certificate.")
            case .connection(let message): String(localized: "Helper connection failed: \(message)")
            case .decoding(let message): String(localized: "Could not read the Helper response: \(message)")
            case .timeout: String(localized: "The Helper is not responding.")
            }
        }
    }

    /// Helper events, output batches and connection loss, in delivery order. Consume it from one task.
    let inbound: AsyncStream<HelperInbound>
    /// BuildID of this app (`CFBundleVersion`); the helper must report the same one.
    let appBuild: BuildID

    private let channel: HelperInboundChannel
    private var link: Link?
    private var nextLinkID: UInt64 = 0
    /// Connection whose peer answered `getVersion` with `appBuild`; only it may receive the password.
    private var verifiedLinkID: UInt64?
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "helper-client")

    /// Replies normally arrive within milliseconds; this only guards against a hung helper.
    private static let replyTimeout = DispatchTimeInterval.seconds(20)

    init(appBuild: BuildID = AppBuild.current) {
        self.appBuild = appBuild
        channel = HelperInboundChannel()
        inbound = channel.stream
    }

    // MARK: - Requests

    func getVersion() async throws -> BuildID {
        try await retryingOnce { link in try await self.version(on: link) }
    }

    func status() async throws -> HelperStatus {
        try await retryingOnce { link in
            let data: Data = try await self.call(link) { proxy, reply in proxy.status(reply: reply) }
            return try Self.decode(HelperStatus.self, from: data)
        }
    }

    /// Starts openfortivpn. The request (with the password) is only sent on a connection whose peer
    /// already answered `getVersion` with this app's BuildID. Never retried, and never moved to another
    /// connection: a lost connection is reported on `inbound` instead.
    func connect(_ request: ConnectRequest) async throws(HelperError) {
        try await perform {
            let link = try self.currentLink()
            if self.verifiedLinkID != link.id {
                let build = try await self.version(on: link)
                guard build == self.appBuild else {
                    throw HelperError(.internalError, "Helper build \(build) differs from app build \(self.appBuild)")
                }
            }
            guard self.channel.isActive(link.id) else {
                throw HelperError(.internalError, "the Helper connection dropped; the connect request was not sent")
            }
            let payload = try XPCCoding.encode(request)
            let reply: Data? = try await self.call(link) { proxy, reply in proxy.connect(payload, reply: reply) }
            try Self.check(reply)
        }
    }

    /// Hands the OTP to the running attempt. Never retried.
    func submitOTP(attemptID: AttemptID, code: String) async throws(HelperError) {
        try await perform {
            let link = try self.currentLink()
            let reply: Data? = try await self.call(link) { proxy, reply in
                proxy.submitOTP(attemptID: attemptID.uuidString, code: code, reply: reply)
            }
            try Self.check(reply)
        }
    }

    /// Asks the helper to stop `attemptID` (or whatever runs, when `nil`). The reply only means the stop
    /// was accepted; completion is the `.exited` event.
    func disconnect(attemptID: AttemptID?, reason: StopReason) async throws(HelperError) {
        try await perform {
            let link = try self.currentLink()
            let reply: Data? = try await self.call(link) { proxy, reply in
                proxy.disconnect(attemptID: attemptID?.uuidString ?? "", reason: reason.rawValue, reply: reply)
            }
            try Self.check(reply)
        }
    }

    /// Asks an idle helper to exit so launchd starts the updated embedded binary on the next call.
    func exitWhenIdle() async throws -> Bool {
        let link = try currentLink()
        let accepted: Bool = try await call(link) { proxy, reply in proxy.exitWhenIdle(reply: reply) }
        invalidate(link)
        return accepted
    }

    // MARK: - Plumbing

    /// `getVersion` on one specific connection; marks it verified when the build matches.
    private func version(on link: Link) async throws -> BuildID {
        let data: Data = try await call(link) { proxy, reply in proxy.getVersion(reply: reply) }
        let build = try Self.decode(BuildID.self, from: data)
        if build == appBuild, channel.isActive(link.id) {
            verifiedLinkID = link.id
        }
        return build
    }

    private struct Link {
        let connection: NSXPCConnection
        let id: UInt64
    }

    /// Retries an idempotent request once, on a fresh connection, after a connection failure. A helper
    /// whose binary was replaced by a reinstall exits instead of answering; launchd starts the new one.
    private func retryingOnce<T>(_ request: (Link) async throws -> T) async throws -> T {
        let link = try currentLink()
        do {
            return try await request(link)
        } catch ClientError.connection {
            invalidate(link)
            try await Task.sleep(for: .milliseconds(1500))
            return try await request(currentLink())
        }
    }

    /// Drops one connection (never a newer one that replaced it). The helper stops a run whose control
    /// connection goes away, so this also reports `.connectionLost` on `inbound` if it was the active one.
    private func invalidate(_ target: Link) {
        channel.end(target.id)
        target.connection.invalidate()
        if link?.id == target.id {
            link = nil
            verifiedLinkID = nil
        }
    }

    /// Runs a non-idempotent request and maps every failure to a `HelperError`.
    private func perform(_ request: () async throws -> Void) async throws(HelperError) {
        do {
            try await request()
        } catch let error as HelperError {
            throw error
        } catch {
            throw HelperError(.internalError, error.localizedDescription)
        }
    }

    /// Sends one request and waits for its reply; a connection error or the timeout resumes it instead.
    private func call<T: Sendable>(
        _ link: Link,
        _ send: (HelperXPCProtocol, @escaping @Sendable (T) -> Void) -> Void
    ) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            let once = ResumeOnce(continuation)
            // XPC calls this on its own queue: the closure must not inherit MainActor isolation.
            let proxy = link.connection.remoteObjectProxyWithErrorHandler { @Sendable error in
                once.resume(throwing: ClientError.connection(error.localizedDescription))
            }
            guard let helper = proxy as? HelperXPCProtocol else {
                once.resume(throwing: ClientError.connection("unexpected proxy type"))
                return
            }
            DispatchQueue.global().asyncAfter(deadline: .now() + Self.replyTimeout) { @Sendable in
                once.resume(throwing: ClientError.timeout)
            }
            send(helper) { value in once.resume(returning: value) }
        }
    }

    /// The live connection, or a new one when there is none or the previous one was lost.
    private func currentLink() throws -> Link {
        if let link {
            if channel.isActive(link.id) { return link }
            invalidate(link)
        }
        guard let teamID = CodeSigningIdentity.currentTeamID() else { throw ClientError.unsignedApp }

        nextLinkID += 1
        let id = nextLinkID
        let connection = NSXPCConnection(machServiceName: FortiIdentifiers.helperMachService, options: .privileged)
        connection.remoteObjectInterface = NSXPCInterface(with: HelperXPCProtocol.self)
        connection.exportedInterface = NSXPCInterface(with: HelperClientXPCProtocol.self)
        connection.exportedObject = HelperEventSink(channel: channel, connectionID: id)
        connection.setCodeSigningRequirement(
            CodeSigningIdentity.requirement(identifier: FortiIdentifiers.helperBundleID, teamID: teamID)
        )
        // Both run on XPC's queue: explicitly @Sendable so they do not inherit MainActor isolation.
        connection.interruptionHandler = { @Sendable [channel] in channel.end(id) }
        connection.invalidationHandler = { @Sendable [channel] in channel.end(id) }
        channel.activate(id)
        connection.resume()

        let link = Link(connection: connection, id: id)
        self.link = link
        logger.info("opened helper connection \(id)")
        return link
    }

    private static func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        do {
            return try XPCCoding.decode(type, from: data)
        } catch {
            throw ClientError.decoding(error.localizedDescription)
        }
    }

    /// `nil` means success; anything else is an encoded `HelperError`.
    private static func check(_ reply: Data?) throws {
        guard let reply else { return }
        throw (try? XPCCoding.decode(HelperError.self, from: reply))
            ?? HelperError(.internalError, "could not decode the Helper's error reply")
    }
}

/// This app's BuildID (`CFBundleVersion`), compared with the helper's.
enum AppBuild {
    static let current = BuildID(Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "unknown")
}

/// The single ordered channel behind `HelperClient.inbound`.
///
/// Only the active connection may yield; messages and loss notifications from a replaced connection
/// are dropped. Yields happen under the lock, so nothing from a connection follows its `.connectionLost`.
nonisolated final class HelperInboundChannel: Sendable {
    let stream: AsyncStream<HelperInbound>
    private let continuation: AsyncStream<HelperInbound>.Continuation
    private let activeID = Mutex<UInt64?>(nil)

    init() {
        (stream, continuation) = AsyncStream.makeStream(of: HelperInbound.self, bufferingPolicy: .unbounded)
    }

    func activate(_ id: UInt64) {
        activeID.withLock { $0 = id }
    }

    func isActive(_ id: UInt64) -> Bool {
        activeID.withLock { $0 == id }
    }

    func yield(_ message: HelperInbound, from id: UInt64) {
        activeID.withLock { active in
            guard active == id else { return }
            continuation.yield(message)
        }
    }

    /// Reports `.connectionLost` once, if `id` is still the active connection.
    func end(_ id: UInt64) {
        activeID.withLock { active in
            guard active == id else { return }
            active = nil
            continuation.yield(.connectionLost)
        }
    }
}

/// Exported to the helper on one connection. XPC calls it on a private queue; it decodes and yields
/// synchronously into the shared channel (never one `Task` per message, which would lose ordering).
nonisolated final class HelperEventSink: NSObject, HelperClientXPCProtocol, Sendable {
    private static let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "helper-client")
    private let channel: HelperInboundChannel
    private let connectionID: UInt64

    init(channel: HelperInboundChannel, connectionID: UInt64) {
        self.channel = channel
        self.connectionID = connectionID
    }

    func event(_ data: Data) {
        do {
            channel.yield(.event(try XPCCoding.decode(HelperEvent.self, from: data)), from: connectionID)
        } catch {
            Self.logger.error("undecodable helper event: \(error.localizedDescription, privacy: .public)")
        }
    }

    func output(_ data: Data) {
        do {
            channel.yield(.output(try XPCCoding.decode([OutputLine].self, from: data)), from: connectionID)
        } catch {
            Self.logger.error("undecodable helper output: \(error.localizedDescription, privacy: .public)")
        }
    }
}

/// Resumes a continuation at most once; XPC may call both the reply and the error handler.
private nonisolated final class ResumeOnce<T: Sendable>: Sendable {
    private let continuation: Mutex<CheckedContinuation<T, Error>?>

    init(_ continuation: CheckedContinuation<T, Error>) {
        self.continuation = Mutex(continuation)
    }

    func resume(returning value: T) {
        take()?.resume(returning: value)
    }

    func resume(throwing error: Error) {
        take()?.resume(throwing: error)
    }

    private func take() -> CheckedContinuation<T, Error>? {
        continuation.withLock { stored in
            defer { stored = nil }
            return stored
        }
    }
}
