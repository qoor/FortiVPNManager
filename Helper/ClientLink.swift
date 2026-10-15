import Foundation
import FortiShared

/// One accepted app connection: who is calling, and the callback channel (`event`/`output`) for
/// the runs it starts. Callbacks travel in FIFO order on the connection.
final class ClientLink: @unchecked Sendable {
    // @unchecked: `uid`/`pid` are immutable and the weak reference is only changed by ARC.

    /// Effective uid of the caller (its audit token, via `NSXPCConnection`).
    let uid: uid_t
    let pid: pid_t
    /// Weak: the connection owns the exported `HelperService`, which owns this link.
    private weak var connection: NSXPCConnection?

    init(connection: NSXPCConnection) {
        uid = connection.effectiveUserIdentifier
        pid = connection.processIdentifier
        self.connection = connection
    }

    func send(_ event: HelperEvent) {
        guard let data = Self.encode(event) else { return }
        proxy()?.event(data)
    }

    func send(_ lines: [OutputLine]) {
        guard let data = Self.encode(lines) else { return }
        proxy()?.output(data)
    }

    /// Runs `block` once everything sent so far has been handed to the kernel (immediately if
    /// the connection is gone).
    func afterPendingSends(_ block: @escaping @Sendable () -> Void) {
        guard let connection else {
            block()
            return
        }
        connection.scheduleSendBarrierBlock(block)
    }

    private func proxy() -> (any HelperClientXPCProtocol)? {
        connection?.remoteObjectProxyWithErrorHandler { @Sendable error in
            HelperLog.helper.debug("callback to the app failed: \(error.localizedDescription, privacy: .public)")
        } as? any HelperClientXPCProtocol
    }

    private static func encode<T: Encodable>(_ value: T) -> Data? {
        do {
            return try XPCCoding.encode(value)
        } catch {
            HelperLog.helper.error("cannot encode a callback: \(error.localizedDescription, privacy: .public)")
            return nil
        }
    }
}
