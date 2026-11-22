import Foundation
import HelperXPC
import Synchronization
import Testing

/// The Keychain names, which must match the items already saved, and the Helper relay's rules
/// for the build check, retries, Exit When Idle and the update calls.
/// Outgoing messages are only encoded, so they are compared as the JSON the runtime reads.
private func json(_ value: some Encodable) -> String {
  String(decoding: try! MessageCoding.encoder().encode(value), as: UTF8.self)
}

private let updateCalls: [(AppInternal.HelperCall, String, String)] = [
  (.prepareUpdate(json: "{}"), "prepareUpdate", FakeHelper.prepareReply),
  (.installUpdate(json: "{}"), "installUpdate", #"{"started":{}}"#),
  (.readPreparedUpdate, "preparedUpdate", #"{"prepared":null}"#),
  (.discardPreparedUpdate, "discardPreparedUpdate", #"{"discarded":{}}"#),
]

@MainActor
struct HelperRelayTests {
  @Test func keychainNamesStayAsBefore() {
    let cases: [(AppInternal.SecretItem, String, String)] = [
      (
        .vpnPassword(username: "alice", host: "vpn.example.com"), "com.qoor.FortiVPNManager.vpn",
        "alice@vpn.example.com"
      ),
      (
        .gmailAppPassword(address: "otp@example.com"), "com.qoor.FortiVPNManager.gmail",
        "otp@example.com"
      ),
      (
        .googleRefreshToken(address: "OTP@Example.com"), "com.qoor.FortiVPNManager.google",
        "refresh-token:otp@example.com"
      ),
      (
        .googleClientSecret(clientId: "123-abc"), "com.qoor.FortiVPNManager.google",
        "client-secret:123-abc"
      ),
    ]
    for (item, service, account) in cases {
      #expect(item.keychainService == service)
      #expect(item.keychainAccount == account)
    }
  }

  @Test func aSecretCallChecksTheBuildOnceThenRelays() async throws {
    let helper = FakeHelper(build: "100")
    let relay = Relay(helpers: [helper])
    #expect(
      json(await relay.call(.connect(json: "{}")))
        == json(AppInternal.PortResponse.helperReply(json: nil)))
    #expect(
      json(await relay.call(.connect(json: "{}")))
        == json(AppInternal.PortResponse.helperReply(json: nil)))
    #expect(helper.calls.withLock { $0 } == ["getVersion", "connect", "connect"])
  }

  @Test func aHelperOfAnotherBuildGetsNoSecret() async {
    let helper = FakeHelper(build: "99")
    let relay = Relay(helpers: [helper])
    #expect(
      json(await relay.call(.submitOtp(attempt: "a", code: "123456")))
        == json(AppInternal.PortResponse.helperBuildMismatch(helperBuild: "99")))
    #expect(helper.calls.withLock { $0 } == ["getVersion"])
  }

  @Test func theStatusIsTriedOnceMoreOnAFreshConnection() async {
    let replaced = FakeHelper(build: "100", fails: true)
    let fresh = FakeHelper(build: "100")
    let relay = Relay(helpers: [replaced, fresh])
    #expect(
      json(await relay.call(.status)) == json(AppInternal.PortResponse.helperReply(json: "{}")))
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

  @Test(arguments: updateCalls)
  func anUpdateCallChecksTheBuildThenRelaysTheReplyUnread(
    call: AppInternal.HelperCall, method: String, reply: String
  ) async {
    let helper = FakeHelper(build: "100")
    let relay = Relay(helpers: [helper])
    #expect(json(await relay.call(call)) == json(AppInternal.PortResponse.helperReply(json: reply)))
    #expect(helper.calls.withLock { $0 } == ["getVersion", method])
  }

  @Test(arguments: updateCalls)
  func aHelperOfAnotherBuildGetsNoUpdateCall(
    call: AppInternal.HelperCall, method: String, reply: String
  ) async {
    let helper = FakeHelper(build: "99")
    let relay = Relay(helpers: [helper])
    #expect(
      json(await relay.call(call))
        == json(AppInternal.PortResponse.helperBuildMismatch(helperBuild: "99")))
    #expect(helper.calls.withLock { $0 } == ["getVersion"])
  }

  @Test(arguments: updateCalls)
  func anUpdateCallIsNeverRepeated(call: AppInternal.HelperCall, method: String, reply: String)
    async
  {
    let relay = Relay(helpers: [FakeHelper(build: "100", fails: true), FakeHelper(build: "100")])
    guard case .helperUnreachable = await relay.call(call) else {
      Issue.record("expected unreachable")
      return
    }
    #expect(relay.opened == 1)
  }

  @Test func anUpdateCallRunsAlongsideTheOtherCalls() async {
    let helper = FakeHelper(build: "100", holdsPrepare: true)
    let relay = Relay(helpers: [helper])
    async let prepared = relay.call(.prepareUpdate(json: "{}"))
    for _ in 0..<1000 where !helper.calls.withLock({ $0.contains("prepareUpdate") }) {
      await Task.yield()
    }
    #expect(
      json(await relay.call(.status)) == json(AppInternal.PortResponse.helperReply(json: "{}")))
    helper.releaseHeldPrepare()
    #expect(
      json(await prepared)
        == json(AppInternal.PortResponse.helperReply(json: FakeHelper.prepareReply)))
    #expect(helper.calls.withLock { $0 } == ["getVersion", "prepareUpdate", "status"])
  }

  @Test func exitWhenIdleDropsTheConnectionAfterTheReply() async {
    let relay = Relay(helpers: [FakeHelper(build: "100")])
    #expect(
      json(await relay.call(.exitWhenIdle))
        == json(AppInternal.PortResponse.helperReplyFlag(value: true)))
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
  static let prepareReply = #"{"prepared":{"version":"26.2.0"}}"#

  let build: String
  let fails: Bool
  let holdsPrepare: Bool
  let calls = Mutex<[String]>([])
  var onError: (@Sendable (any Error) -> Void)?
  private let heldPrepare = Mutex<(@Sendable () -> Void)?>(nil)

  init(build: String, fails: Bool = false, holdsPrepare: Bool = false) {
    self.build = build
    self.fails = fails
    self.holdsPrepare = holdsPrepare
  }

  func releaseHeldPrepare() {
    heldPrepare.withLock { $0 }?()
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

  func prepareUpdate(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
    answer("prepareUpdate") {
      let send: @Sendable () -> Void = { reply(Data(Self.prepareReply.utf8)) }
      if holdsPrepare {
        heldPrepare.withLock { $0 = send }
      } else {
        send()
      }
    }
  }

  func installUpdate(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
    answer("installUpdate") { reply(Data(#"{"started":{}}"#.utf8)) }
  }

  func preparedUpdate(reply: @escaping @Sendable (Data) -> Void) {
    answer("preparedUpdate") { reply(Data(#"{"prepared":null}"#.utf8)) }
  }

  func discardPreparedUpdate(reply: @escaping @Sendable (Data) -> Void) {
    answer("discardPreparedUpdate") { reply(Data(#"{"discarded":{}}"#.utf8)) }
  }
}
