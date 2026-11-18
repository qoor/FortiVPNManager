import Foundation

/// Repairs the IPv4 routing table after the VPN client exited without restoring it: removes the
/// gateway host route the client added. The gateway comes from the Helper entry as an IPv4 address.
enum RouteRepair {
    static func repair(gatewayIP: String?) -> HelperInternal.RouteRepairOutcome {
        guard let gatewayIP, let removed = removeHostRoute(to: gatewayIP) else { return .notNeeded }
        return removed ? .repaired : .failed
    }

    /// Deletes the static host route to the gateway (`route add -host <gateway> <router>` by
    /// vpnc-script), if it is still there. `nil` when there was nothing to delete.
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
}
