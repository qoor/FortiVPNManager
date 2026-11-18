import Foundation
import Testing

/// Checks the Swift types in components/contract/macos against the JSON the Rust tests pin in
/// ../fixtures/helper-internal: Swift encodes what it sends and decodes what it receives.
struct HelperInternalTests {
    private let attempt = "E621E1F8-C36C-495A-93FC-0C247A3E6E5F"
    private let ethernet = BypassRoutesSnapshot(
        serviceID: "8A3F1C2E-5B6D-4E7F-9A0B-1C2D3E4F5A6B", serviceName: "Ethernet", router: "10.20.161.1",
        address: "10.20.161.20", subnetMask: "255.255.255.0"
    )
    private let host = BypassRoute(destination: "10.30.161.16", subnetMask: "255.255.255.255", gateway: "10.20.161.1")
    private let lan = BypassRoute(destination: "10.30.170.0", subnetMask: "255.255.255.0", gateway: "10.20.161.1")
    /// The host as the Helper added it through a former network.
    private let stale = HelperInternal.AddedRoute(
        route: BypassRoute(destination: "10.30.161.16", subnetMask: "255.255.255.255", gateway: "10.20.161.254"),
        serviceID: "0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0"
    )

    private let dnsService = "6B29FC40-CA47-1067-B31D-00DD010662DA"
    private let utun = HelperInternal.TunnelDevice(name: "utun5", index: nil)

    private func ipNetwork(_ address: String, _ prefixLength: Int) -> HelperInternal.IPNetwork {
        HelperInternal.IPNetwork(address: address, prefixLength: prefixLength)
    }

    private func outside(_ destination: HelperInternal.IPNetwork) -> HelperInternal.OutsideRoute {
        HelperInternal.OutsideRoute(destination: destination, router: "10.20.161.1", serviceID: ethernet.serviceID)
    }

    private var gatewayRoute: HelperInternal.OutsideRoute { outside(ipNetwork("203.0.113.10", 32)) }
    private var excludedRoute: HelperInternal.OutsideRoute { outside(ipNetwork("10.30.161.0", 24)) }

    private var splitTunnel: HelperInternal.NetworkSettings {
        HelperInternal.NetworkSettings(
            tunnel: utun, gateway: "203.0.113.10", mtu: 1354,
            ipv4: .init(
                address: ipNetwork("10.212.133.105", 32),
                routes: .split(include: [ipNetwork("10.30.0.0", 16), ipNetwork("192.168.50.0", 24)])
            ),
            ipv6: nil, splitExclude: [ipNetwork("10.30.161.0", 24)], dnsServers: ["10.30.0.53", "10.30.0.54"],
            defaultDomains: ["corp.example.com"], splitDnsDomains: ["corp.example.com", "example.net"], winsServers: []
        )
    }

    private var fullTunnel: HelperInternal.NetworkSettings {
        HelperInternal.NetworkSettings(
            tunnel: .init(name: "OpenConnect", index: 23), gateway: "2001:db8::10", mtu: nil,
            ipv4: .init(address: ipNetwork("10.212.133.105", 24), routes: .full),
            ipv6: .init(address: ipNetwork("fd00:10::5", 64), routes: .full),
            splitExclude: [], dnsServers: ["10.30.0.53", "fd00:10::53"], defaultDomains: [], splitDnsDomains: [],
            winsServers: ["10.30.0.60"]
        )
    }

    private func fixture(_ name: String) throws -> Data {
        let tests = URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        return try Data(contentsOf: tests.appending(path: "fixtures/helper-internal/\(name)"))
    }

    @Test func messagesToTheEntry() throws {
        let messages: [HelperInternal.ToEntry] = [
            .appConnected(app: 1, uid: 501),
            .request(id: 1, app: 1, request: .connect(json: #"{"attemptID":"\#(attempt)","host":"vpn.example.com"}"#)),
            .request(id: 2, app: 1, request: .submitOtp(attempt: attempt, code: "123456")),
            .request(id: 3, app: 1, request: .disconnect(attempt: attempt, reason: "userDisconnect")),
            .request(id: 4, app: 1, request: .status),
            .request(id: 5, app: 1, request: .exitWhenIdle),
            .request(id: 6, app: 1, request: .readBypassRoutes),
            .request(id: 7, app: 1, request: .writeBypassRoutes(json: #"{"remove":{}}"#)),
            .appDisconnected(app: 1),
            .executableReplaced,
            .terminationRequested,
            .bypassRoutesChanged(outcome: .changed(network: ethernet, added: [lan], found: [host, lan], left: [])),
            .bypassRoutesChanged(outcome: .changed(network: nil, added: [], found: [], left: [stale])),
            .bypassRoutesChanged(outcome: .refused(error: HelperError(.wrongPhase, "no active network service"))),
            .networkChanged(outcome: .lookedUp(dnsServiceID: dnsService, missing: [gatewayRoute, excludedRoute])),
            .networkChanged(outcome: .applied(added: [gatewayRoute, excludedRoute], left: [])),
            .networkChanged(outcome: .reverted(left: [excludedRoute])),
            .networkChanged(outcome: .failed(
                step: .dns, error: "the DNS settings could not be written", added: [gatewayRoute], left: []
            )),
            .networkChanged(outcome: .failed(step: .network, error: "no active network service", added: [], left: [])),
        ]
        let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(messages)) as? NSArray
        let expected = try JSONSerialization.jsonObject(with: fixture("to_entry.json")) as? NSArray
        #expect(encoded != nil && encoded == expected)
    }

    @Test func networkSteps() throws {
        let steps: [HelperInternal.NetworkStep] = [.network, .addresses, .mtu, .routes, .outsideRoutes, .dns, .wins]
        let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(steps)) as? NSArray
        let expected = try JSONSerialization.jsonObject(with: fixture("network_steps.json")) as? NSArray
        #expect(encoded != nil && encoded == expected)
    }

    @Test func messagesFromTheEntry() throws {
        let messages = try JSONDecoder().decode([HelperInternal.FromEntry].self, from: fixture("from_entry.json"))
        let expected: [HelperInternal.FromEntry] = [
            .ready,
            .admitted(app: 1, accept: true),
            .admitted(app: 2, accept: false),
            .reply(id: 1, json: nil),
            .reply(id: 2, json: #"{"code":"busy","message":"openfortivpn is already running"}"#),
            .replyFlag(id: 5, value: true),
            .sendEvent(app: 1, json: #"{"attemptID":"\#(attempt)","kind":{"milestone":{"_0":0}}}"#),
            .sendOutput(
                app: 1,
                json: #"[{"date":1762412345.25,"source":"stdout","text":"INFO:   Connected to gateway."}]"#
            ),
            .changeBypassRoutes(change: .init(
                network: .expect(network: ethernet), remove: [stale], add: [host, lan], lookUp: [host, lan]
            )),
            .changeBypassRoutes(change: .init(network: .read, remove: [], add: [], lookUp: [host])),
            .changeBypassRoutes(change: .init(network: .skip, remove: [stale], add: [], lookUp: [])),
            .changeNetwork(change: .lookUp(tunnel: utun, outside: [ipNetwork("203.0.113.10", 32), ipNetwork("10.30.161.0", 24)])),
            .changeNetwork(change: .apply(
                settings: splitTunnel, previous: nil, remove: [], add: [gatewayRoute, excludedRoute]
            )),
            .changeNetwork(change: .apply(settings: fullTunnel, previous: splitTunnel, remove: [excludedRoute], add: [])),
            .changeNetwork(change: .prepare(settings: fullTunnel, add: [gatewayRoute])),
            .changeNetwork(change: .switch(settings: fullTunnel, replaced: splitTunnel)),
            .changeNetwork(change: .revert(changes: .init(dnsServiceID: dnsService, routes: [gatewayRoute, excludedRoute]))),
            .exitAfterDraining,
            .exitNow,
            .log(level: .info, category: .routes, message: "gateway has no IPv4 address; its host route will not be repaired"),
            .log(level: .notice, category: .helper, message: "run \(attempt): tunnel up"),
            .log(level: .error, category: .helper, message: "cannot write the pidfile: ‹denied›"),
        ]
        // The Helper needs no Equatable messages; the description shows every case and value.
        #expect(String(describing: messages) == String(describing: expected))
    }
}
