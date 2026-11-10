import Foundation

/// Methods the app calls on the root helper. Payloads are `XPCCoding`-encoded `Data` of the types in
/// components/contract/src/app_helper.rs. Replies carry either an encoded success payload or an
/// encoded `HelperError`.
///
/// Ordering note: replies are not ordered with respect to `HelperClientXPCProtocol` messages.
/// Treat the `exited` event (not a reply) as the signal that a run has finished.
@objc protocol HelperXPCProtocol {
    /// Reply: encoded `BuildID`.
    func getVersion(reply: @escaping @Sendable (Data) -> Void)
    /// Request: encoded `RunRequest`. Reply: `nil` on success or encoded `HelperError`.
    func connect(_ request: Data, reply: @escaping @Sendable (Data?) -> Void)
    /// Reply: `nil` on success or encoded `HelperError`.
    func submitOTP(attemptID: String, code: String, reply: @escaping @Sendable (Data?) -> Void)
    /// `reason` is a `StopReason` raw value.
    /// Reply: `nil` once the stop has been accepted (not when it has finished).
    func disconnect(attemptID: String, reason: String, reply: @escaping @Sendable (Data?) -> Void)
    /// Reply: encoded `HelperStatus`.
    func status(reply: @escaping @Sendable (Data) -> Void)
    /// Asks an idle helper to exit so launchd relaunches the (updated) embedded binary.
    /// Reply: `true` if it will exit. Sent to a helper of another build, like `getVersion`, so the
    /// signatures of these two must never change.
    func exitWhenIdle(reply: @escaping @Sendable (Bool) -> Void)
    /// Reads the additional routes of the primary network service. Reply: encoded `BypassRoutesReply`.
    func bypassRoutes(reply: @escaping @Sendable (Data) -> Void)
    /// Request: encoded `BypassRoutesRequest`. Replaces the additional routes of the primary network
    /// service. Reply: encoded `BypassRoutesReply` with the routes read back afterwards.
    func setBypassRoutes(_ request: Data, reply: @escaping @Sendable (Data) -> Void)
}

/// Methods the helper calls back on the app. Both travel on the same connection in FIFO order.
@objc protocol HelperClientXPCProtocol {
    /// Encoded `HelperEvent`.
    func event(_ data: Data)
    /// Encoded `[OutputLine]` batch.
    func output(_ data: Data)
}

/// JSON coding shared by both sides of the XPC connection.
enum XPCCoding {
    static func encode<T: Encodable>(_ value: T) throws -> Data {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        return try encoder.encode(value)
    }

    static func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return try decoder.decode(type, from: data)
    }
}
