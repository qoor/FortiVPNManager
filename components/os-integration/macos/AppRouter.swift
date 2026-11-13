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
        // The only notification is about exception-rule routes that could not be written.
        userNotifier.onOpen = { [ui] in ui.settings.show(.exceptionRules) }
        userNotifier.start()
        events.start()
    }

    func receive(_ message: AppInternal.FromRuntime) {
        switch message {
        case let .section(section): ui.apply(section)
        case let .logLines(lines): ui.log.append(lines)
        case let .portRequest(id, request): answer(request) { [send] in send(.portResponse(id: id, response: $0)) }
        case let .portAction(action): perform(action)
        case .openSettings: ui.openSettings()
        }
    }

    private func answer(_ request: AppInternal.PortRequest, reply: @escaping @Sendable (AppInternal.PortResponse) -> Void) {
        switch request {
        case let .readSecret(item): reply(SecretStore.read(item))
        case let .hasSecret(item): reply(SecretStore.exists(item))
        case let .writeSecret(item, value): reply(SecretStore.write(value, to: item))
        case let .deleteSecret(item): reply(SecretStore.delete(item))
        case .helperRegistration: reply(ServiceRegistration.helperStatus())
        case .registerHelper: reply(ServiceRegistration.registerHelper())
        case .helperBuild: Task { await relay.build(reply: reply) }
        case let .callHelper(call): Task { await relay.handle(call, reply: reply) }
        case let .routeInterface(destination):
            SystemFacts.routeInterface(to: destination) { reply(.routeInterface(interface: $0)) }
        case .interfaceAddresses: reply(.interfaceAddresses(addresses: SystemFacts.interfaceAddresses()))
        case .primaryRouter: reply(.primaryRouter(router: SystemFacts.primaryRouter()))
        case .screenState: reply(SystemFacts.screenState())
        case let .openBrowser(url):
            reply(.browserOpened(opened: URL(string: url).map { NSWorkspace.shared.open($0) } ?? false))
        case .loginItemRegistration: reply(ServiceRegistration.loginItemStatus())
        case let .setLoginItem(enabled): reply(ServiceRegistration.setLoginItem(enabled))
        // macOS imports openfortivpn's settings file instead.
        case .fortiClientConnections: reply(.fortiClientConnections(connections: []))
        }
    }

    private func perform(_ action: AppInternal.PortAction) {
        switch action {
        case let .log(level, category, message): SystemLog.write(level, category, message)
        case let .postNotification(notification):
            userNotifier.post(identifier: notification.identifier, title: notification.title, body: notification.body)
        case let .holdActivity(hold): lifetime.holdActivity(hold)
        case .openHelperApprovalSettings: ServiceRegistration.openApprovalSettings()
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
            installedCopy: ServiceRegistration.isInstalledCopy(at: bundle.bundleURL),
            builtInOauthClient: client,
            timeZone: TimeZone.current.identifier,
            signInPages: .localized
        )
    }
}
