import AppKit
import Network
import SystemConfiguration

/// Reports the system's events as they happen, facts only: what they mean together, such as the
/// user being away, is the runtime's call. The callbacks arrive on the main queue. Events that can
/// come just before the system sleeps take their time in the callback, since their message may reach
/// the runtime only after the wake.
@MainActor
final class SystemEvents {
    private let post: @Sendable (AppInternal.SystemEvent) -> Void
    private let pathMonitor = NWPathMonitor()
    private var observers: [(NotificationCenter, NSObjectProtocol)] = []
    private var settingsStore: SCDynamicStore?

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
        observe(distributed, Notification.Name("com.apple.screenIsLocked")) { .screenLocked(at: Date()) }
        observe(distributed, Notification.Name("com.apple.screenIsUnlocked")) { .screenUnlocked }
        let local = NotificationCenter.default
        observe(local, .NSSystemClockDidChange) { .clockChanged }
        observe(local, .NSSystemTimeZoneDidChange) {
            NSTimeZone.resetSystemTimeZone()
            return .timeZoneChanged(timeZone: TimeZone.current.identifier)
        }
        observe(local, NSApplication.didBecomeActiveNotification) { .appActivated }

        pathMonitor.pathUpdateHandler = { @Sendable [post] path in
            post(.networkPathChanged(
                satisfied: path.status == .satisfied,
                interfaces: path.availableInterfaces.map { AppInternal.PathInterface(name: $0.name, kind: Self.kind(of: $0.type)) }
            ))
        }
        pathMonitor.start(queue: .main)
        watchNetworkSettings()
    }

    private func observe(
        _ center: NotificationCenter, _ name: Notification.Name, _ event: @escaping @MainActor () -> AppInternal.SystemEvent
    ) {
        let observer = center.addObserver(forName: name, object: nil, queue: .main) { @Sendable [post] _ in
            MainActor.assumeIsolated { post(event()) }
        }
        observers.append((center, observer))
    }

    /// The saved IPv4 settings of any network service (networksetup, System Settings) and the primary
    /// IPv4 state, from SCDynamicStore on the main run loop. configd publishes the `Setup:` keys
    /// whenever network settings are applied, so routes changed outside the app show up at once. The
    /// store holds `self` unretained; this object lives as long as the app.
    private func watchNetworkSettings() {
        var context = SCDynamicStoreContext(
            version: 0, info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil, release: nil, copyDescription: nil)
        guard let store = SCDynamicStoreCreate(nil, "FortiVPNManager" as CFString, { _, _, info in
            guard let info else { return }
            // Delivered on the main run loop, where the source is scheduled.
            MainActor.assumeIsolated {
                Unmanaged<SystemEvents>.fromOpaque(info).takeUnretainedValue().post(.networkSettingsChanged)
            }
        }, &context),
            SCDynamicStoreSetNotificationKeys(
                store, ["State:/Network/Global/IPv4"] as CFArray, ["Setup:/Network/Service/[^/]+/IPv4"] as CFArray),
            let source = SCDynamicStoreCreateRunLoopSource(nil, store, 0)
        else { return }
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        settingsStore = store
    }

    nonisolated private static func kind(of type: NWInterface.InterfaceType) -> AppInternal.InterfaceKind {
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
