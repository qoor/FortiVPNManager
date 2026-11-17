import Foundation
import Testing

@Suite struct TunnelNetworkTests {
    private func network(_ address: String, _ prefixLength: Int) -> HelperInternal.IPNetwork {
        HelperInternal.IPNetwork(address: address, prefixLength: prefixLength)
    }

    private var split: HelperInternal.NetworkSettings {
        HelperInternal.NetworkSettings(
            tunnel: HelperInternal.TunnelDevice(name: "utun5", index: nil), gateway: "203.0.113.10", mtu: 1354,
            ipv4: HelperInternal.FamilySettings(
                address: network("10.212.133.105", 32),
                routes: .split(include: [network("10.30.0.0", 16), network("192.168.50.0", 24)])
            ),
            ipv6: nil, splitExclude: [network("10.30.161.0", 24)], dnsServers: ["10.30.0.53", "10.30.0.54"],
            defaultDomains: ["corp.example.com"], splitDnsDomains: ["corp.example.com", "example.net"], winsServers: []
        )
    }

    private var full: HelperInternal.NetworkSettings {
        HelperInternal.NetworkSettings(
            tunnel: HelperInternal.TunnelDevice(name: "utun5", index: nil), gateway: "203.0.113.10", mtu: nil,
            ipv4: HelperInternal.FamilySettings(address: network("10.212.133.105", 24), routes: .full),
            ipv6: HelperInternal.FamilySettings(address: network("fd00:10::5", 64), routes: .full),
            splitExclude: [], dnsServers: ["10.30.0.53", "fd00:10::53"], defaultDomains: [], splitDnsDomains: [],
            winsServers: ["10.30.0.60"]
        )
    }

    @Test func aSplitTunnelRoutesItsIncludesThroughItsOwnAddress() {
        #expect(TunnelNetwork.tunnelRoutes(split).map(\.addArguments) == [
            ["add", "-net", "10.30.0.0", "-netmask", "255.255.0.0", "10.212.133.105"],
            ["add", "-net", "192.168.50.0", "-netmask", "255.255.255.0", "10.212.133.105"],
        ])
    }

    @Test func aFullTunnelCoversEverythingBesideTheDefaultRoute() {
        #expect(TunnelNetwork.tunnelRoutes(full).map(\.addArguments) == [
            ["add", "-net", "10.212.133.0", "-netmask", "255.255.255.0", "10.212.133.105"],
            ["add", "-net", "0.0.0.0", "-netmask", "128.0.0.0", "10.212.133.105"],
            ["add", "-net", "128.0.0.0", "-netmask", "128.0.0.0", "10.212.133.105"],
            ["add", "-inet6", "-net", "::/1", "fd00:10::5"],
            ["add", "-inet6", "-net", "8000::/1", "fd00:10::5"],
        ])
    }

    @Test func anIncludeOfTheTunnelsOwnNetworkIsRoutedOnce() {
        var settings = split
        settings.ipv4 = HelperInternal.FamilySettings(
            address: network("10.212.133.105", 24),
            routes: .split(include: [network("10.212.133.0", 24), network("10.30.0.0", 16)])
        )
        #expect(TunnelNetwork.tunnelRoutes(settings).map(\.addArguments) == [
            ["add", "-net", "10.212.133.0", "-netmask", "255.255.255.0", "10.212.133.105"],
            ["add", "-net", "10.30.0.0", "-netmask", "255.255.0.0", "10.212.133.105"],
        ])
    }

    @Test func theTunnelRoutesAreFoundAsNetstatPrintsThem() {
        let table = RouteTable.parse(table: """
            Routing tables

            Internet:
            Destination        Gateway            Flags               Netif Expire
            0/1                10.212.133.105     UGSc                utun5
            default            10.71.161.254      UGScg                 en0
            10.212.133/24      10.212.133.105     UGSc                utun5
            10.212.133.105     10.212.133.105     UH                  utun5
            128.0/1            10.212.133.105     UGSc                utun5

            """, family: .inet) + RouteTable.parse(table: """
            Routing tables

            Internet6:
            Destination                             Gateway                                 Flags               Netif Expire
            ::/1                                    fd00:10::5                              UGSc                utun5
            8000::/1                                fd00:10::5                              UGSc                utun5

            """, family: .inet6)
        #expect(TunnelNetwork.tunnelRoutes(full).allSatisfy { route in table.contains(where: route.matches) })
        #expect(!TunnelNetwork.tunnelRoutes(split).contains { route in table.contains(where: route.matches) })
    }

    @Test func splitDNSAnswersItsDomainsWithoutMarkingTheTunnelPrimary() {
        let entries = TunnelNetwork.dnsEntries(split)
        #expect(entries.dns.map { $0 as NSDictionary } == [
            "ServerAddresses": ["10.30.0.53", "10.30.0.54"],
            "SearchDomains": ["corp.example.com"],
            "SupplementalMatchDomains": ["corp.example.com", "example.net"],
        ] as NSDictionary)
        #expect(entries.ipv4 == nil)
    }

    @Test func fullDNSMarksTheTunnelPrimaryOnlyInTheRunningState() {
        let entries = TunnelNetwork.dnsEntries(full)
        #expect(entries.dns.map { $0 as NSDictionary } == ["ServerAddresses": ["10.30.0.53", "fd00:10::53"]] as NSDictionary)
        #expect(entries.ipv4.map { $0 as NSDictionary } == [
            "Addresses": ["10.212.133.105"],
            "SubnetMasks": ["255.255.255.255"],
            "Router": "10.212.133.105",
            "InterfaceName": "utun5",
            "OverridePrimary": 1,
        ] as NSDictionary)
    }

    @Test func noDNSServersWriteNoState() {
        var settings = full
        settings.dnsServers = []
        let entries = TunnelNetwork.dnsEntries(settings)
        #expect(entries.dns == nil && entries.ipv4 == nil)
    }

    @Test func onlyFE80Slash10RoutersAreLinkLocal() {
        #expect(TunnelNetwork.isLinkLocal("fe80::1"))
        #expect(TunnelNetwork.isLinkLocal("febf::1"))
        #expect(!TunnelNetwork.isLinkLocal("fec0::1"))
        #expect(!TunnelNetwork.isLinkLocal("2001:db8::1"))
        #expect(!TunnelNetwork.isLinkLocal("10.20.161.1"))
    }

    @Test func thePrimaryServiceIsTheGlobalOneUnlessItIsATunnel() {
        let states: [String: [String: Any]] = ["ETH": ["InterfaceName": "en0", "Router": "10.20.161.1"]]
        let primary = { (service: String, interface: String) in
            BypassRoutes.primaryServiceID(
                global: ["PrimaryService": service, "PrimaryInterface": interface],
                serviceOrder: ["ETH"], ipv4: { states[$0] }
            )
        }
        #expect(primary("ETH", "en0") == "ETH")
        #expect(primary("PPP", "ppp0") == nil)
        #expect(primary("OTHER", "utun3") == nil)
        #expect(BypassRoutes.primaryServiceID(global: nil, serviceOrder: ["ETH"], ipv4: { states[$0] }) == nil)
    }

    @Test func whileTheTunnelIsPrimaryTheFirstPhysicalServiceInOrderIs() {
        let states: [String: [String: Any]] = [
            "BRIDGE": ["InterfaceName": "bridge0"],
            "VPN": ["InterfaceName": "utun3", "Router": "10.8.0.1"],
            "WIFI": ["InterfaceName": "en1", "Router": "192.168.0.1"],
            "ETH": ["InterfaceName": "en0", "Router": "10.20.161.1"],
        ]
        let global: [String: Any] = ["PrimaryService": TunnelNetwork.serviceID, "PrimaryInterface": "utun5"]
        #expect(BypassRoutes.primaryServiceID(
            global: global, serviceOrder: ["GONE", "BRIDGE", "VPN", "WIFI", "ETH"], ipv4: { states[$0] }
        ) == "WIFI")
        #expect(BypassRoutes.primaryServiceID(global: global, serviceOrder: ["BRIDGE", "VPN"], ipv4: { states[$0] }) == nil)
    }
}
