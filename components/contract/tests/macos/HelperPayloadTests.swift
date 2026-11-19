import Foundation
import Testing

/// Checks the XPC payload the Helper's Swift side writes itself, a `HelperError` for a request it
/// cannot pass on, against the JSON the Rust tests pin in ../fixtures/app-helper.
struct HelperPayloadTests {
    private func fixture(_ name: String) throws -> Data {
        let tests = URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        return try Data(contentsOf: tests.appending(path: "fixtures/app-helper/\(name)"))
    }

    private func json(_ data: Data) throws -> NSObject? {
        try JSONSerialization.jsonObject(with: data) as? NSObject
    }

    @Test func helperErrors() throws {
        let errors = [
            HelperError(.busy, "openfortivpn is already running"),
            HelperError(.invalidRequest, "could not decode the connect request"),
            HelperError(.unknownAttempt, "not the active connection attempt"),
            HelperError(.wrongPhase, "openfortivpn is not waiting for an OTP"),
            HelperError(.notAuthorized, "the VPN run belongs to another user"),
            HelperError(.internalError, "could not create the openfortivpn config file: permission denied"),
        ]
        let encoded = try json(XPCCoding.encode(errors))
        #expect(try encoded != nil && encoded == json(fixture("helper_errors.json")))
    }
}
