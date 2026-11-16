import Testing

/// The `/sbin/route` commands the Helper builds and how it reads `netstat -rn -f inet`. The table is
/// as macOS 27 prints it, with the Helper's routes, the VPN's and copies the system made.
@Suite struct RouteTableTests {
    private let host = RouteTable.Route(destination: "10.30.161.16", subnetMask: "255.255.255.255", gateway: "10.20.161.254")!
    private let network = RouteTable.Route(destination: "10.30.170.0", subnetMask: "255.255.255.0", gateway: "10.20.161.254")!

    private let table = """
        Routing tables

        Internet:
        Destination        Gateway            Flags               Netif Expire
        default            10.20.161.254      UGScg                 en0
        10.20.161/24       link#22            UCS                   en0      !
        10.20.161.20/32    link#22            UCS                   en0      !
        10.30.161.16       10.20.161.254      UGHS                  en0
        10.30.161.17       10.20.161.254      UGHWIi                en0
        10.30.161.18       10.20.161.254      UGHW                  en0
        10.30.161.19       10.20.161.254      UGHSI                 en0
        10.30.170/24       10.20.161.254      UGSc                  en0
        10.30.171.5/32     10.212.133.100     UGSc                utun6
        10.30/16           10.212.133.100     UGSc                utun6
        127                127.0.0.1          UCS                   lo0
        127.0.0.1          127.0.0.1          UH                    lo0
        169.254            link#22            UCS                   en0      !
        192.168.50         10.20.161.254      UGSc                  en0

        """

    private func route(_ destination: String, _ mask: String, _ gateway: String = "10.20.161.254") -> RouteTable.Route {
        RouteTable.Route(destination: destination, subnetMask: mask, gateway: gateway)!
    }

    private func has(_ route: RouteTable.Route) -> Bool {
        RouteTable.parse(table: table).contains(where: route.matches)
    }

    @Test func onlyDottedQuadsAndContiguousMasksAreRoutes() {
        #expect(RouteTable.Route(destination: "-net", subnetMask: "255.255.255.255", gateway: "10.20.161.254") == nil)
        #expect(RouteTable.Route(destination: "10.30.161.16", subnetMask: "255.0.255.0", gateway: "10.20.161.254") == nil)
        #expect(RouteTable.Route(destination: "10.30.161.16", subnetMask: "255.255.255.255", gateway: "10.20.161.254 ") == nil)
        #expect(RouteTable.Route(destination: "010.30.161.16", subnetMask: "255.255.255.255", gateway: "10.20.161.254") == nil)
        #expect(RouteTable.Route(destination: "0.0.0.0", subnetMask: "0.0.0.0", gateway: "10.20.161.254")?.prefixLength == 0)
        let bypass = BypassRoute(destination: "10.30.170.0", subnetMask: "255.255.255.0", gateway: "10.20.161.254")
        #expect(RouteTable.Route(bypass) == network)
        #expect(network.prefixLength == 24)
    }

    @Test func aHostIsNamedAsOneAndANetworkWithItsMask() {
        #expect(host.addArguments == ["add", "-host", "10.30.161.16", "10.20.161.254"])
        #expect(host.deleteArguments == ["delete", "-host", "10.30.161.16", "10.20.161.254"])
        #expect(network.addArguments == ["add", "-net", "10.30.170.0", "-netmask", "255.255.255.0", "10.20.161.254"])
        #expect(network.deleteArguments == ["delete", "-net", "10.30.170.0", "-netmask", "255.255.255.0", "10.20.161.254"])
    }

    @Test func theTableIsReadWithNetstatsShortForms() throws {
        let rows = RouteTable.parse(table: table)
        #expect(rows.count == 14)
        let found = { (destination: String, length: Int) in
            rows.first { $0.destination == destination && $0.prefixLength == length }
        }
        #expect(try #require(found("0.0.0.0", 0)).gateway == "10.20.161.254")
        #expect(try #require(found("10.20.161.0", 24)).flags == ["U", "C", "S"])
        #expect(found("10.20.161.20", 32) != nil)
        #expect(try #require(found("10.30.0.0", 16)).interface == "utun6")
        #expect(found("127.0.0.0", 8) != nil)
        #expect(found("127.0.0.1", 32) != nil)
        #expect(found("169.254.0.0", 16) != nil)
        #expect(found("192.168.50.0", 24) != nil)
    }

    @Test func theRoutesTheHelperAddsAreFound() {
        #expect(has(host))
        #expect(has(network))
        #expect(has(route("192.168.50.0", "255.255.255.0")))
    }

    /// A copy of the default route the system made for one host (W), or a route bound to one
    /// interface (I), is not a route the Helper adds.
    @Test func copiesAndScopedRoutesAreNotTheHelpers() {
        #expect(!has(route("10.30.161.17", "255.255.255.255")))
        #expect(!has(route("10.30.161.18", "255.255.255.255")))
        #expect(!has(route("10.30.161.19", "255.255.255.255")))
    }

    @Test func anotherGatewayOrPrefixIsAnotherRoute() {
        #expect(!has(route("10.30.161.16", "255.255.255.255", "10.20.161.1")))
        #expect(!has(route("10.30.170.0", "255.255.254.0")))
        // The VPN's single host is a /32 network through the tunnel.
        #expect(!has(route("10.30.171.5", "255.255.255.255")))
    }

    @Test func nothingBeforeTheHeaderIsARoute() {
        #expect(RouteTable.parse(table: "").isEmpty)
        #expect(RouteTable.parse(table: "Routing tables\n\nInternet:\n").isEmpty)
    }

    @Test func aLookupIsReadFromItsKeyValueLines() throws {
        let output = """
               route to: 10.30.161.16
            destination: 10.30.161.16
                gateway: 10.20.161.254
              interface: en0
                  flags: <UP,GATEWAY,HOST,DONE,STATIC>

            """
        let entry = try #require(RouteTable.parse(lookup: output))
        #expect(entry.destination == "10.30.161.16")
        #expect(entry.interface == "en0")
        #expect(entry.flags == ["UP", "GATEWAY", "HOST", "DONE", "STATIC"])
        #expect(RouteTable.parse(lookup: "route: writing to routing socket: not in table\n") == nil)
    }
}
