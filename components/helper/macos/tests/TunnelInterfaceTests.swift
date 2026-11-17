import Testing

@Suite struct TunnelInterfaceTests {
    private let configured = """
        utun5: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1412
        \tinet 10.212.133.105 --> 10.212.133.105 netmask 0xffffffff
        \tinet6 fe80::8683:2d63:15a1:fa95%utun5 prefixlen 64 scopeid 0x1f
        \tinet6 fd00:10::5 prefixlen 64

        """

    private let bare = """
        utun5: flags=8050<POINTOPOINT,RUNNING,MULTICAST> mtu 1500

        """

    private func network(_ address: String, _ prefixLength: Int) -> HelperInternal.IPNetwork {
        HelperInternal.IPNetwork(address: address, prefixLength: prefixLength)
    }

    private func settings(
        address: String = "10.212.133.105", mtu: UInt32? = nil, ipv6: HelperInternal.IPNetwork? = nil
    ) -> HelperInternal.NetworkSettings {
        HelperInternal.NetworkSettings(
            tunnel: HelperInternal.TunnelDevice(name: "utun5", index: nil), gateway: "203.0.113.10", mtu: mtu,
            ipv4: HelperInternal.FamilySettings(address: network(address, 32), routes: .split(include: [network("10.30.0.0", 16)])),
            ipv6: ipv6.map { HelperInternal.FamilySettings(address: $0, routes: .full) },
            splitExclude: [], dnsServers: [], defaultDomains: [], splitDnsDomains: [], winsServers: []
        )
    }

    @Test func ifconfigIsReadWithItsFlagsAddressesAndMTU() throws {
        let interface = try #require(TunnelInterface.parse(configured))
        #expect(interface.up)
        #expect(interface.mtu == 1412)
        #expect(interface.ipv4 == [TunnelInterface.IPv4Address(address: "10.212.133.105", destination: "10.212.133.105")])
        #expect(interface.ipv6 == [network("fe80::8683:2d63:15a1:fa95", 64), network("fd00:10::5", 64)])
        let down = try #require(TunnelInterface.parse(bare))
        #expect(!down.up && down.mtu == 1500 && down.ipv4.isEmpty && down.ipv6.isEmpty)
    }

    @Test func anAddressWithoutPeerHasNoDestination() throws {
        let output = """
            en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
            \toptions=567<RXCSUM,TXCSUM,VLAN_MTU,TSO4,TSO6,AV,CHANNEL_IO>
            \tinet 10.71.161.20 netmask 0xffffff00 broadcast 10.71.161.255

            """
        let interface = try #require(TunnelInterface.parse(output))
        #expect(interface.ipv4 == [TunnelInterface.IPv4Address(address: "10.71.161.20", destination: nil)])
    }

    @Test func outputWithoutFlagsIsNoInterface() {
        #expect(TunnelInterface.parse("") == nil)
        #expect(TunnelInterface.parse("ifconfig: interface utun9 does not exist\n") == nil)
    }

    @Test func aBareTunnelGetsItsAddressUpAndTheDefaultMTU() throws {
        let changes = TunnelInterface.changes(to: settings(), from: try #require(TunnelInterface.parse(bare)))
        #expect(changes.map(\.step) == [.addresses, .addresses, .mtu])
        #expect(changes.map(\.arguments) == [
            ["inet", "10.212.133.105", "10.212.133.105", "netmask", "255.255.255.255"],
            ["up"],
            ["mtu", "1412"],
        ])
    }

    @Test func theGivenMTUAndIPv6AddressAreSet() throws {
        let wanted = settings(mtu: 1354, ipv6: network("fd00:10::5", 64))
        let changes = TunnelInterface.changes(to: wanted, from: try #require(TunnelInterface.parse(bare)))
        #expect(changes.map(\.step) == [.addresses, .addresses, .addresses, .mtu])
        #expect(changes.map(\.arguments) == [
            ["inet", "10.212.133.105", "10.212.133.105", "netmask", "255.255.255.255"],
            ["inet6", "fd00:10::5/64"],
            ["up"],
            ["mtu", "1354"],
        ])
    }

    @Test func aTunnelThatHasTheSettingsIsLeftAlone() throws {
        let interface = try #require(TunnelInterface.parse(configured))
        let wanted = settings(ipv6: network("FD00:10:0::5", 64))
        #expect(TunnelInterface.changes(to: wanted, from: interface).isEmpty)
        #expect(interface.mismatch(with: wanted, previous: nil) == nil)
    }

    @Test func onlyAddressesTheSettingsNoLongerHaveAreRemoved() {
        let dual = settings(ipv6: network("fd00:10::5", 64))
        #expect(TunnelInterface.removals(from: dual, to: settings()) == [["inet6", "fd00:10::5", "delete"]])
        #expect(TunnelInterface.removals(from: dual, to: settings(ipv6: network("fd00:10::5", 80)))
            == [["inet6", "fd00:10::5", "delete"]])
        #expect(TunnelInterface.removals(from: settings(), to: settings(address: "10.212.133.106"))
            == [["inet", "10.212.133.105", "delete"]])
        #expect(TunnelInterface.removals(from: dual, to: dual).isEmpty)
    }

    @Test func aMissingAddressALeftoverOrAnotherMTUIsAMismatch() throws {
        let interface = try #require(TunnelInterface.parse(configured))
        #expect(interface.mismatch(with: settings(), previous: nil) == nil)
        #expect(interface.mismatch(with: settings(), previous: settings(ipv6: network("fd00:10::5", 64))) == .addresses)
        #expect(interface.mismatch(with: settings(address: "10.212.133.106"), previous: nil) == .addresses)
        #expect(interface.mismatch(with: settings(ipv6: network("fd00:10::6", 64)), previous: nil) == .addresses)
        #expect(interface.mismatch(with: settings(mtu: 1354), previous: nil) == .mtu)
        #expect(try #require(TunnelInterface.parse(bare)).mismatch(with: settings(), previous: nil) == .addresses)
    }
}
