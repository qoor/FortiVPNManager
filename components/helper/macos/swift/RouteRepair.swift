import Foundation
import SystemConfiguration

/// Repairs the IPv4 routing table after the VPN client exited without restoring it: restores the
/// default route via the primary service's router and removes the gateway host route the client
/// added. The router is checked as dotted-quad IPv4 before it reaches `/sbin/route`; the gateway
/// comes from the Helper entry as an IPv4 address.
enum RouteRepair {
    static func repair(gatewayIP: String?) -> HelperInternal.RouteRepairOutcome {
        var outcome = HelperInternal.RouteRepairOutcome.notNeeded
        let defaultRoute = RouteTable.lookup("default")
        if defaultRoute == nil || isPPPDefault(defaultRoute) {
            outcome = outcome.combined(with: restoreDefaultRoute(replacing: defaultRoute))
        }
        if let gatewayIP, let removed = removeHostRoute(to: gatewayIP) {
            outcome = outcome.combined(with: removed ? .repaired : .failed)
        }
        return outcome
    }

    private static func restoreDefaultRoute(replacing current: RouteTable.Entry?) -> HelperInternal.RouteRepairOutcome {
        guard let primary = primaryIPv4() else {
            // No usable network (e.g. Wi-Fi off): there is no route to restore yet; configd adds
            // the default route when a network comes up.
            if current == nil {
                HelperLog.routes.notice("no default route and no primary IPv4 service; nothing to restore")
                return .notNeeded
            }
            HelperLog.routes.error("default route points into a ppp interface, but SCDynamicStore has no usable IPv4 router")
            return .failed
        }
        // pppd may remove its default route between the lookup and the delete, so a failed delete
        // is fine as long as the ppp default is gone afterwards.
        if current != nil, !RouteTable.change(["delete", "default"]), isPPPDefault(RouteTable.lookup("default")) {
            return .failed
        }
        RouteTable.change(["add", "default", primary.router])
        guard let restored = RouteTable.lookup("default"), !isPPPDefault(restored) else {
            HelperLog.routes.error("default route via \(primary.router, privacy: .public) is still missing after route add")
            return .failed
        }
        HelperLog.routes.notice("restored the default route via \(primary.router, privacy: .public) (\(primary.interface, privacy: .public))")
        return .repaired
    }

    private static func isPPPDefault(_ entry: RouteTable.Entry?) -> Bool {
        entry?.interface?.hasPrefix("ppp") == true
    }

    /// Deletes the static host route to the gateway (`route add -host <gateway> <router>` by
    /// openfortivpn or vpnc-script), if it is still there. `nil` when there was nothing to delete.
    private static func removeHostRoute(to gateway: String) -> Bool? {
        guard let entry = RouteTable.lookup(gateway),
              entry.destination == gateway,
              entry.flags.isSuperset(of: ["GATEWAY", "HOST", "STATIC"])
        else { return nil }
        RouteTable.change(["delete", "-host", gateway])
        if let remaining = RouteTable.lookup(gateway), remaining.destination == gateway,
           remaining.flags.isSuperset(of: ["GATEWAY", "HOST", "STATIC"]) {
            HelperLog.routes.error("gateway host route to \(gateway, privacy: .public) is still present after route delete")
            return false
        }
        HelperLog.routes.notice("removed the gateway host route to \(gateway, privacy: .public)")
        return true
    }

    /// `Router` and `PrimaryInterface` of `State:/Network/Global/IPv4`, unless the router is not
    /// IPv4 or the primary interface is a ppp tunnel.
    private static func primaryIPv4() -> (router: String, interface: String)? {
        guard let store = SCDynamicStoreCreate(nil, FortiIdentifiers.helperBundleID as CFString, nil, nil),
              let global = SCDynamicStoreCopyValue(store, "State:/Network/Global/IPv4" as CFString) as? [String: Any],
              let router = global[kSCPropNetIPv4Router as String] as? String,
              let interface = global[kSCDynamicStorePropNetPrimaryInterface as String] as? String,
              IPv4Network.parseDottedQuad(Substring(router)) != nil,
              !interface.hasPrefix("ppp")
        else { return nil }
        return (router, interface)
    }
}

private extension HelperInternal.RouteRepairOutcome {
    /// A failure wins over a repair, which wins over "nothing to do".
    func combined(with other: HelperInternal.RouteRepairOutcome) -> HelperInternal.RouteRepairOutcome {
        if self == .failed || other == .failed { return .failed }
        if self == .repaired || other == .repaired { return .repaired }
        return .notNeeded
    }
}
