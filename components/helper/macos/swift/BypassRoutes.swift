import Dispatch
import SystemConfiguration

/// Changes the exception routes in the routing table as the Helper entry asks, through `RouteTable`,
/// and reads the primary network service they go through. The routes are static routes of the
/// running system, which a restart forgets; only routes such as the Helper adds count as its own.
enum BypassRoutes {
    /// Every change runs here, one at a time and off the entry's reader thread, so tunnel control
    /// never waits behind `/sbin/route`.
    static let queue = DispatchQueue(label: "com.qoor.FortiVPNManager.Helper.routes")

    /// Carries out `change`, as `HelperInternal.BypassRoutesChange` describes it: one read of the
    /// table before the changes and, if there were any, one after them. Call on ``queue``.
    static func change(_ change: HelperInternal.BypassRoutesChange) -> HelperInternal.BypassRoutesOutcome {
        dispatchPrecondition(condition: .onQueue(queue))
        var network: BypassRoutesSnapshot?
        switch change.network {
        case .skip:
            break
        case .read:
            switch primaryNetwork() {
            case .failure(let error): return .refused(error: error)
            case .success(let read): network = read
            }
        case .expect(let expected):
            switch primaryNetwork() {
            case .failure(let error): return .refused(error: error)
            case .success(let read) where read != expected:
                return .refused(error: HelperError(.wrongPhase, "the network changed since it was read; read it again"))
            case .success(let read): network = read
            }
        }
        if !change.add.isEmpty {
            guard let network, change.add.count <= IPv4Network.maxCount, Set(change.add).count == change.add.count,
                  change.add.allSatisfy({ $0.gateway == network.router && $0.network != nil })
            else {
                return .refused(error: HelperError(
                    .invalidRequest, "the route list has repeats, too many routes, or routes this helper does not write"
                ))
            }
        }
        guard let before = RouteTable.read() else {
            return .refused(error: HelperError(.internalError, "the routing table could not be read"))
        }
        var changed = false
        var removed = 0
        for added in change.remove {
            guard let route = RouteTable.Route(added.route), before.contains(where: route.matches) else { continue }
            changed = true
            if RouteTable.delete(route) { removed += 1 }
        }
        var tried: [BypassRoute] = []
        for wanted in change.add {
            guard let route = RouteTable.Route(wanted), !before.contains(where: route.matches) else { continue }
            changed = true
            // A copy of the default route the system made for this host, for every interface, makes
            // the add fail; the system makes another when it needs one.
            for copy in before where copy.flags.contains("W") && !copy.flags.contains("I")
                && copy.destination == route.destination && copy.prefixLength == route.prefixLength {
                if let copy = RouteTable.Route(destination: copy.destination, subnetMask: route.subnetMask, gateway: copy.gateway) {
                    RouteTable.delete(copy)
                }
            }
            RouteTable.add(route)
            tried.append(wanted)
        }
        let after = changed ? RouteTable.read() : before
        // `nil` when the table could not be read again.
        let inTable = { (route: BypassRoute) -> Bool? in
            guard let route = RouteTable.Route(route) else { return false }
            return after.map { $0.contains(where: route.matches) }
        }
        // Unread, the routes tried count as added and those to remove as left, so that the Helper
        // removes them later rather than leaving them behind.
        let added = tried.filter { inTable($0) ?? true }
        if removed > 0 || !added.isEmpty {
            HelperLog.routes.notice("removed \(removed) and added \(added.count) exception route(s)")
        }
        return .changed(
            network: network,
            added: added,
            found: network == nil ? [] : change.lookUp.filter { inTable($0) ?? false },
            left: change.remove.filter { inTable($0.route) ?? true }
        )
    }

    /// The service of `State:/Network/Global/IPv4` with its IPv4 router, this Mac's first IPv4 address
    /// and mask on it, and its name in the network settings. A tunnel interface never counts as primary.
    private static func primaryNetwork() -> Result<BypassRoutesSnapshot, HelperError> {
        guard let store = SCDynamicStoreCreate(nil, FortiIdentifiers.helperBundleID as CFString, nil, nil),
              let global = SCDynamicStoreCopyValue(store, "State:/Network/Global/IPv4" as CFString) as? [String: Any],
              let serviceID = global[kSCDynamicStorePropNetPrimaryService as String] as? String,
              let interface = global[kSCDynamicStorePropNetPrimaryInterface as String] as? String,
              !interface.hasPrefix("ppp"), !interface.hasPrefix("utun")
        else {
            return .failure(HelperError(.wrongPhase, "no active network service"))
        }
        let serviceKey = "State:/Network/Service/\(serviceID)/IPv4" as CFString
        guard let router = global[kSCPropNetIPv4Router as String] as? String,
              IPv4Network.parseDottedQuad(Substring(router)) != nil,
              let state = SCDynamicStoreCopyValue(store, serviceKey) as? [String: Any],
              let address = (state[kSCPropNetIPv4Addresses as String] as? [String])?.first,
              let subnetMask = (state[kSCPropNetIPv4SubnetMasks as String] as? [String])?.first,
              IPv4Network.parseDottedQuad(Substring(address)) != nil,
              IPv4Network.parseDottedQuad(Substring(subnetMask)) != nil
        else {
            return .failure(HelperError(.wrongPhase, "the active network service has no IPv4 router or address"))
        }
        guard let preferences = SCPreferencesCreate(nil, FortiIdentifiers.helperBundleID as CFString, nil),
              let service = SCNetworkServiceCopy(preferences, serviceID as CFString),
              let name = SCNetworkServiceGetName(service) as String?, !name.isEmpty
        else {
            return .failure(HelperError(.internalError, "the active network service \(serviceID) has no name"))
        }
        return .success(BypassRoutesSnapshot(
            serviceID: serviceID, serviceName: name, router: router, address: address, subnetMask: subnetMask
        ))
    }
}
