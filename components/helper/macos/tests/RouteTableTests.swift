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
        RouteTable.parse(table: table, family: .inet).contains(where: route.matches)
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
        let rows = RouteTable.parse(table: table, family: .inet)
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
        #expect(RouteTable.parse(table: "", family: .inet).isEmpty)
        #expect(RouteTable.parse(table: "Routing tables\n\nInternet:\n", family: .inet).isEmpty)
    }

    private let ipv6Table = """
        Routing tables

        Internet6:
        Destination                             Gateway                                 Flags               Netif Expire
        default                                 fe80::1%en0                             UGcg                  en0
        default                                 fe80::%utun0                            UGcIg               utun0
        ::/1                                    fd00:10::5                              UGSc                utun5
        ::1                                     ::1                                     UHL                   lo0
        2001:db8::10                            fe80::1%en0                             UGHS                  en0
        2001:db8::12                            fe80::1%en0                             UGHW                  en0
        2001:db8:1::/48                         2001:db8::1                             UGSc                  en0
        8000::/1                                fd00:10::5                              UGSc                utun5
        fe80::%lo0/64                           fe80::1%lo0                             UcI                   lo0
        fe80::1%lo0                             link#1                                  UHLI                  lo0
        fe80::877:2144:ddd4:6e55%en0            2:0:0:0:0:0                             UHLI                  lo0
        ff00::/8                                ::1                                     UmCI                  lo0

        """

    private func has(ipv6 route: RouteTable.Route) -> Bool {
        RouteTable.parse(table: ipv6Table, family: .inet6).contains(where: route.matches)
    }

    @Test func aPrefixLengthMakesTheSameIPv4RouteAsAMask() throws {
        let route = try #require(RouteTable.Route(destination: "10.30.170.0", prefixLength: 24, gateway: "10.20.161.254"))
        #expect(route == network)
        #expect(RouteTable.Route(destination: "0.0.0.0", prefixLength: 1, gateway: "10.212.133.105")?.addArguments
            == ["add", "-net", "0.0.0.0", "-netmask", "128.0.0.0", "10.212.133.105"])
        #expect(RouteTable.Route(destination: "10.30.170.0", prefixLength: 33, gateway: "10.20.161.254") == nil)
        #expect(RouteTable.Route(destination: "10.30.170.0", prefixLength: 24, gateway: "fe80::1") == nil)
    }

    @Test func ipv6RoutesAreCanonicalAndNamedWithTheirPrefix() throws {
        let host = try #require(RouteTable.Route(destination: "2001:DB8:0:0::10", prefixLength: 128, gateway: "FE80:0::1%en0"))
        #expect(host.family == .inet6)
        #expect(host.destination == "2001:db8::10")
        #expect(host.gateway == "fe80::1%en0")
        #expect(host.addArguments == ["add", "-inet6", "-host", "2001:db8::10", "fe80::1%en0"])
        let network = try #require(RouteTable.Route(destination: "2001:db8:1::", prefixLength: 48, gateway: "2001:db8::1"))
        #expect(network.deleteArguments == ["delete", "-inet6", "-net", "2001:db8:1::/48", "2001:db8::1"])
    }

    @Test func onlyIPv6AddressesAndInterfaceNamesReachAnIPv6Route() {
        #expect(RouteTable.Route(destination: "-net", prefixLength: 64, gateway: "fe80::1%en0") == nil)
        #expect(RouteTable.Route(destination: "2001:db8::", prefixLength: 129, gateway: "fe80::1%en0") == nil)
        #expect(RouteTable.Route(destination: "2001:db8::", prefixLength: 64, gateway: "fe80::1%") == nil)
        #expect(RouteTable.Route(destination: "2001:db8::", prefixLength: 64, gateway: "fe80::1%-ifscope") == nil)
        #expect(RouteTable.Route(destination: "2001:db8::", prefixLength: 64, gateway: "10.20.161.254") == nil)
        #expect(RouteTable.Route(destination: "2001:db8::", prefixLength: 64, gateway: "fe80::1 ") == nil)
    }

    @Test func theIPv6TableIsReadWithScopesAndHostsWithoutPrefix() throws {
        let rows = RouteTable.parse(table: ipv6Table, family: .inet6)
        #expect(rows.count == 12)
        #expect(rows[0] == RouteTable.Row(
            destination: "::", prefixLength: 0, gateway: "fe80::1%en0", flags: ["U", "G", "c", "g"], interface: "en0"
        ))
        let found = { (destination: String, length: Int) in
            rows.first { $0.destination == destination && $0.prefixLength == length }
        }
        #expect(found("::1", 128) != nil)
        #expect(found("2001:db8::10", 128) != nil)
        #expect(found("8000::", 1) != nil)
        #expect(try #require(found("fe80::", 64)).gateway == "fe80::1%lo0")
        #expect(try #require(found("fe80::1", 128)).gateway == "link#1")
        #expect(try #require(found("fe80::877:2144:ddd4:6e55", 128)).gateway == "2:0:0:0:0:0")
    }

    @Test func theIPv6RoutesTheHelperAddsAreFound() throws {
        #expect(has(ipv6: try #require(RouteTable.Route(destination: "2001:db8::10", prefixLength: 128, gateway: "fe80::1%en0"))))
        #expect(has(ipv6: try #require(RouteTable.Route(destination: "2001:db8:1::", prefixLength: 48, gateway: "2001:db8::1"))))
        #expect(has(ipv6: try #require(RouteTable.Route(destination: "::", prefixLength: 1, gateway: "fd00:10::5"))))
        #expect(!has(ipv6: try #require(RouteTable.Route(destination: "2001:db8::12", prefixLength: 128, gateway: "fe80::1%en0"))))
        #expect(!has(ipv6: try #require(RouteTable.Route(destination: "2001:db8::10", prefixLength: 128, gateway: "fe80::1%en1"))))
        #expect(!has(ipv6: try #require(RouteTable.Route(destination: "::", prefixLength: 0, gateway: "fe80::1%en0"))))
    }
}
