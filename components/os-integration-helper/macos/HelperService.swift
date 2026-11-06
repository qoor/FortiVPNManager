import Foundation
import FortiShared

/// This helper's build (`CFBundleVersion`, bumped on every `make build`); the app compares it with its own.
enum HelperBuild {
    static let id = BuildID(Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "unknown")
}

/// XPC endpoint exported on one app connection. Answers the version check and the exception-route
/// methods itself and passes every other request, unread, to the Helper entry.
final class HelperService: NSObject, HelperXPCProtocol {
    private let client: ClientLink

    init(client: ClientLink) {
        self.client = client
        super.init()
    }

    func getVersion(reply: @escaping @Sendable (Data) -> Void) {
        reply((try? XPCCoding.encode(HelperBuild.id)) ?? Data())
    }

    func connect(_ request: Data, reply: @escaping @Sendable (Data?) -> Void) {
        guard let json = String(validating: request, as: UTF8.self) else {
            reply(Self.encode(HelperError(.invalidRequest, "could not decode the connect request")))
            return
        }
        EntryRelay.forward(.connect(json: json), from: client, reply: .optionalData(reply))
    }

    func submitOTP(attemptID: String, code: String, reply: @escaping @Sendable (Data?) -> Void) {
        EntryRelay.forward(.submitOtp(attempt: attemptID, code: code), from: client, reply: .optionalData(reply))
    }

    func disconnect(attemptID: String, reason: String, reply: @escaping @Sendable (Data?) -> Void) {
        EntryRelay.forward(.disconnect(attempt: attemptID, reason: reason), from: client, reply: .optionalData(reply))
    }

    func status(reply: @escaping @Sendable (Data) -> Void) {
        EntryRelay.forward(.status, from: client, reply: .data(reply))
    }

    func exitWhenIdle(reply: @escaping @Sendable (Bool) -> Void) {
        EntryRelay.forward(.exitWhenIdle, from: client, reply: .flag(reply))
    }

    func bypassRoutes(reply: @escaping @Sendable (Data) -> Void) {
        BypassRoutes.queue.async { reply(Self.encode(BypassRoutes.read())) }
    }

    func setBypassRoutes(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
        let decoded: BypassRoutesRequest
        do {
            decoded = try XPCCoding.decode(BypassRoutesRequest.self, from: request)
        } catch {
            reply(Self.encode(.failure(HelperError(.invalidRequest, "could not decode the route list"))))
            return
        }
        BypassRoutes.queue.async { reply(Self.encode(BypassRoutes.write(decoded))) }
    }

    /// Never `nil`: a `nil` reply means success.
    private static func encode(_ error: HelperError) -> Data {
        (try? XPCCoding.encode(error)) ?? Data(#"{"code":"internalError","message":""}"#.utf8)
    }

    private static func encode(_ reply: BypassRoutesReply) -> Data {
        (try? XPCCoding.encode(reply)) ?? Data(#"{"failure":{"_0":{"code":"internalError","message":""}}}"#.utf8)
    }
}
