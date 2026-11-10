import Foundation
import Testing

/// Checks the Swift types in components/contract/macos against the JSON the Rust tests pin in
/// ../fixtures/helper-internal: Swift encodes what it sends and decodes what it receives.
struct HelperInternalTests {
    private let attempt = "E621E1F8-C36C-495A-93FC-0C247A3E6E5F"

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
            .appDisconnected(app: 1),
            .executableReplaced,
            .terminationRequested,
            .routesRepaired(outcome: .repaired),
            .routesRepaired(outcome: .notNeeded),
            .routesRepaired(outcome: .failed),
        ]
        let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(messages)) as? NSArray
        let expected = try JSONSerialization.jsonObject(with: fixture("to_entry.json")) as? NSArray
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
            .repairRoutes(gateway: "10.30.161.16"),
            .repairRoutes(gateway: nil),
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
