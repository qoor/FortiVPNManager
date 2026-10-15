import Foundation
import FortiShared

/// This helper's build (`CFBundleVersion`, bumped on every `make build`); the app compares it with its own.
enum HelperBuild {
    static let id = BuildID(Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "unknown")
}

/// XPC endpoint exported on one app connection. Decodes requests and forwards them, together with
/// the caller's `ClientLink`, to the shared `TunnelController`.
final class HelperService: NSObject, HelperXPCProtocol {
    private let controller: TunnelController
    private let client: ClientLink

    init(controller: TunnelController, client: ClientLink) {
        self.controller = controller
        self.client = client
        super.init()
    }

    func getVersion(reply: @escaping @Sendable (Data) -> Void) {
        reply((try? XPCCoding.encode(HelperBuild.id)) ?? Data())
    }

    func connect(_ request: Data, reply: @escaping @Sendable (Data?) -> Void) {
        let decoded: ConnectRequest
        do {
            decoded = try XPCCoding.decode(ConnectRequest.self, from: request)
        } catch {
            reply(Self.encode(HelperError(.invalidRequest, "could not decode the connect request")))
            return
        }
        reply(controller.connect(decoded, from: client).map(Self.encode))
    }

    func submitOTP(attemptID: String, code: String, reply: @escaping @Sendable (Data?) -> Void) {
        reply(controller.submitOTP(attemptID: attemptID, code: code, from: client).map(Self.encode))
    }

    func disconnect(attemptID: String, reason: String, reply: @escaping @Sendable (Data?) -> Void) {
        reply(controller.disconnect(attemptID: attemptID, reason: reason, from: client).map(Self.encode))
    }

    func status(reply: @escaping @Sendable (Data) -> Void) {
        reply((try? XPCCoding.encode(controller.status())) ?? Data())
    }

    func exitWhenIdle(reply: @escaping @Sendable (Bool) -> Void) {
        reply(controller.exitWhenIdle())
    }

    /// Never `nil`: a `nil` reply means success.
    private static func encode(_ error: HelperError) -> Data {
        (try? XPCCoding.encode(error)) ?? Data(#"{"code":"internalError","message":""}"#.utf8)
    }
}
