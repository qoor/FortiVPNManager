import Foundation
import SystemConfiguration

/// Reports changes to the saved IPv4 settings of any network service (networksetup, System Settings)
/// and to the primary IPv4 state (another service or router), from SCDynamicStore notifications on
/// the main run loop (DESIGN §5.8). configd publishes `Setup:` keys whenever network settings are
/// applied, so additional routes changed outside the app are noticed right away.
final class NetworkSettingsWatcher {
    /// Called on the main actor, possibly several times for one change.
    var onChange: (() -> Void)?

    private var store: SCDynamicStore?
    private var source: CFRunLoopSource?

    static let keys = ["State:/Network/Global/IPv4"]
    static let patterns = ["Setup:/Network/Service/[^/]+/IPv4"]

    /// Subscribes once; the watcher must stay alive while subscribed (the store holds it unretained).
    func start() {
        guard store == nil else { return }
        var context = SCDynamicStoreContext(
            version: 0, info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil, release: nil, copyDescription: nil)
        guard let store = SCDynamicStoreCreate(nil, "FortiVPNManager" as CFString, { _, _, info in
            guard let info else { return }
            // Delivered on the main run loop, where the source is scheduled.
            MainActor.assumeIsolated {
                Unmanaged<NetworkSettingsWatcher>.fromOpaque(info).takeUnretainedValue().onChange?()
            }
        }, &context),
            SCDynamicStoreSetNotificationKeys(store, Self.keys as CFArray, Self.patterns as CFArray),
            let source = SCDynamicStoreCreateRunLoopSource(nil, store, 0)
        else { return }
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        self.store = store
        self.source = source
    }
}
