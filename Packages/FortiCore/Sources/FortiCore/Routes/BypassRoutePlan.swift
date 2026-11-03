import FortiShared

/// How the primary network service's additional routes follow the exception rules (DESIGN §5.8).
/// Pure: the app reads the routes through the helper, plans, then writes the config and the routes.
///
/// The rules apply only while this Mac's address is inside one of `networks`; elsewhere everything
/// goes through the VPN. A route on the service is the app's own when the app wrote it before
/// (`previouslyApplied`, destination and gateway) or, while the rules apply, when it is a listed
/// target through the current router outside this Mac's own network. The app's own routes are
/// rewritten from the list (or removed). While the rules apply, other routes through the router were
/// added outside the app and join the list, unless they overlap a listed target or a wider route
/// joining with them, lie inside this Mac's own network, or the list is full. Every remaining route (other
/// gateways, routes on this Mac's own network the app never wrote, anything on a network where the
/// rules do not apply) is kept exactly as it is.
public enum BypassRoutePlan {
    public struct Outcome: Sendable, Equatable {
        /// The entry of `networks` this Mac is in; `nil` when the rules do not apply here.
        public var matchedNetwork: IPv4Network?
        /// Targets to append to the list (added outside the app).
        public var imported: [IPv4Network]
        /// Routes added outside the app that were not imported because the list is full; they are kept.
        public var notImported: Int
        /// Listed targets inside this Mac's own network: reached directly, so no route is written.
        public var onLink: [IPv4Network]
        /// Routes left untouched.
        public var kept: [AdditionalRoute]
        /// How many of `kept` go through another gateway than the router.
        public var keptThroughOtherGateways: Int
        /// The app's own routes after this plan (to record as applied).
        public var applied: [AdditionalRoute]
        /// The complete list the service should have: `applied`, then `kept`.
        public var routes: [AdditionalRoute]
        /// Whether `routes` differs from what the service has now.
        public var needsWrite: Bool
    }

    /// - Parameters:
    ///   - networks: `routes.networks`.
    ///   - targets: `routes.bypass_vpn`.
    ///   - previouslyApplied: the routes the app last wrote for its list on this service.
    ///   - snapshot: the service's routes, router and this Mac's address now.
    public static func plan(
        networks: [IPv4Network],
        targets: [IPv4Network],
        previouslyApplied: [AdditionalRoute],
        snapshot: BypassRoutesSnapshot
    ) -> Outcome {
        let router = snapshot.router
        let address = IPv4Network.parseDottedQuad(Substring(snapshot.address))
        let matchedNetwork = address.flatMap { address in networks.first { $0.contains(address) } }
        let active = matchedNetwork != nil
        let localNetwork = snapshot.localNetwork

        /// A route through the router the rules may own: a valid target outside this Mac's network.
        func candidate(_ route: AdditionalRoute) -> IPv4Network? {
            guard active, route.gateway == router, !previouslyApplied.contains(route),
                  let network = route.network, localNetwork?.contains(network) != true
            else { return nil }
            return network
        }

        var imported: [IPv4Network] = []
        var notImported = 0
        let room = max(0, IPv4Network.maxCount - targets.count)
        // Widest first (the sort is stable), so of two overlapping routes the wider one joins the
        // list whatever their order on the service.
        for network in snapshot.routes.compactMap(candidate).sorted(by: { $0.prefixLength < $1.prefixLength }) {
            if (targets + imported).contains(where: { $0.overlaps(network) }) { continue }
            if imported.count < room {
                imported.append(network)
            } else {
                notImported += 1
            }
        }

        var kept: [AdditionalRoute] = []
        for route in snapshot.routes where !previouslyApplied.contains(route) {
            if let network = candidate(route), targets.contains(network) || imported.contains(network) { continue }
            if !kept.contains(route) { kept.append(route) }
        }

        var onLink: [IPv4Network] = []
        var applied: [AdditionalRoute] = []
        if active {
            for target in targets + imported {
                if let localNetwork, localNetwork.contains(target) {
                    onLink.append(target)
                } else {
                    applied.append(AdditionalRoute(target, gateway: router))
                }
            }
        }
        let routes = applied + kept
        let needsWrite = routes.count != snapshot.routes.count || Set(routes) != Set(snapshot.routes)
        return Outcome(
            matchedNetwork: matchedNetwork, imported: imported, notImported: notImported, onLink: onLink,
            kept: kept, keptThroughOtherGateways: kept.filter { $0.gateway != router }.count,
            applied: applied, routes: routes, needsWrite: needsWrite)
    }
}
