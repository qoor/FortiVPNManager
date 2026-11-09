import Foundation
import FortiShared
import os
import Synchronization

/// Carries the runtime's Helper calls over one XPC connection and passes the Helper's messages back.
/// Payloads cross unread; the one reply read here is the build, which marks its connection verified
/// when it is the app's. A new connection is opened when there is none or the last one was lost.
///
/// - Calls that send secrets or change routes go only over a verified connection.
/// - The build, the status and the route calls are tried once more on a fresh connection when the
///   connection fails: a Helper whose executable a reinstall replaced exits instead of answering, and
///   the system starts the new one for the next connection. Other calls are never repeated or moved.
@MainActor
final class HelperRelay {
    typealias Open = @MainActor (_ id: UInt64, _ inbound: HelperInbound) throws(AppInternal.HelperChannelFailure)
        -> any HelperChannel

    private let appBuild: String
    private let inbound: HelperInbound
    private let open: Open
    private var link: Link?
    private var nextLinkID: UInt64 = 0
    /// The connection whose Helper answered with the app's build.
    private var verifiedLinkID: UInt64?
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "helper-client")

    /// Replies normally arrive within milliseconds; this only guards against a hung Helper.
    private static let replyTimeout = DispatchTimeInterval.seconds(20)
    /// Gives the system time to start the replaced Helper.
    private static let retryDelay = Duration.milliseconds(1500)

    init(appBuild: String, inbound: HelperInbound, open: @escaping Open = HelperRelay.xpcConnection) {
        self.appBuild = appBuild
        self.inbound = inbound
        self.open = open
    }

    func build(reply: @escaping @Sendable (AppInternal.PortResponse) -> Void) async {
        switch await onConnection(retrying: true, { await self.version(on: $0) }) {
        case let .success(build): reply(.helperBuild(build: build))
        case let .failure(failure): reply(.helperUnreachable(failure: failure))
        }
    }

    func handle(_ call: AppInternal.HelperCall, reply: @escaping @Sendable (AppInternal.PortResponse) -> Void) async {
        switch call {
        case let .connect(json):
            let request = Data(json.utf8)
            reply(response(await relay(verified: true, retrying: false) { (helper, done: @escaping @Sendable (Data?) -> Void) in
                helper.connect(request, reply: done)
            }))
        case let .submitOtp(attempt, code):
            reply(response(await relay(verified: true, retrying: false) { (helper, done: @escaping @Sendable (Data?) -> Void) in
                helper.submitOTP(attemptID: attempt, code: code, reply: done)
            }))
        case let .disconnect(attempt, reason):
            reply(response(await relay(verified: false, retrying: false) { (helper, done: @escaping @Sendable (Data?) -> Void) in
                helper.disconnect(attemptID: attempt, reason: reason, reply: done)
            }))
        case .status:
            reply(response(await relay(verified: false, retrying: true) { (helper, done: @escaping @Sendable (Data) -> Void) in
                helper.status(reply: done)
            }))
        case .exitWhenIdle:
            await exitWhenIdle(reply: reply)
        case .readBypassRoutes:
            reply(response(await relay(verified: true, retrying: true) { (helper, done: @escaping @Sendable (Data) -> Void) in
                helper.bypassRoutes(reply: done)
            }))
        case let .writeBypassRoutes(json):
            let request = Data(json.utf8)
            reply(response(await relay(verified: true, retrying: true) { (helper, done: @escaping @Sendable (Data) -> Void) in
                helper.setBypassRoutes(request, reply: done)
            }))
        }
    }

    // MARK: - Calls

    private struct Link {
        let channel: any HelperChannel
        let id: UInt64
    }

    /// A Helper of another build gets no call that needs a verified connection.
    private enum Answer<Reply> {
        case reply(Reply)
        case buildMismatch(helperBuild: String)
    }

    private func relay<Reply: Sendable>(
        verified: Bool, retrying: Bool,
        _ request: (HelperXPCProtocol, @escaping @Sendable (Reply) -> Void) -> Void
    ) async -> Result<Answer<Reply>, AppInternal.HelperChannelFailure> {
        await onConnection(retrying: retrying) { link in
            if verified, verifiedLinkID != link.id {
                switch await version(on: link) {
                case let .failure(failure): return .failure(failure)
                case let .success(build) where build != appBuild: return .success(.buildMismatch(helperBuild: build))
                case .success: break
                }
            }
            guard inbound.isActive(link.id) else { return .failure(.connectionLost) }
            return await send(link, request).map(Answer.reply)
        }
    }

    /// Asks a Helper with no run to exit, then drops the connection: that clears its verified mark and
    /// reports it lost, after the reply.
    private func exitWhenIdle(reply: @escaping @Sendable (AppInternal.PortResponse) -> Void) async {
        let link: Link
        do {
            link = try currentLink()
        } catch {
            reply(.helperUnreachable(failure: error))
            return
        }
        switch await send(link, { (helper, done: @escaping @Sendable (Bool) -> Void) in helper.exitWhenIdle(reply: done) }) {
        case let .success(willExit):
            reply(.helperReplyFlag(value: willExit))
            invalidate(link)
        case let .failure(failure):
            reply(.helperUnreachable(failure: failure))
        }
    }

    /// The Helper's build on `link`, which is verified when the build is the app's.
    private func version(on link: Link) async -> Result<String, AppInternal.HelperChannelFailure> {
        let data: Data
        switch await send(link, { (helper, done: @escaping @Sendable (Data) -> Void) in helper.getVersion(reply: done) }) {
        case let .success(reply): data = reply
        case let .failure(failure): return .failure(failure)
        }
        let build: String
        do {
            build = try XPCCoding.decode(BuildID.self, from: data).value
        } catch {
            return .failure(.unreadableReply(description: error.localizedDescription))
        }
        if build == appBuild, inbound.isActive(link.id) {
            verifiedLinkID = link.id
        }
        return .success(build)
    }

    private func response(_ result: Result<Answer<Data?>, AppInternal.HelperChannelFailure>) -> AppInternal.PortResponse {
        response(result) { $0.map { String(decoding: $0, as: UTF8.self) } }
    }

    private func response(_ result: Result<Answer<Data>, AppInternal.HelperChannelFailure>) -> AppInternal.PortResponse {
        response(result) { String(decoding: $0, as: UTF8.self) }
    }

    private func response<Reply>(
        _ result: Result<Answer<Reply>, AppInternal.HelperChannelFailure>, json: (Reply) -> String?
    ) -> AppInternal.PortResponse {
        switch result {
        case let .success(.reply(reply)): .helperReply(json: json(reply))
        case let .success(.buildMismatch(helperBuild)): .helperBuildMismatch(helperBuild: helperBuild)
        case let .failure(failure): .helperUnreachable(failure: failure)
        }
    }

    // MARK: - Connection

    /// Runs `body` on the current connection, and once more on a fresh one when `retrying` and the
    /// connection failed.
    private func onConnection<Success>(
        retrying: Bool, _ body: @MainActor (Link) async -> Result<Success, AppInternal.HelperChannelFailure>
    ) async -> Result<Success, AppInternal.HelperChannelFailure> {
        let link: Link
        do {
            link = try currentLink()
        } catch {
            return .failure(error)
        }
        let result = await body(link)
        guard retrying, case let .failure(failure) = result, failure.isConnectionFailure else { return result }
        invalidate(link)
        try? await Task.sleep(for: Self.retryDelay)
        let fresh: Link
        do {
            fresh = try currentLink()
        } catch {
            return .failure(error)
        }
        return await body(fresh)
    }

    /// Sends one request on `link` and waits for its reply, a connection failure or the time limit.
    private func send<Reply: Sendable>(
        _ link: Link, _ request: (HelperXPCProtocol, @escaping @Sendable (Reply) -> Void) -> Void
    ) async -> Result<Reply, AppInternal.HelperChannelFailure> {
        await withCheckedContinuation { continuation in
            let once = ResumeOnce(continuation)
            // XPC calls this on its own queue: the closure must not inherit main-actor isolation.
            let proxy = link.channel.helper { @Sendable error in
                once.resume(.failure(.connection(description: error.localizedDescription)))
            }
            guard let proxy else {
                once.resume(.failure(.connection(description: "unexpected proxy type")))
                return
            }
            DispatchQueue.global().asyncAfter(deadline: .now() + Self.replyTimeout) { @Sendable in
                once.resume(.failure(.noReply))
            }
            request(proxy) { @Sendable reply in once.resume(.success(reply)) }
        }
    }

    /// The live connection, or a new one when there is none or the last one was lost.
    private func currentLink() throws(AppInternal.HelperChannelFailure) -> Link {
        if let link {
            if inbound.isActive(link.id) { return link }
            invalidate(link)
        }
        nextLinkID += 1
        let id = nextLinkID
        let channel = try open(id, inbound)
        inbound.activate(id)
        channel.resume()
        let link = Link(channel: channel, id: id)
        self.link = link
        logger.info("opened helper connection \(id)")
        return link
    }

    /// Drops one connection, never a newer one that replaced it. The Helper stops the run of a
    /// connection that goes away, so it is reported lost if it carried the runs.
    private func invalidate(_ target: Link) {
        inbound.end(target.id)
        target.channel.invalidate()
        if link?.id == target.id {
            link = nil
            verifiedLinkID = nil
        }
    }
}

extension HelperRelay {
    /// A connection to the Helper's mach service. Its peer must be the Helper signed by the app's team;
    /// the Helper checks the app the same way.
    static func xpcConnection(id: UInt64, inbound: HelperInbound) throws(AppInternal.HelperChannelFailure)
        -> any HelperChannel
    {
        guard let teamID = CodeSigningIdentity.currentTeamID() else { throw .appNotSigned }
        let connection = NSXPCConnection(machServiceName: FortiIdentifiers.helperMachService, options: .privileged)
        connection.remoteObjectInterface = NSXPCInterface(with: HelperXPCProtocol.self)
        connection.exportedInterface = NSXPCInterface(with: HelperClientXPCProtocol.self)
        connection.exportedObject = HelperEventSink(inbound: inbound, connectionID: id)
        connection.setCodeSigningRequirement(
            CodeSigningIdentity.requirement(identifier: FortiIdentifiers.helperBundleID, teamID: teamID)
        )
        connection.interruptionHandler = { @Sendable in inbound.end(id) }
        connection.invalidationHandler = { @Sendable in inbound.end(id) }
        return connection
    }
}

/// What the relay needs of one connection to the Helper; `NSXPCConnection` in the app.
protocol HelperChannel: AnyObject {
    /// The Helper's methods on this connection; when the connection fails, `onError` runs instead of
    /// the reply.
    func helper(onError: @escaping @Sendable (any Error) -> Void) -> HelperXPCProtocol?
    func resume()
    func invalidate()
}

extension NSXPCConnection: HelperChannel {
    func helper(onError: @escaping @Sendable (any Error) -> Void) -> HelperXPCProtocol? {
        remoteObjectProxyWithErrorHandler(onError) as? HelperXPCProtocol
    }
}

extension AppInternal.HelperChannelFailure: Error {}

extension AppInternal.HelperChannelFailure {
    /// The connection went away, so a fresh one may answer.
    fileprivate var isConnectionFailure: Bool {
        switch self {
        case .connection, .connectionLost: true
        case .appNotSigned, .noReply, .unreadableReply: false
        }
    }
}

/// The messages of the connection that carries the runs, passed on as system events in the order the
/// Helper sent them. Only the active connection passes anything, and its loss is reported once, after
/// all it passed: both happen under the lock.
nonisolated final class HelperInbound: Sendable {
    private let post: @Sendable (AppInternal.SystemEvent) -> Void
    private let activeID = Mutex<UInt64?>(nil)

    init(post: @escaping @Sendable (AppInternal.SystemEvent) -> Void) {
        self.post = post
    }

    func activate(_ id: UInt64) {
        activeID.withLock { $0 = id }
    }

    func isActive(_ id: UInt64) -> Bool {
        activeID.withLock { $0 == id }
    }

    func pass(_ event: AppInternal.SystemEvent, from id: UInt64) {
        activeID.withLock { active in
            guard active == id else { return }
            post(event)
        }
    }

    /// Reports the loss once, if `id` is still the active connection.
    func end(_ id: UInt64) {
        activeID.withLock { active in
            guard active == id else { return }
            active = nil
            post(.helperConnectionLost)
        }
    }
}

/// Exported to the Helper on one connection. XPC calls it on a private queue; each message is passed
/// on synchronously, never through a task per message, which would lose their order.
nonisolated final class HelperEventSink: NSObject, HelperClientXPCProtocol, Sendable {
    private let inbound: HelperInbound
    private let connectionID: UInt64

    init(inbound: HelperInbound, connectionID: UInt64) {
        self.inbound = inbound
        self.connectionID = connectionID
    }

    func event(_ data: Data) {
        inbound.pass(.helperEvent(json: String(decoding: data, as: UTF8.self)), from: connectionID)
    }

    func output(_ data: Data) {
        inbound.pass(.helperOutput(json: String(decoding: data, as: UTF8.self)), from: connectionID)
    }
}

/// Resumes a continuation at most once: XPC may call both the reply and the error handler, and the
/// time limit may pass as well.
private nonisolated final class ResumeOnce<Value: Sendable>: Sendable {
    private let continuation: Mutex<CheckedContinuation<Value, Never>?>

    init(_ continuation: CheckedContinuation<Value, Never>) {
        self.continuation = Mutex(continuation)
    }

    func resume(_ value: Value) {
        let taken = continuation.withLock { stored in
            defer { stored = nil }
            return stored
        }
        taken?.resume(returning: value)
    }
}
