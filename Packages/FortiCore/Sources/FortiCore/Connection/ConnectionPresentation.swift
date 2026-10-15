import Foundation

/// Menu bar icon (§5.6).
public enum ConnectionIcon: String, Sendable, Equatable {
    case connected
    case disconnected
    case inProgress
    case error
}

/// Which menu items are shown or enabled (§5.6).
public struct ConnectionMenuActions: Sendable, Equatable {
    /// "Connect": desired is off, or the state is `failed` / `needsSetup`.
    public var showsConnect: Bool
    /// "Disconnect": desired is on.
    public var showsDisconnect: Bool
    /// "Reconnect Now": enabled while `connected` or `waitingRetry`.
    public var reconnectNowEnabled: Bool

    public init(showsConnect: Bool, showsDisconnect: Bool, reconnectNowEnabled: Bool) {
        self.showsConnect = showsConnect
        self.showsDisconnect = showsDisconnect
        self.reconnectNowEnabled = reconnectNowEnabled
    }
}

/// The one-line reason shown with "Open Settings" when the icon is in the error state.
public enum ConnectionAttention: Sendable, Equatable {
    case needsSetup([SetupItem])
    case failed(ConnectionFailureReason)
}

extension ConnectionState {
    public var icon: ConnectionIcon {
        switch self {
        case .connected: .connected
        case .idle: .disconnected
        case .connecting, .waitingRetry, .waitingNetwork, .disconnecting: .inProgress
        case .failed, .needsSetup: .error
        }
    }
}

extension ConnectionModel {
    public var icon: ConnectionIcon { state.icon }

    public var menuActions: ConnectionMenuActions {
        ConnectionMenuActions(
            showsConnect: desired == .disconnected || attention != nil,
            showsDisconnect: desired == .connected,
            reconnectNowEnabled: {
                switch state {
                case .connected, .waitingRetry: true
                default: false
                }
            }()
        )
    }

    public var attention: ConnectionAttention? {
        switch state {
        case let .needsSetup(items): .needsSetup(items)
        case let .failed(reason): .failed(reason)
        default: nil
        }
    }

    /// Next proactive reconnect while connected (shown in the connection tab).
    public var nextPlannedReconnect: Date? {
        guard case let .connected(context) = state else { return nil }
        return context.plannedReconnectAt
    }

    /// When the next automatic attempt is due while waiting, if known.
    public var nextRetryAt: Date? {
        guard case let .waitingRetry(wait) = state else { return nil }
        return wait.until
    }
}
