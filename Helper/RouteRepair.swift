import Foundation
import FortiHelperCore
import SystemConfiguration

/// Repairs the IPv4 routing table after openfortivpn exited without restoring it (DESIGN §3.8,
/// §5.2 "RouteRepair"): restores the default route via the primary service's router and removes
/// the gateway host route openfortivpn added. Every address handed to `/sbin/route` is validated
/// as dotted-quad IPv4 first.
enum RouteRepair {
    static func repair(gatewayIP: String?) -> RouteRepairOutcome {
        var outcome = RouteRepairOutcome.notNeeded
        let defaultRoute = RouteTable.lookup("default")
        if defaultRoute == nil || defaultRoute?.interface?.hasPrefix("ppp") == true {
            outcome = outcome.combined(with: restoreDefaultRoute(replacing: defaultRoute))
        }
        if let gatewayIP, let removed = removeHostRoute(to: gatewayIP) {
            outcome = outcome.combined(with: removed ? .repaired : .failed)
        }
        return outcome
    }

    private static func restoreDefaultRoute(replacing current: RouteTable.Entry?) -> RouteRepairOutcome {
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
        // /sbin/route reports kernel errors only as text, so judge by the resulting table.
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
    /// openfortivpn), if it is still there. `nil` when there was nothing to delete.
    private static func removeHostRoute(to gateway: String) -> Bool? {
        guard IPv4.isAddress(gateway),
              let entry = RouteTable.lookup(gateway),
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
        guard let store = SCDynamicStoreCreate(nil, "com.qoor.FortiVPNManager.Helper" as CFString, nil, nil),
              let global = SCDynamicStoreCopyValue(store, "State:/Network/Global/IPv4" as CFString) as? [String: Any],
              let router = global[kSCPropNetIPv4Router as String] as? String,
              let interface = global[kSCDynamicStorePropNetPrimaryInterface as String] as? String,
              IPv4.isAddress(router),
              !interface.hasPrefix("ppp")
        else { return nil }
        return (router, interface)
    }
}

/// `/sbin/route -n` lookups and changes.
enum RouteTable {
    struct Entry: Equatable, Sendable {
        var destination: String
        var gateway: String?
        var interface: String?
        /// `UP`, `GATEWAY`, `HOST`, `STATIC`, ...
        var flags: Set<String>
    }

    static let routePath = "/sbin/route"

    /// The route `route -n get` reports for `destination`, or `nil` if there is none. A missing
    /// route ("not in table") still exits with status 0, so the output decides.
    static func lookup(_ destination: String) -> Entry? {
        let result = SystemCommand.run(routePath, ["-n", "get", destination])
        guard result.succeeded else { return nil }
        return parse(result.output)
    }

    /// Runs `route -n <arguments>`. macOS route exits 0 even when the kernel rejects the change and
    /// only prints "route: writing to routing socket: …" to stderr, so stderr output counts as failure.
    /// Callers that need certainty re-check the table with `lookup`.
    @discardableResult
    static func change(_ arguments: [String]) -> Bool {
        let result = SystemCommand.run(routePath, ["-n"] + arguments)
        let succeeded = result.succeeded && result.errorOutput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        if !succeeded {
            let command = arguments.joined(separator: " ")
            let message = (result.errorOutput + result.output).trimmingCharacters(in: .whitespacesAndNewlines)
            HelperLog.routes.error("route -n \(command, privacy: .public) failed (\(String(describing: result.status), privacy: .public)): \(message, privacy: .public)")
        }
        return succeeded
    }

    /// Parses the `key: value` lines of `route -n get`.
    static func parse(_ output: String) -> Entry? {
        var fields: [String: String] = [:]
        for line in output.split(whereSeparator: \.isNewline) {
            guard let colon = line.firstIndex(of: ":") else { continue }
            let key = line[..<colon].trimmingCharacters(in: .whitespaces)
            fields[key] = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
        }
        guard let destination = fields["destination"], !destination.isEmpty else { return nil }
        let flags = fields["flags"].map { value in
            Set(value.trimmingCharacters(in: CharacterSet(charactersIn: "<>")).split(separator: ",").map(String.init))
        } ?? []
        return Entry(destination: destination, gateway: fields["gateway"], interface: fields["interface"], flags: flags)
    }
}

private extension RouteRepairOutcome {
    /// A failure wins over a repair, which wins over "nothing to do".
    func combined(with other: RouteRepairOutcome) -> RouteRepairOutcome {
        if self == .failed || other == .failed { return .failed }
        if self == .repaired || other == .repaired { return .repaired }
        return .notNeeded
    }
}
