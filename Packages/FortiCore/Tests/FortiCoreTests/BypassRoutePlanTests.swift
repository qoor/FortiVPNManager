import Foundation
import FortiShared
import Testing
@testable import FortiCore

@Suite struct BypassRoutePlanTests {
    let router = "10.20.161.254"
    let office = try! IPv4Network(parsing: "10.20.161.0/24")
    let servers = try! IPv4Network(parsing: "10.30.161.0/24")
    let host = try! IPv4Network(parsing: "10.30.161.16")
    let network = try! IPv4Network(parsing: "10.30.170.0/24")
    let other = try! IPv4Network(parsing: "10.50.100.10")

    /// The Mac at the office desk (10.20.161.20/24) unless told otherwise.
    private func snapshot(
        _ routes: [AdditionalRoute], router: String? = nil, address: String = "10.20.161.20",
        subnetMask: String = "255.255.255.0"
    ) -> BypassRoutesSnapshot {
        BypassRoutesSnapshot(
            serviceID: "SERVICE-1", serviceName: "Ethernet", router: router ?? self.router,
            address: address, subnetMask: subnetMask, routes: routes)
    }

    private func via(_ network: IPv4Network, _ gateway: String? = nil) -> AdditionalRoute {
        AdditionalRoute(network, gateway: gateway ?? router)
    }

    private func plan(
        networks: [IPv4Network]? = nil, targets: [IPv4Network], applied: [AdditionalRoute] = [],
        _ snapshot: BypassRoutesSnapshot
    ) -> BypassRoutePlan.Outcome {
        BypassRoutePlan.plan(
            networks: networks ?? [office], targets: targets, previouslyApplied: applied, snapshot: snapshot)
    }

    // MARK: - Where the rules apply

    @Test func alreadyAppliedNeedsNoWrite() {
        let outcome = plan(targets: [host, network], applied: [via(host), via(network)], snapshot([via(network), via(host)]))
        #expect(outcome.matchedNetwork == office)
        #expect(!outcome.needsWrite)
        #expect(outcome.imported.isEmpty)
        #expect(outcome.applied == [via(host), via(network)])
    }

    @Test func aReformattedMacGetsTheList() {
        let outcome = plan(targets: [host, network], snapshot([]))
        #expect(outcome.needsWrite)
        #expect(outcome.routes == [via(host), via(network)])
    }

    @Test func outsideTheNetworksTheAppsRoutesGoAndTheRestStays() {
        let homeRouter = "192.168.0.1"
        let homeRoute = via(other, homeRouter)
        let listedThroughHomeRouter = via(network, homeRouter)
        let outcome = plan(
            targets: [host, network], applied: [via(host)],
            snapshot([via(host), homeRoute, listedThroughHomeRouter], router: homeRouter, address: "192.168.0.12"))
        #expect(outcome.matchedNetwork == nil)
        #expect(outcome.imported.isEmpty)
        #expect(outcome.applied.isEmpty)
        #expect(outcome.routes == [homeRoute, listedThroughHomeRouter])
        #expect(outcome.needsWrite)
    }

    @Test func withoutNetworksTheRulesApplyNowhere() {
        let outcome = plan(networks: [], targets: [host], applied: [via(host)], snapshot([via(host)]))
        #expect(outcome.matchedNetwork == nil)
        #expect(outcome.routes.isEmpty)
        #expect(outcome.needsWrite)
    }

    @Test func anotherListedNetworkUsesItsOwnRouterAndSkipsTargetsOnTheSameLAN() {
        let lanRouter = "10.30.161.1"
        let outcome = plan(
            networks: [office, servers], targets: [host, network],
            snapshot([], router: lanRouter, address: "10.30.161.50"))
        #expect(outcome.matchedNetwork == servers)
        #expect(outcome.onLink == [host])
        #expect(outcome.routes == [via(network, lanRouter)])
    }

    @Test func aTargetThatBecameOnLinkIsRemoved() {
        let lanRouter = "10.30.161.1"
        let outcome = plan(
            networks: [servers], targets: [host], applied: [via(host, lanRouter)],
            snapshot([via(host, lanRouter)], router: lanRouter, address: "10.30.161.50"))
        #expect(outcome.onLink == [host])
        #expect(outcome.routes.isEmpty)
        #expect(outcome.needsWrite)
    }

    @Test func aRouteOnThisMacsNetworkTheAppNeverWroteIsKept() {
        // An admin's VLAN behind the router, more specific than the Mac's /16.
        let vlan = AdditionalRoute(destination: "10.20.200.0", subnetMask: "255.255.255.0", gateway: router)
        let outcome = plan(targets: [host], snapshot([vlan], subnetMask: "255.255.0.0"))
        #expect(outcome.imported.isEmpty)
        #expect(outcome.applied == [via(host)])
        #expect(outcome.routes == [via(host), vlan])
        #expect(outcome.keptThroughOtherGateways == 0)
    }

    @Test func aListedTargetOnThisMacsNetworkKeepsARouteTheAppNeverWrote() {
        let deskNeighbour = try! IPv4Network(parsing: "10.20.161.16")
        let outcome = plan(targets: [deskNeighbour], snapshot([via(deskNeighbour)]))
        #expect(outcome.onLink == [deskNeighbour])
        #expect(outcome.applied.isEmpty)
        #expect(outcome.routes == [via(deskNeighbour)])
        #expect(!outcome.needsWrite)
    }

    // MARK: - The list and the service

    @Test func removedFromTheListIsDeleted() {
        let outcome = plan(targets: [host], applied: [via(host), via(network)], snapshot([via(host), via(network)]))
        #expect(outcome.needsWrite)
        #expect(outcome.imported.isEmpty)
        #expect(outcome.routes == [via(host)])
        #expect(outcome.applied == [via(host)])
    }

    @Test func addedOutsideTheAppThroughTheRouterJoinsTheList() {
        let outcome = plan(targets: [host], applied: [via(host)], snapshot([via(host), via(other)]))
        #expect(outcome.imported == [other])
        #expect(!outcome.needsWrite)
        #expect(outcome.applied == [via(host), via(other)])
    }

    @Test func firstAdoptionImportsWhatIsThereOnce() {
        let outcome = plan(targets: [], snapshot([via(host), via(host)]))
        #expect(outcome.imported == [host])
        #expect(outcome.needsWrite)  // the repeat goes away
        #expect(outcome.routes == [via(host)])
    }

    @Test func otherGatewaysAreKeptWhateverTheOrder() {
        let foreign = via(other, "192.168.0.1")
        let wideRoute = AdditionalRoute(destination: "16.0.0.0", subnetMask: "240.0.0.0", gateway: router)
        for routes in [[via(other), foreign, wideRoute], [foreign, wideRoute, via(other)]] {
            let outcome = plan(targets: [host], snapshot(routes))
            #expect(outcome.imported == [other])
            #expect(Set(outcome.applied) == [via(host), via(other)])
            #expect(Set(outcome.routes) == [via(host), via(other), foreign, wideRoute])
        }
    }

    @Test func routesOverlappingATargetAreNotImported() {
        let inside = AdditionalRoute(destination: "10.30.170.5", subnetMask: "255.255.255.255", gateway: router)
        let wider = AdditionalRoute(destination: "10.30.0.0", subnetMask: "255.255.0.0", gateway: router)
        let outcome = plan(targets: [network], snapshot([inside, wider, via(other), via(other, "10.50.0.0")]))
        #expect(outcome.imported == [other])
        #expect(outcome.routes == outcome.applied + [inside, wider, via(other, "10.50.0.0")])
        #expect(outcome.keptThroughOtherGateways == 1)
    }

    @Test func ofOverlappingRoutesAddedOutsideTheWiderJoinsWhateverTheOrder() throws {
        let wide = try IPv4Network(parsing: "10.30.0.0/16")
        for routes in [[via(host), via(wide), via(other)], [via(wide), via(other), via(host)]] {
            let outcome = plan(targets: [], snapshot(routes))
            #expect(outcome.imported == [wide, other])
            #expect(Set(outcome.applied) == [via(wide), via(other)])
            #expect(Set(outcome.routes) == [via(wide), via(other), via(host)])
            #expect(!outcome.needsWrite)
        }
    }

    @Test func onlyOtherGatewaysCountAsSuch() {
        let homeRouter = "192.168.0.1"
        let homeRoute = via(other, homeRouter)
        let foreign = via(network, "192.168.0.254")
        let outcome = plan(targets: [host], snapshot([homeRoute, foreign], router: homeRouter, address: "192.168.0.12"))
        #expect(outcome.applied.isEmpty)
        #expect(outcome.routes == [homeRoute, foreign])
        #expect(outcome.keptThroughOtherGateways == 1)
    }

    @Test func aListedTargetThroughAnotherGatewayIsLeftAlone() {
        let foreign = via(host, "192.168.0.1")
        let outcome = plan(targets: [host], snapshot([foreign]))
        #expect(outcome.applied == [via(host)])
        #expect(outcome.routes == [via(host), foreign])
    }

    @Test func aNewRouterRewritesTheAppsRoutesOnly() {
        let oldRouter = "10.20.161.1"
        let foreign = via(other, "192.168.0.1")
        let outcome = plan(
            targets: [host], applied: [via(host, oldRouter), via(network, oldRouter)],
            snapshot([via(host, oldRouter), via(network, oldRouter), foreign]))
        #expect(outcome.imported.isEmpty)
        #expect(outcome.routes == [via(host), foreign])
        #expect(outcome.needsWrite)
    }

    @Test func aFullListLeavesFurtherOutsideRoutesUntouched() throws {
        let targets = try (1...99).map { try IPv4Network(parsing: "10.90.0.\($0)") }
        let outside = try (1...3).map { try IPv4Network(parsing: "10.91.0.\($0)") }
        let outcome = plan(targets: targets, applied: targets.map { via($0) }, snapshot((targets + outside).map { via($0) }))
        #expect(outcome.imported == [outside[0]])
        #expect(outcome.notImported == 2)
        #expect(outcome.routes == outcome.applied + [via(outside[1]), via(outside[2])])
        #expect(outcome.keptThroughOtherGateways == 0)
        #expect(!outcome.needsWrite)
    }

    // MARK: - XPC payloads

    @Test func overlaps() throws {
        let wide = try IPv4Network(parsing: "10.30.0.0/16")
        #expect(wide.overlaps(network) && network.overlaps(wide))
        #expect(network.overlaps(network))
        #expect(!network.overlaps(other))
    }

    @Test func networksTravelAsValidatedText() throws {
        let data = try XPCCoding.encode([host, network])
        #expect(String(decoding: data, as: UTF8.self) == #"["10.30.161.16","10.30.170.0\/24"]"#)
        #expect(try XPCCoding.decode([IPv4Network].self, from: data) == [host, network])
        #expect(throws: DecodingError.self) {
            try XPCCoding.decode([IPv4Network].self, from: Data(#"["10.30.161.16 -x"]"#.utf8))
        }
        #expect(throws: DecodingError.self) {
            try XPCCoding.decode([IPv4Network].self, from: Data(#"["127.0.0.1"]"#.utf8))
        }
    }

    @Test func requestAndReplyRoundTrip() throws {
        let read = snapshot([via(host)])
        let request = BypassRoutesRequest(expected: read, routes: [via(host), via(network)])
        #expect(try XPCCoding.decode(BypassRoutesRequest.self, from: XPCCoding.encode(request)) == request)
        let replies: [BypassRoutesReply] = [.success(read), .failure(HelperError(.wrongPhase, "no active network service"))]
        for reply in replies {
            #expect(try XPCCoding.decode(BypassRoutesReply.self, from: XPCCoding.encode(reply)) == reply)
        }
        #expect(read.localNetwork == office)
    }
}
