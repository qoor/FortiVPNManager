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

/// What the schedule does next (menu and Connection tab).
public enum ScheduleOutlook: Sendable, Equatable {
    /// Not connecting now: the next scheduled start connects.
    case connects(at: Date)
    /// Not connecting now, and no weekday is selected.
    case noStart
    /// Inside the scheduled hours; after `end`, reconnects wait while the user is away.
    case withinHours(end: Date)
    /// Outside the scheduled hours with the user present: reconnects continue until the user leaves.
    case outsideHoursPresent
    /// Outside the scheduled hours with the user away: reconnects wait, and stop for good at `until`.
    case holdingWhileAway(until: Date)
    /// Reconnects stopped after the user was away too long; the current connection is kept until it ends.
    case suspended
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
        // Waiting for an away user (schedule) is not a reconnect in progress.
        case let .waitingRetry(wait) where wait.reason == .userAway: .disconnected
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

    /// `nil` when there is no schedule.
    public func scheduleOutlook(now: Date) -> ScheduleOutlook? {
        guard let schedule = settings.schedule else { return nil }
        let wantsConnection = switch state {
        case .failed: false
        default: desired == .connected
        }
        guard wantsConnection else {
            return schedule.nextStart(after: now, calendar: calendar).map(ScheduleOutlook.connects(at:)) ?? .noStart
        }
        if schedule.isActive(at: now, calendar: calendar),
           let next = schedule.nextBoundary(after: now, calendar: calendar), next.edge == .end {
            return .withinHours(end: next.date)
        }
        if reconnectSuspended { return .suspended }
        if let awaySince { return .holdingWhileAway(until: awaySince.addingTimeInterval(schedule.awayLimit)) }
        return .outsideHoursPresent
    }
}
