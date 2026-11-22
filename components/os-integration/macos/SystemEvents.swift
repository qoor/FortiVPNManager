import AppKit
import Network

/// Reports the system's events as they happen, facts only: what they mean together, such as the
/// user being away, is the runtime's call. The callbacks arrive on the main queue. Events that can
/// come just before the system sleeps take their time in the callback, since their message may reach
/// the runtime only after the wake.
@MainActor
final class SystemEvents {
  private let post: @Sendable (AppInternal.SystemEvent) -> Void
  private let pathMonitor = NWPathMonitor()
  private var observers: [(NotificationCenter, NSObjectProtocol)] = []

  init(post: @escaping @Sendable (AppInternal.SystemEvent) -> Void) {
    self.post = post
  }

  func start() {
    let workspace = NSWorkspace.shared.notificationCenter
    observe(workspace, NSWorkspace.willSleepNotification) { .willSleep(at: Date()) }
    observe(workspace, NSWorkspace.didWakeNotification) { .didWake }
    observe(workspace, NSWorkspace.screensDidSleepNotification) { .displaysSlept(at: Date()) }
    observe(workspace, NSWorkspace.screensDidWakeNotification) { .displaysWoke }
    // Not documented, but posted by loginwindow on every lock and unlock for many releases.
    let distributed = DistributedNotificationCenter.default()
    observe(distributed, Notification.Name("com.apple.screenIsLocked")) {
      .screenLocked(at: Date())
    }
    observe(distributed, Notification.Name("com.apple.screenIsUnlocked")) { .screenUnlocked }
    let local = NotificationCenter.default
    observe(local, .NSSystemClockDidChange) { .clockChanged }
    observe(local, .NSSystemTimeZoneDidChange) {
      NSTimeZone.resetSystemTimeZone()
      return .timeZoneChanged(timeZone: TimeZone.current.identifier)
    }
    observe(local, NSApplication.didBecomeActiveNotification) { .appActivated }

    pathMonitor.pathUpdateHandler = { @Sendable [post] path in
      post(
        .networkPathChanged(
          satisfied: path.status == .satisfied,
          interfaces: path.availableInterfaces.map {
            AppInternal.PathInterface(name: $0.name, kind: Self.kind(of: $0.type))
          }
        ))
    }
    pathMonitor.start(queue: .main)
  }

  private func observe(
    _ center: NotificationCenter, _ name: Notification.Name,
    _ event: @escaping @MainActor () -> AppInternal.SystemEvent
  ) {
    let observer = center.addObserver(forName: name, object: nil, queue: .main) {
      @Sendable [post] _ in
      MainActor.assumeIsolated { post(event()) }
    }
    observers.append((center, observer))
  }

  nonisolated private static func kind(of type: NWInterface.InterfaceType)
    -> AppInternal.InterfaceKind
  {
    switch type {
    case .wifi: .wifi
    case .wiredEthernet: .wiredEthernet
    case .cellular: .cellular
    case .loopback: .loopback
    case .other: .other
    @unknown default: .other
    }
  }
}
