import Foundation

/// Methods the app calls on the root helper. Payloads are `XPCCoding`-encoded `Data`.
/// Replies carry either an encoded success payload or an encoded `HelperError`.
///
/// Ordering note: replies are not ordered with respect to `HelperClientXPCProtocol` messages.
/// Treat `HelperEvent.Kind.exited` (not a reply) as the signal that a run has finished.
@objc public protocol HelperXPCProtocol {
    /// Reply: encoded `BuildID`.
    func getVersion(reply: @escaping @Sendable (Data) -> Void)
    /// Request: encoded `ConnectRequest`. Reply: `nil` on success or encoded `HelperError`.
    func connect(_ request: Data, reply: @escaping @Sendable (Data?) -> Void)
    /// Reply: `nil` on success or encoded `HelperError`.
    func submitOTP(attemptID: String, code: String, reply: @escaping @Sendable (Data?) -> Void)
    /// `attemptID` may be empty to stop whatever is running. `reason` is a `StopReason` raw value.
    /// Reply: `nil` once the stop has been accepted (not when it has finished).
    func disconnect(attemptID: String, reason: String, reply: @escaping @Sendable (Data?) -> Void)
    /// Reply: encoded `HelperStatus`.
    func status(reply: @escaping @Sendable (Data) -> Void)
    /// Asks an idle helper to exit so launchd relaunches the (updated) embedded binary.
    func exitWhenIdle(reply: @escaping @Sendable (Bool) -> Void)
}

/// Methods the helper calls back on the app. Both travel on the same connection in FIFO order.
@objc public protocol HelperClientXPCProtocol {
    /// Encoded `HelperEvent`.
    func event(_ data: Data)
    /// Encoded `[OutputLine]` batch.
    func output(_ data: Data)
}

/// JSON coding shared by both sides of the XPC connection.
public enum XPCCoding {
    public static func encode<T: Encodable>(_ value: T) throws -> Data {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        return try encoder.encode(value)
    }

    public static func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return try decoder.decode(type, from: data)
    }
}
