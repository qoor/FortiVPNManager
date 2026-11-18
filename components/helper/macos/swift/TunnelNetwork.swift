import Darwin
import Dispatch
import Foundation
import SystemConfiguration

enum TunnelNetwork {
    static let serviceID = FortiIdentifiers.helperBundleID + ".VPN"

    static func change(_ change: HelperInternal.NetworkChange) -> HelperInternal.NetworkOutcome {
        dispatchPrecondition(condition: .onQueue(BypassRoutes.queue))
        switch change {
        case .lookUp(_, let outside):
            return lookUp(outside)
        case .apply(let settings, let previous, let remove, let add):
            return apply(settings, previous: previous, remove: remove, add: add, routesAndDNS: true)
        case .prepare(let settings, let add):
            return apply(settings, previous: nil, remove: [], add: add, routesAndDNS: false)
        case .switch(let settings, let replaced):
            return switchTraffic(to: settings, from: replaced)
        case .revert(let changes):
            return revert(changes)
        }
    }

    private static func lookUp(_ outside: [HelperInternal.IPNetwork]) -> HelperInternal.NetworkOutcome {
        guard let store = SCDynamicStoreCreate(nil, FortiIdentifiers.helperBundleID as CFString, nil, nil),
              let routers = primaryRouters(store)
        else {
            return failed(.network, "no active network service", left: [])
        }
        let routes = outside.compactMap { destination -> HelperInternal.OutsideRoute? in
            guard let router = routers.router(for: destination) else { return nil }
            return HelperInternal.OutsideRoute(destination: destination, router: router, serviceID: routers.serviceID)
        }
        guard let table = readTable(Set(routes.compactMap { systemRoute($0, store)?.family })) else {
            return failed(.network, "the routing table could not be read", left: [])
        }
        let missing = routes.filter { isIn(table, $0, store) != true }
        return .lookedUp(dnsServiceID: serviceID, missing: missing)
    }

    private static func apply(
        _ settings: HelperInternal.NetworkSettings, previous: HelperInternal.NetworkSettings?,
        remove: [HelperInternal.OutsideRoute], add: [HelperInternal.OutsideRoute], routesAndDNS: Bool
    ) -> HelperInternal.NetworkOutcome {
        guard let store = SCDynamicStoreCreate(nil, FortiIdentifiers.helperBundleID as CFString, nil, nil) else {
            return failed(.network, "the network configuration could not be read", left: remove)
        }
        if !add.isEmpty {
            guard let routers = primaryRouters(store) else {
                return failed(.network, "no active network service", left: remove)
            }
            guard add.allSatisfy({ $0.serviceID == routers.serviceID && routers.router(for: $0.destination) == $0.router })
            else {
                return failed(.network, "the network changed since it was looked up; look it up again", left: remove)
            }
        }
        let wanted = routesAndDNS ? tunnelRoutes(settings) : []
        let stale = previous.map { tunnelRoutes($0).filter { !wanted.contains($0) } } ?? []
        let families = Set((wanted + stale).map(\.family) + (remove + add).compactMap { systemRoute($0, store)?.family })
        guard let before = readTable(families) else {
            return failed(.network, "the routing table could not be read", left: remove)
        }
        let name = settings.tunnel.name
        var tried: [HelperInternal.OutsideRoute] = []
        func finish(
            _ failure: (step: HelperInternal.NetworkStep, error: String)?, table: [RouteTable.Row]?
        ) -> HelperInternal.NetworkOutcome {
            let added = tried.filter { isIn(table, $0, store) ?? true }
            let left = remove.filter { isIn(table, $0, store) ?? true }
            guard let failure else {
                HelperLog.routes.notice("applied the VPN network settings to \(name, privacy: .public)")
                return .applied(added: added, left: left)
            }
            HelperLog.routes.error("applying the VPN network settings to \(name, privacy: .public) failed at \(failure.step.rawValue, privacy: .public): \(failure.error, privacy: .public)")
            return .failed(step: failure.step, error: failure.error, added: added, left: left)
        }

        for route in stale where before.contains(where: route.matches) {
            RouteTable.delete(route)
        }
        if let previous {
            for arguments in TunnelInterface.removals(from: previous, to: settings) {
                TunnelInterface.change(name, arguments)
            }
        }
        for route in remove {
            if let system = systemRoute(route, store), before.contains(where: system.matches) {
                RouteTable.delete(system)
            }
        }

        guard let current = TunnelInterface.read(name) else {
            return finish((.addresses, "the tunnel \(name) could not be read"), table: readTable(families))
        }
        for (step, arguments) in TunnelInterface.changes(to: settings, from: current) {
            guard TunnelInterface.change(name, arguments) else {
                return finish((step, "ifconfig \(name) \(arguments.joined(separator: " ")) failed"), table: readTable(families))
            }
        }
        for route in add {
            guard let system = systemRoute(route, store) else {
                return finish((.outsideRoutes, "the network service \(route.serviceID) has no IPv6 interface"), table: readTable(families))
            }
            guard !before.contains(where: system.matches), !tried.contains(route) else { continue }
            tried.append(route)
            guard RouteTable.add(system, replacingCopiesIn: before) else {
                return finish((.outsideRoutes, "route -n \(system.addArguments.joined(separator: " ")) failed"), table: readTable(families))
            }
        }
        for route in wanted where !before.contains(where: route.matches) {
            guard RouteTable.add(route, replacingCopiesIn: before) else {
                return finish((.routes, "route -n \(route.addArguments.joined(separator: " ")) failed"), table: readTable(families))
            }
        }
        guard !routesAndDNS || writeDNS(settings, store) else {
            return finish((.dns, "the DNS settings could not be written"), table: readTable(families))
        }

        guard let interface = TunnelInterface.read(name) else {
            return finish((.addresses, "the tunnel \(name) could not be read"), table: readTable(families))
        }
        if let step = interface.mismatch(with: settings, previous: previous) {
            let what = step == .mtu ? "MTU" : "addresses"
            return finish((step, "the tunnel \(name) does not have its \(what) after ifconfig"), table: readTable(families))
        }
        guard let after = readTable(families) else {
            return finish((.outsideRoutes, "the routing table could not be read"), table: nil)
        }
        let missing = add.filter { isIn(after, $0, store) != true }
        guard missing.isEmpty else {
            let routes = missing.map { "\($0.destination.address)/\($0.destination.prefixLength)" }
            return finish((.outsideRoutes, "the routing table lacks \(routes.joined(separator: ", "))"), table: after)
        }
        let wrong = wanted.filter { route in !after.contains(where: route.matches) }
            + stale.filter { route in after.contains(where: route.matches) }
        guard wrong.isEmpty else {
            let routes = wrong.map { "\($0.destination)/\($0.prefixLength) through \($0.gateway)" }
            return finish((.routes, "the routing table lacks or still has \(routes.joined(separator: ", "))"), table: after)
        }
        guard !routesAndDNS || hasDNS(settings, store) else {
            return finish((.dns, "the DNS settings were not kept"), table: after)
        }
        return finish(nil, table: after)
    }

    private static func switchTraffic(
        to settings: HelperInternal.NetworkSettings, from replaced: HelperInternal.NetworkSettings
    ) -> HelperInternal.NetworkOutcome {
        guard let store = SCDynamicStoreCreate(nil, FortiIdentifiers.helperBundleID as CFString, nil, nil) else {
            return failed(.network, "the network configuration could not be read", left: [])
        }
        let wanted = tunnelRoutes(settings)
        let old = tunnelRoutes(replaced)
        let families = Set((wanted + old).map(\.family))
        guard let before = readTable(families) else {
            return failed(.routes, "the routing table could not be read", left: [])
        }
        for step in switchSteps(wanted: wanted, replaced: old, table: before) {
            let done = switch step {
            case .move(let route): RouteTable.change(route.changeArguments)
            case .add(let route): RouteTable.add(route, replacingCopiesIn: before)
            case .delete(let route): RouteTable.delete(route)
            }
            guard done else {
                return failed(.routes, "route -n \(step.arguments.joined(separator: " ")) failed", left: [])
            }
        }
        guard writeDNS(settings, store) else {
            return failed(.dns, "the DNS settings could not be written", left: [])
        }
        guard let after = readTable(families) else {
            return failed(.routes, "the routing table could not be read", left: [])
        }
        let wrong = wanted.filter { route in !after.contains(where: route.matches) }
            + old.filter { route in !wanted.contains(route) && after.contains(where: route.matches) }
        guard wrong.isEmpty else {
            let routes = wrong.map { "\($0.destination)/\($0.prefixLength) through \($0.gateway)" }
            return failed(.routes, "the routing table lacks or still has \(routes.joined(separator: ", "))", left: [])
        }
        guard hasDNS(settings, store) else {
            return failed(.dns, "the DNS settings were not kept", left: [])
        }
        HelperLog.routes.notice("moved the VPN traffic from \(replaced.tunnel.name, privacy: .public) to \(settings.tunnel.name, privacy: .public)")
        return .applied(added: [], left: [])
    }

    enum SwitchStep: Equatable {
        case move(RouteTable.Route)
        case add(RouteTable.Route)
        case delete(RouteTable.Route)

        var arguments: [String] {
            switch self {
            case .move(let route): route.changeArguments
            case .add(let route): route.addArguments
            case .delete(let route): route.deleteArguments
            }
        }
    }

    static func switchSteps(
        wanted: [RouteTable.Route], replaced: [RouteTable.Route], table: [RouteTable.Row]
    ) -> [SwitchStep] {
        let sameDestination = { (one: RouteTable.Route, other: RouteTable.Route) in
            one.family == other.family && one.destination == other.destination && one.prefixLength == other.prefixLength
        }
        let present = replaced.filter { route in table.contains(where: route.matches) }
        let moves = wanted.filter { route in !table.contains(where: route.matches) }.map { route in
            present.contains { sameDestination($0, route) } ? SwitchStep.move(route) : .add(route)
        }
        let deletes = present.filter { route in !wanted.contains { sameDestination($0, route) } }.map(SwitchStep.delete)
        return moves + deletes
    }

    private static func revert(_ changes: HelperInternal.OutsideChanges) -> HelperInternal.NetworkOutcome {
        guard let store = SCDynamicStoreCreate(nil, FortiIdentifiers.helperBundleID as CFString, nil, nil) else {
            return failed(.dns, "the network configuration could not be read", left: changes.routes)
        }
        let keys = [kSCEntNetDNS, kSCEntNetIPv4].map { stateKey(changes.dnsServiceID, $0) }
        for key in keys {
            SCDynamicStoreRemoveValue(store, key)
        }
        let families = Set(changes.routes.compactMap { systemRoute($0, store)?.family })
        let before = readTable(families)
        var changed = false
        for route in changes.routes {
            if let system = systemRoute(route, store), before?.contains(where: system.matches) == true {
                RouteTable.delete(system)
                changed = true
            }
        }
        let after = changed ? readTable(families) : before
        let left = changes.routes.filter { isIn(after, $0, store) ?? true }
        guard keys.allSatisfy({ SCDynamicStoreCopyValue(store, $0) == nil }) else {
            return failed(.dns, "the DNS settings under \(changes.dnsServiceID) could not be removed", left: left)
        }
        guard after != nil else {
            return failed(.outsideRoutes, "the routing table could not be read", left: left)
        }
        HelperLog.routes.notice("reverted the VPN network settings; \(left.count) route(s) left")
        return .reverted(left: left)
    }

    private static func failed(
        _ step: HelperInternal.NetworkStep, _ error: String, left: [HelperInternal.OutsideRoute]
    ) -> HelperInternal.NetworkOutcome {
        HelperLog.routes.error("the VPN network change failed at \(step.rawValue, privacy: .public): \(error, privacy: .public)")
        return .failed(step: step, error: error, added: [], left: left)
    }

    private struct Routers {
        var serviceID: String
        var ipv4: String
        var ipv6: String?

        func router(for destination: HelperInternal.IPNetwork) -> String? {
            IPv4Network.parseDottedQuad(Substring(destination.address)) != nil ? ipv4 : ipv6
        }
    }

    private static func primaryRouters(_ store: SCDynamicStore) -> Routers? {
        guard let serviceID = BypassRoutes.primaryServiceID(in: store),
              let ipv4 = serviceState(store, serviceID, kSCEntNetIPv4)?[kSCPropNetIPv4Router as String] as? String,
              IPv4Network.parseDottedQuad(Substring(ipv4)) != nil
        else { return nil }
        let ipv6 = serviceState(store, serviceID, kSCEntNetIPv6)?[kSCPropNetIPv6Router as String] as? String
        return Routers(serviceID: serviceID, ipv4: ipv4, ipv6: ipv6.flatMap { RouteTable.canonicalIPv6($0) })
    }

    private static func serviceState(
        _ store: SCDynamicStore, _ serviceID: String, _ entity: CFString
    ) -> [String: Any]? {
        SCDynamicStoreCopyValue(store, stateKey(serviceID, entity)) as? [String: Any]
    }

    private static func stateKey(_ serviceID: String, _ entity: CFString) -> CFString {
        SCDynamicStoreKeyCreateNetworkServiceEntity(nil, kSCDynamicStoreDomainState, serviceID as CFString, entity)
    }

    private static func readTable(_ families: Set<RouteTable.Family>) -> [RouteTable.Row]? {
        var rows: [RouteTable.Row] = []
        for family in [RouteTable.Family.inet, .inet6] where families.contains(family) {
            guard let read = RouteTable.read(family) else { return nil }
            rows += read
        }
        return rows
    }

    private static func isIn(
        _ table: [RouteTable.Row]?, _ route: HelperInternal.OutsideRoute, _ store: SCDynamicStore
    ) -> Bool? {
        guard let table, let system = systemRoute(route, store) else { return nil }
        return table.contains(where: system.matches)
    }

    private static func systemRoute(_ route: HelperInternal.OutsideRoute, _ store: SCDynamicStore) -> RouteTable.Route? {
        guard isLinkLocal(route.router) else {
            return tableRoute(route.destination, gateway: route.router)
        }
        guard let interface = serviceState(store, route.serviceID, kSCEntNetIPv6)?[kSCPropInterfaceName as String] as? String else {
            return nil
        }
        return tableRoute(route.destination, gateway: "\(route.router)%\(interface)")
    }

    static func isLinkLocal(_ address: String) -> Bool {
        var bytes = in6_addr()
        guard inet_pton(AF_INET6, address, &bytes) == 1 else { return false }
        return bytes.__u6_addr.__u6_addr8.0 == 0xfe && bytes.__u6_addr.__u6_addr8.1 & 0xc0 == 0x80
    }

    static func tunnelRoutes(_ settings: HelperInternal.NetworkSettings) -> [RouteTable.Route] {
        let ipv4 = settings.ipv4.address
        guard let address = IPv4Network.parseDottedQuad(Substring(ipv4.address)) else {
            fatalError("the Helper entry sent \(ipv4.address) as the tunnel's IPv4 address")
        }
        var routes: [RouteTable.Route] = []
        if ipv4.prefixLength < 32 {
            let network = address & UInt32.max << (32 - ipv4.prefixLength)
            let destination = "\(network >> 24).\(network >> 16 & 0xff).\(network >> 8 & 0xff).\(network & 0xff)"
            routes.append(tableRoute(
                HelperInternal.IPNetwork(address: destination, prefixLength: ipv4.prefixLength), gateway: ipv4.address
            ))
        }
        routes += destinations(settings.ipv4.routes, full: ["0.0.0.0", "128.0.0.0"]).map {
            tableRoute($0, gateway: ipv4.address)
        }
        if let ipv6 = settings.ipv6 {
            routes += destinations(ipv6.routes, full: ["::", "8000::"]).map {
                tableRoute($0, gateway: ipv6.address.address)
            }
        }
        var unique: [RouteTable.Route] = []
        for route in routes where !unique.contains(route) {
            unique.append(route)
        }
        return unique
    }

    private static func destinations(
        _ routes: HelperInternal.TunnelRoutes, full: [String]
    ) -> [HelperInternal.IPNetwork] {
        switch routes {
        case .full: full.map { HelperInternal.IPNetwork(address: $0, prefixLength: 1) }
        case .split(let include): include
        }
    }

    private static func tableRoute(_ destination: HelperInternal.IPNetwork, gateway: String) -> RouteTable.Route {
        guard let route = RouteTable.Route(
            destination: destination.address, prefixLength: destination.prefixLength, gateway: gateway
        ) else {
            fatalError("the Helper entry sent \(destination.address)/\(destination.prefixLength) through \(gateway), which is not a route")
        }
        return route
    }

    static func dnsEntries(_ settings: HelperInternal.NetworkSettings) -> (dns: [String: Any]?, ipv4: [String: Any]?) {
        guard !settings.dnsServers.isEmpty else { return (nil, nil) }
        var dns: [String: Any] = [kSCPropNetDNSServerAddresses as String: settings.dnsServers]
        if !settings.defaultDomains.isEmpty {
            dns[kSCPropNetDNSSearchDomains as String] = settings.defaultDomains
        }
        var matchDomains: [String] = []
        for domain in settings.defaultDomains + settings.splitDnsDomains where !matchDomains.contains(domain) {
            matchDomains.append(domain)
        }
        if !matchDomains.isEmpty {
            dns[kSCPropNetDNSSupplementalMatchDomains as String] = matchDomains
        }
        guard case .full = settings.ipv4.routes else { return (dns, nil) }
        let address = settings.ipv4.address.address
        return (dns, [
            kSCPropNetIPv4Addresses as String: [address],
            kSCPropNetIPv4SubnetMasks as String: ["255.255.255.255"],
            kSCPropNetIPv4Router as String: address,
            kSCPropInterfaceName as String: settings.tunnel.name,
            kSCPropNetOverridePrimary as String: 1,
        ])
    }

    private static func writeDNS(_ settings: HelperInternal.NetworkSettings, _ store: SCDynamicStore) -> Bool {
        let entries = dnsEntries(settings)
        for (entity, value) in [(kSCEntNetDNS, entries.dns), (kSCEntNetIPv4, entries.ipv4)] {
            let key = stateKey(serviceID, entity)
            if let value {
                guard SCDynamicStoreSetValue(store, key, value as CFDictionary) else { return false }
            } else {
                SCDynamicStoreRemoveValue(store, key)
            }
        }
        return true
    }

    private static func hasDNS(_ settings: HelperInternal.NetworkSettings, _ store: SCDynamicStore) -> Bool {
        let entries = dnsEntries(settings)
        return [(kSCEntNetDNS, entries.dns), (kSCEntNetIPv4, entries.ipv4)].allSatisfy { entity, value in
            SCDynamicStoreCopyValue(store, stateKey(serviceID, entity)) as? NSDictionary == value.map { $0 as NSDictionary }
        }
    }
}
