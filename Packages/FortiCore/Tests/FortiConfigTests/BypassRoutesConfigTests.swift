import FortiShared
import Foundation
import Testing
@testable import FortiConfig

@Suite("routes.networks and routes.bypass_vpn in config.yaml")
struct BypassRoutesConfigTests {
    @Test("Hosts and networks in canonical spelling", arguments: [
        ("10.30.161.16", "10.30.161.16", 32), ("10.30.161.16/32", "10.30.161.16", 32),
        ("10.30.170.0/24", "10.30.170.0/24", 24), ("10.0.0.0/8", "10.0.0.0/8", 8),
        ("192.168.50.128/25", "192.168.50.128/25", 25), ("223.255.255.255", "223.255.255.255", 32),
        ("1.0.0.0/8", "1.0.0.0/8", 8),
    ])
    func accepted(raw: String, canonical: String, prefixLength: Int) throws {
        let destination = try IPv4Network(parsing: raw)
        #expect(destination.description == canonical)
        #expect(destination.prefixLength == prefixLength)
    }

    @Test("Malformed spellings", arguments: [
        "", " 10.0.0.1", "10.0.0.1 ", "10.0.0", "10.0.0.1.2", "10.0.0.256", "010.0.0.1", "10.0.0.01", "10.0.0.1/",
        "10.0.0.1/024", "10.0.0.1/24/1", "10.0.0.1/+8", "0x0a.0.0.1", "10.0.0.-1", "10..0.1", "host.example.com",
        "10.0.0.1/33a", "\u{FF11}0.0.0.1", "::1", "10.0.0.1/100",
    ])
    func malformed(raw: String) {
        #expect(throws: IPv4Network.ParseError.malformed) { try IPv4Network(parsing: raw) }
    }

    @Test func prefixLengthsOutsideEightToThirtyTwo() {
        #expect(throws: IPv4Network.ParseError.prefixOutOfRange(7)) { try IPv4Network(parsing: "10.0.0.0/7") }
        #expect(throws: IPv4Network.ParseError.prefixOutOfRange(0)) { try IPv4Network(parsing: "10.0.0.0/0") }
        #expect(throws: IPv4Network.ParseError.prefixOutOfRange(33)) { try IPv4Network(parsing: "10.0.0.1/33") }
    }

    @Test("Addresses that cannot be routed", arguments: [
        "0.0.0.0", "0.1.2.3", "127.0.0.1", "127.0.0.0/8", "224.0.0.1", "239.255.255.250", "240.0.0.0/8", "255.255.255.255",
    ])
    func unroutable(raw: String) {
        #expect(throws: IPv4Network.ParseError.unroutable) { try IPv4Network(parsing: raw) }
    }

    @Test func hostBitsAfterThePrefixSuggestTheNetwork() throws {
        let error = try #require(throws: IPv4Network.ParseError.self) {
            try IPv4Network(parsing: "10.30.170.5/24")
        }
        guard case .hostBitsSet(let network) = error else {
            Issue.record("expected hostBitsSet, got \(error)")
            return
        }
        #expect(network.description == "10.30.170.0/24")
        #expect(error.message(for: "10.30.170.5/24").contains("10.30.170.0/24"))
    }

    @Test func networksetupMasks() throws {
        #expect(try IPv4Network(address: "10.30.161.16", subnetMask: "255.255.255.255").description == "10.30.161.16")
        #expect(try IPv4Network(address: "10.30.170.0", subnetMask: "255.255.255.0").description == "10.30.170.0/24")
        #expect(throws: IPv4Network.ParseError.nonContiguousMask) {
            try IPv4Network(address: "10.0.0.0", subnetMask: "255.0.255.0")
        }
        #expect(throws: IPv4Network.ParseError.prefixOutOfRange(0)) {
            try IPv4Network(address: "10.0.0.0", subnetMask: "0.0.0.0")
        }
        let route = try AdditionalRoute(IPv4Network(parsing: "10.30.170.0/24"), gateway: "10.20.161.254")
        #expect(route == AdditionalRoute(destination: "10.30.170.0", subnetMask: "255.255.255.0", gateway: "10.20.161.254"))
    }

    @Test func defaultIsEmpty() throws {
        #expect(AppConfig().routes.bypassVPN.isEmpty)
        #expect(try decodeYAML("routes:\n").config.routes.bypassVPN.isEmpty)
        #expect(try decodeYAML("routes: { bypass_vpn: ~ }\n").config.routes.bypassVPN.isEmpty)
    }

    @Test func entriesAreReadInCanonicalSpellingAndRepeatsDropped() throws {
        let result = try decodeYAML("""
            routes:
              bypass_vpn:
                - 10.30.161.16/32
                - '10.30.170.0/24'
                - "10.30.161.16"
            """)
        #expect(result.config.routes.bypassVPN.map(\.description) == ["10.30.161.16", "10.30.170.0/24"])
        #expect(result.warnings == [ConfigWarning(.duplicateRouteEntry(path: "routes.bypass_vpn", index: 2), at: loc(5, 7))])
    }

    @Test func aMalformedEntryFailsTheFileAtTheEntry() throws {
        let error = try #require(loadError("routes:\n  bypass_vpn:\n    - 10.30.161.16\n    - 10.30.161\n"))
        #expect(error.kind == .invalidRouteEntry(path: "routes.bypass_vpn", index: 1, raw: "10.30.161", reason: .malformed))
        #expect(error.location == loc(4, 7))
        #expect(error.message.contains("routes.bypass_vpn"))
        #expect(error.message.contains("entry 2"))
    }

    @Test func wrongShapes() throws {
        let notList = try #require(loadError("routes: { bypass_vpn: 10.0.0.1 }\n"))
        #expect(notList.kind == .typeMismatch(path: "routes.bypass_vpn", expected: .sequence))
        let nested = try #require(loadError("routes: { bypass_vpn: [[10.0.0.1]] }\n"))
        #expect(nested.kind == .typeMismatch(path: "routes.bypass_vpn", expected: .string))
        let empty = try #require(loadError("routes: { bypass_vpn: [~] }\n"))
        #expect(empty.kind == .invalidRouteEntry(path: "routes.bypass_vpn", index: 0, raw: "", reason: .malformed))
        let unknown = try decodeYAML("routes: { bypass: [] }\n")
        #expect(unknown.warnings.map(\.kind) == [.unknownKey(path: "routes.bypass")])
    }

    @Test func atMostAHundredEntries() throws {
        let hosts = (1...101).map { "10.0.\($0 / 256).\($0 % 256)" }
        let yaml = "routes:\n  bypass_vpn:\n" + hosts.map { "    - \($0)\n" }.joined()
        let error = try #require(loadError(yaml))
        #expect(error.kind == .tooManyRouteEntries(path: "routes.bypass_vpn", count: 101, limit: 100))

        var config = AppConfig()
        config.routes.bypassVPN = try hosts.map { try IPv4Network(parsing: $0) }
        #expect(throws: ConfigLoadError(.tooManyRouteEntries(path: "routes.bypass_vpn", count: 101, limit: 100))) { try config.validate() }
        config.routes.bypassVPN.removeLast()
        try config.validate()
    }

    @Test func validateRejectsRepeats() throws {
        var config = AppConfig()
        config.routes.bypassVPN = [bypassHost, bypassNetwork, bypassHost]
        #expect(throws: ConfigLoadError(.duplicateRouteEntry(path: "routes.bypass_vpn", index: 2))) { try config.validate() }
    }

    @Test func messages() {
        #expect(IPv4Network.ParseError.malformed.message(for: "10.0.0").contains("10.30.170.0/24"))
        #expect(IPv4Network.ParseError.prefixOutOfRange(4).message(for: "10.0.0.0/4").contains("/8"))
        #expect(IPv4Network.ParseError.unroutable.message(for: "127.0.0.1").contains("127.0.0.1"))
    }

    @Test func networksAreReadLikeTargets() throws {
        let result = try decodeYAML("""
            routes:
              networks: [10.20.161.0/24, "10.30.161.0/24", 10.20.161.0/24]
              bypass_vpn: [10.30.161.16]
            """)
        #expect(result.config.routes.networks.map(\.description) == ["10.20.161.0/24", "10.30.161.0/24"])
        #expect(result.warnings.map(\.kind) == [.duplicateRouteEntry(path: "routes.networks", index: 2)])
        let error = try #require(loadError("routes:\n  networks: [10.20.161.5/24]\n"))
        guard case .invalidRouteEntry(let path, 0, "10.20.161.5/24", .hostBitsSet) = error.kind else {
            Issue.record("expected invalidRouteEntry, got \(error.kind)")
            return
        }
        #expect(path == "routes.networks")
        #expect(error.message.contains("routes.networks"))
        var config = AppConfig()
        config.routes.networks = [bypassNetwork, bypassNetwork]
        #expect(throws: ConfigLoadError(.duplicateRouteEntry(path: "routes.networks", index: 1))) { try config.validate() }
    }

    @Test func overlappingEntriesFailTheFile() throws {
        let error = try #require(loadError("routes:\n  bypass_vpn:\n    - 10.30.161.0/24\n    - 10.30.161.16\n"))
        #expect(error.kind == .overlappingRouteEntry(path: "routes.bypass_vpn", index: 1, other: try IPv4Network(parsing: "10.30.161.0/24")))
        #expect(error.location == loc(4, 7))
        #expect(error.message.contains("10.30.161.0/24"))
        let wider = try #require(loadError("routes:\n  networks: [10.20.161.0/24, 10.20.0.0/16]\n"))
        #expect(wider.kind == .overlappingRouteEntry(path: "routes.networks", index: 1, other: officeNetwork))
        var config = AppConfig()
        config.routes.bypassVPN = [bypassNetwork, try IPv4Network(parsing: "10.30.0.0/16")]
        #expect(throws: ConfigLoadError(.overlappingRouteEntry(path: "routes.bypass_vpn", index: 1, other: bypassNetwork))) {
            try config.validate()
        }
        // The two lists may overlap each other.
        config.routes.bypassVPN = [bypassHost]
        config.routes.networks = [try IPv4Network(parsing: "10.30.161.0/24")]
        try config.validate()
    }

    @Test func containment() throws {
        #expect(officeNetwork.contains(IPv4Network.parseDottedQuad("10.20.161.20")!))
        #expect(!officeNetwork.contains(IPv4Network.parseDottedQuad("10.20.162.20")!))
        #expect(officeNetwork.contains(try IPv4Network(parsing: "10.20.161.16")))
        #expect(officeNetwork.contains(officeNetwork))
        #expect(!officeNetwork.contains(try IPv4Network(parsing: "10.20.0.0/16")))
        #expect(try IPv4Network(enclosing: "10.20.161.20", subnetMask: "255.255.255.0") == officeNetwork)
        #expect(throws: IPv4Network.ParseError.nonContiguousMask) {
            try IPv4Network(enclosing: "10.20.161.20", subnetMask: "255.0.255.0")
        }
    }
}
