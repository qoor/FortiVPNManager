import Foundation

/// One accepted app connection: its number in messages to the Helper entry, who is calling, and
/// the callback channel (`event`/`output`) for the runs it starts.
final class ClientLink: @unchecked Sendable {
    // @unchecked: the other properties are immutable and the weak reference is only changed by ARC.

    let id: UInt64
    /// Effective uid of the caller (its audit token, via `NSXPCConnection`).
    let uid: uid_t
    let pid: pid_t
    /// Weak: the connection owns the exported `HelperService`, which owns this link.
    private weak var connection: NSXPCConnection?

    init(id: UInt64, connection: NSXPCConnection) {
        self.id = id
        uid = connection.effectiveUserIdentifier
        pid = connection.processIdentifier
        self.connection = connection
    }

    /// An encoded `HelperEvent`.
    func send(event: Data) {
        proxy()?.event(event)
    }

    /// An encoded `[OutputLine]` batch.
    func send(output: Data) {
        proxy()?.output(output)
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
}
