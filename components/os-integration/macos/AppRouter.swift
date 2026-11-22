import AppKit

/// Passes the runtime's messages on: sections, log lines and the Settings request to the UI, port
/// requests and actions to the OS integration. Sends the UI's commands, the answers to port requests
/// and the system's events to the runtime. Messages from the runtime are handled on the main actor in
/// arrival order.
@MainActor
final class AppRouter {
  let ui: AppUI
  let lifetime: AppLifetime
  private let send: @Sendable (AppInternal.ToRuntime) -> Void
  private let events: SystemEvents
  private let userNotifier = UserNotifier()
  private let updateWaiter = UpdateWaiter()
  private let relay: HelperRelay

  init(appBuild: String, send: @escaping @Sendable (AppInternal.ToRuntime) -> Void) {
    self.send = send
    let post: @Sendable (AppInternal.SystemEvent) -> Void = { send(.event($0)) }
    ui = AppUI(send: { send(.command($0)) })
    lifetime = AppLifetime(post: post)
    events = SystemEvents(post: post)
    relay = HelperRelay(appBuild: appBuild, inbound: HelperInbound(post: post))
  }

  /// Reports the app's facts, which the runtime takes before anything else, then starts reporting
  /// the system's events.
  func start(_ facts: AppInternal.AppFacts) {
    send(.launched(facts: facts))
    // The other notification is about exception-rule routes that could not be written.
    userNotifier.onOpen = { [ui] identifier in
      if identifier == AppInternal.UserNotification.updateAvailableIdentifier {
        ui.settings.showSoftwareUpdate()
      } else {
        ui.settings.show(.exceptionRules)
      }
    }
    userNotifier.onAction = { [ui] _ in
      ui.settings.showSoftwareUpdate()
      ui.send(.updateNowFromNotification)
    }
    userNotifier.start()
    events.start()
  }

  func receive(_ message: AppInternal.FromRuntime) {
    switch message {
    case .section(let section): ui.apply(section)
    case .logLines(let lines): ui.log.append(lines)
    case .portRequest(let id, let request):
      answer(request) { [send] in send(.portResponse(id: id, response: $0)) }
    case .portAction(let action): perform(action)
    case .openSettings: ui.openSettings()
    }
  }

  private func answer(
    _ request: AppInternal.PortRequest,
    reply: @escaping @Sendable (AppInternal.PortResponse) -> Void
  ) {
    switch request {
    case .helperRegistration: reply(ServiceRegistration.helperStatus())
    case .registerHelper: reply(ServiceRegistration.registerHelper())
    case .helperBuild: Task { await relay.build(reply: reply) }
    case .callHelper(let call): Task { await relay.handle(call, reply: reply) }
    case .routeInterface(let destination):
      SystemFacts.routeInterface(to: destination) { reply(.routeInterface(interface: $0)) }
    case .primaryRouter: reply(.primaryRouter(router: SystemFacts.primaryRouter()))
    case .screenState: reply(SystemFacts.screenState())
    case .openBrowser(let url):
      reply(.browserOpened(opened: URL(string: url).map { NSWorkspace.shared.open($0) } ?? false))
    case .loginItemRegistration: reply(ServiceRegistration.loginItemStatus())
    case .setLoginItem(let enabled): reply(ServiceRegistration.setLoginItem(enabled))
    // macOS imports openfortivpn's settings file instead.
    case .fortiClientConnections: reply(.fortiClientConnections(connections: []))
    case .startUpdateWaiter(let installId): reply(updateWaiter.start(installId: installId))
    }
  }

  private func perform(_ action: AppInternal.PortAction) {
    switch action {
    case .log(let level, let category, let message): SystemLog.write(level, category, message)
    case .postNotification(let notification):
      userNotifier.post(
        identifier: notification.identifier, title: notification.title, body: notification.body,
        action: notification.actionTitle
      )
    case .holdActivity(let hold): lifetime.holdActivity(hold)
    case .openHelperApprovalSettings: ServiceRegistration.openApprovalSettings()
    case .useHelperTeam(let teamID): relay.useTeam(teamID)
    case .releaseUpdateWaiter: break
    case .stopUpdateWaiter: updateWaiter.stop()
    case .approveQuit: lifetime.approveQuit()
    }
  }
}

extension AppInternal.AppFacts {
  /// The running app's facts. The built-in OAuth client is put in the bundle at build time
  /// (packaging/GoogleOAuthClient.json, gitignored).
  static func current(bundle: Bundle = .main) -> Self {
    let client = bundle.url(forResource: "GoogleOAuthClient", withExtension: "json")
      .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
    return Self(
      appBuild: bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "unknown",
      appVersion: bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as! String,
      installedCopy: ServiceRegistration.isInstalledCopy(at: bundle.bundleURL),
      builtInOauthClient: client,
      timeZone: TimeZone.current.identifier,
      signInPages: .localized
    )
  }
}
