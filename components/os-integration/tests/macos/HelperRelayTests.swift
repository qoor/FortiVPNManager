import Foundation
import FortiShared
import Synchronization
import Testing

/// The Keychain names, which must match the items earlier builds saved, and the Helper relay's rules
/// for the build check, retries and Exit When Idle.
/// Outgoing messages are only encoded, so they are compared as the JSON the runtime reads.
private func json(_ value: some Encodable) -> String {
    String(decoding: try! MessageCoding.encoder().encode(value), as: UTF8.self)
}

@MainActor
struct HelperRelayTests {
    @Test func keychainNamesStayAsBefore() {
        let cases: [(AppInternal.SecretItem, String, String)] = [
            (.vpnPassword(username: "alice", host: "vpn.example.com"), "com.qoor.FortiVPNManager.vpn", "alice@vpn.example.com"),
            (.gmailAppPassword(address: "otp@example.com"), "com.qoor.FortiVPNManager.gmail", "otp@example.com"),
            (.googleRefreshToken(address: "OTP@Example.com"), "com.qoor.FortiVPNManager.google", "refresh-token:otp@example.com"),
            (.googleClientSecret(clientId: "123-abc"), "com.qoor.FortiVPNManager.google", "client-secret:123-abc"),
        ]
        for (item, service, account) in cases {
            #expect(item.keychainService == service)
            #expect(item.keychainAccount == account)
        }
    }

    @Test func aSecretCallChecksTheBuildOnceThenRelays() async throws {
        let helper = FakeHelper(build: "100")
        let relay = Relay(helpers: [helper])
        #expect(json(await relay.call(.connect(json: "{}"))) == json(AppInternal.PortResponse.helperReply(json: nil)))
        #expect(json(await relay.call(.connect(json: "{}"))) == json(AppInternal.PortResponse.helperReply(json: nil)))
        #expect(helper.calls.withLock { $0 } == ["getVersion", "connect", "connect"])
    }

    @Test func aHelperOfAnotherBuildGetsNoSecret() async {
        let helper = FakeHelper(build: "99")
        let relay = Relay(helpers: [helper])
        #expect(json(await relay.call(.submitOtp(attempt: "a", code: "123456"))) == json(AppInternal.PortResponse.helperBuildMismatch(helperBuild: "99")))
        #expect(helper.calls.withLock { $0 } == ["getVersion"])
    }

    @Test func theStatusIsTriedOnceMoreOnAFreshConnection() async {
        let replaced = FakeHelper(build: "100", fails: true)
        let fresh = FakeHelper(build: "100")
        let relay = Relay(helpers: [replaced, fresh])
        #expect(json(await relay.call(.status)) == json(AppInternal.PortResponse.helperReply(json: "{}")))
        #expect(relay.opened == 2)
    }

    @Test func aConnectIsNeverRepeated() async {
        let failing = FakeHelper(build: "100", fails: true)
        let relay = Relay(helpers: [failing, FakeHelper(build: "100")])
        guard case .helperUnreachable = await relay.call(.connect(json: "{}")) else {
            Issue.record("expected unreachable")
            return
        }
        #expect(relay.opened == 1)
    }

    @Test func exitWhenIdleDropsTheConnectionAfterTheReply() async {
        let relay = Relay(helpers: [FakeHelper(build: "100")])
        #expect(json(await relay.call(.exitWhenIdle)) == json(AppInternal.PortResponse.helperReplyFlag(value: true)))
        #expect(json(relay.events.all) == json([AppInternal.SystemEvent.helperConnectionLost]))
    }
}

/// A relay over fake connections, opened in order.
@MainActor
private final class Relay {
    let events = EventLog()
    private(set) var opened = 0
    private var relay: HelperRelay!

    init(helpers: [FakeHelper]) {
        let inbound = HelperInbound(post: { [events] event in events.append(event) })
        var remaining = helpers
        relay = HelperRelay(appBuild: "100", inbound: inbound) { [unowned self] _, _ in
            opened += 1
            return FakeChannel(helper: remaining.removeFirst())
        }
    }

    func call(_ call: AppInternal.HelperCall) async -> AppInternal.PortResponse {
        await withCheckedContinuation { continuation in
            Task { await relay.handle(call) { continuation.resume(returning: $0) } }
        }
    }
}

private final class EventLog: Sendable {
    private let list = Mutex<[AppInternal.SystemEvent]>([])

    func append(_ event: AppInternal.SystemEvent) {
        list.withLock { $0.append(event) }
    }

    var all: [AppInternal.SystemEvent] {
        list.withLock { $0 }
    }
}

private final class FakeChannel: HelperChannel {
    let helper: FakeHelper

    init(helper: FakeHelper) {
        self.helper = helper
    }

    func helper(onError: @escaping @Sendable (any Error) -> Void) -> HelperXPCProtocol? {
        helper.onError = onError
        return helper
    }

    func resume() {}
    func invalidate() {}
}

/// Answers like a Helper of `build`, or fails every call like a Helper that exited.
private final class FakeHelper: NSObject, HelperXPCProtocol, @unchecked Sendable {
    let build: String
    let fails: Bool
    let calls = Mutex<[String]>([])
    var onError: (@Sendable (any Error) -> Void)?

    init(build: String, fails: Bool = false) {
        self.build = build
        self.fails = fails
    }

    private func answer(_ name: String, _ reply: () -> Void) {
        calls.withLock { $0.append(name) }
        if fails {
            onError?(NSError(domain: NSCocoaErrorDomain, code: NSXPCConnectionInterrupted))
        } else {
            reply()
        }
    }

    func getVersion(reply: @escaping @Sendable (Data) -> Void) {
        answer("getVersion") { reply(try! XPCCoding.encode(BuildID(build))) }
    }

    func connect(_ request: Data, reply: @escaping @Sendable (Data?) -> Void) {
        answer("connect") { reply(nil) }
    }

    func submitOTP(attemptID: String, code: String, reply: @escaping @Sendable (Data?) -> Void) {
        answer("submitOTP") { reply(nil) }
    }

    func disconnect(attemptID: String, reason: String, reply: @escaping @Sendable (Data?) -> Void) {
        answer("disconnect") { reply(nil) }
    }

    func status(reply: @escaping @Sendable (Data) -> Void) {
        answer("status") { reply(Data("{}".utf8)) }
    }

    func exitWhenIdle(reply: @escaping @Sendable (Bool) -> Void) {
        answer("exitWhenIdle") { reply(true) }
    }

    func bypassRoutes(reply: @escaping @Sendable (Data) -> Void) {
        answer("bypassRoutes") { reply(Data("{}".utf8)) }
    }

    func setBypassRoutes(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
        answer("setBypassRoutes") { reply(Data("{}".utf8)) }
    }
}
