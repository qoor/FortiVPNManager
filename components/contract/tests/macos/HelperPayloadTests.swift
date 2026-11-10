import Foundation
import Testing

/// Checks the XPC payloads the Helper's Swift side reads and writes itself against the JSON the
/// Rust tests pin in ../fixtures/app-helper: the bypass-route request it decodes, and the replies
/// and errors it encodes.
struct HelperPayloadTests {
    private let snapshot = BypassRoutesSnapshot(
        serviceID: "8A3F1C2E-5B6D-4E7F-9A0B-1C2D3E4F5A6B", serviceName: "Ethernet", router: "10.20.161.1",
        address: "10.20.161.20", subnetMask: "255.255.255.0",
        routes: [
            AdditionalRoute(destination: "10.30.161.16", subnetMask: "255.255.255.255", gateway: "10.20.161.1"),
            AdditionalRoute(destination: "192.168.50.0", subnetMask: "255.255.255.0", gateway: "10.20.161.254"),
        ]
    )

    private func fixture(_ name: String) throws -> Data {
        let tests = URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        return try Data(contentsOf: tests.appending(path: "fixtures/app-helper/\(name)"))
    }

    private func json(_ data: Data) throws -> NSObject? {
        try JSONSerialization.jsonObject(with: data) as? NSObject
    }

    @Test func bypassRoutesRequest() throws {
        let request = try XPCCoding.decode(BypassRoutesRequest.self, from: fixture("bypass_routes_request.json"))
        let routes = [
            AdditionalRoute(destination: "10.30.170.0", subnetMask: "255.255.255.0", gateway: "10.20.161.1"),
            AdditionalRoute(destination: "192.168.50.0", subnetMask: "255.255.255.0", gateway: "10.20.161.254"),
        ]
        #expect(request == BypassRoutesRequest(expected: snapshot, routes: routes))
    }

    @Test func bypassRoutesReplies() throws {
        let failure = HelperError(.wrongPhase, "the routes changed since they were read; read them again")
        let replies: [(BypassRoutesReply, String)] = [
            (.success(snapshot), "bypass_routes_reply_success.json"),
            (.failure(failure), "bypass_routes_reply_failure.json"),
        ]
        for (reply, name) in replies {
            let encoded = try json(XPCCoding.encode(reply))
            #expect(try encoded != nil && encoded == json(fixture(name)), "\(name)")
        }
    }

    @Test func helperErrors() throws {
        let errors = [
            HelperError(.busy, "openfortivpn is already running"),
            HelperError(.invalidRequest, "could not decode the connect request"),
            HelperError(.unknownAttempt, "not the active connection attempt"),
            HelperError(.wrongPhase, "openfortivpn is not waiting for an OTP"),
            HelperError(
                .binaryUnavailable,
                "openfortivpn is not executable: /opt/homebrew/Cellar/openfortivpn/1.24.1/bin/openfortivpn"
            ),
            HelperError(.notAuthorized, "the VPN run belongs to another user"),
            HelperError(.internalError, "could not create the openfortivpn config file: permission denied"),
        ]
        let encoded = try json(XPCCoding.encode(errors))
        #expect(try encoded != nil && encoded == json(fixture("helper_errors.json")))
    }
}
