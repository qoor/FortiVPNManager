import Foundation

/// This helper's build (`CFBundleVersion`, bumped on every `make build`); the app compares it with its own.
enum HelperBuild {
    static let id = BuildID(Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "unknown")
}

/// XPC endpoint exported on one app connection. Answers the version check itself and passes every
/// other request, unread, to the Helper entry.
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
        EntryRelay.forward(.readBypassRoutes, from: client, reply: .data(reply))
    }

    /// Bytes that are not UTF-8 reach the entry replaced, and it refuses what it cannot read.
    func setBypassRoutes(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
        let json = String(decoding: request, as: UTF8.self)
        EntryRelay.forward(.writeBypassRoutes(json: json), from: client, reply: .data(reply))
    }

    func prepareUpdate(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
        let json = String(decoding: request, as: UTF8.self)
        EntryRelay.forward(.prepareUpdate(json: json), from: client, reply: .data(reply))
    }

    func installUpdate(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
        let json = String(decoding: request, as: UTF8.self)
        EntryRelay.forward(.installUpdate(json: json), from: client, reply: .data(reply))
    }

    func preparedUpdate(reply: @escaping @Sendable (Data) -> Void) {
        EntryRelay.forward(.readPreparedUpdate, from: client, reply: .data(reply))
    }

    func discardPreparedUpdate(reply: @escaping @Sendable (Data) -> Void) {
        EntryRelay.forward(.discardPreparedUpdate, from: client, reply: .data(reply))
    }

    /// Never `nil`: a `nil` reply means success.
    private static func encode(_ error: HelperError) -> Data {
        (try? XPCCoding.encode(error)) ?? Data(#"{"code":"internalError","message":""}"#.utf8)
    }
}
