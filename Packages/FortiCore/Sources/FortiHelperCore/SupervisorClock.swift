import Dispatch
import Foundation

/// Time source for `TunnelSupervisor`: monotonic time for watchdogs, batching and line flushing,
/// wall-clock dates for events and output lines, and timers. Tests inject a manual clock.
public protocol SupervisorClock: Sendable {
    /// Monotonic time (`ContinuousClock` keeps counting while the Mac sleeps).
    var now: ContinuousClock.Instant { get }
    /// Wall-clock time stamped on `HelperEvent` and `OutputLine`.
    var date: Date { get }
    /// Runs `action` on `queue` at or after `deadline` unless the returned timer is cancelled first.
    func schedule(
        at deadline: ContinuousClock.Instant,
        on queue: DispatchQueue,
        _ action: @escaping @Sendable () -> Void
    ) -> any SupervisorTimer
}

/// A pending `SupervisorClock.schedule` action.
public protocol SupervisorTimer: Sendable {
    func cancel()
}

/// The real clock: `ContinuousClock`, `Date()` and `DispatchQueue.asyncAfter`.
///
/// Dispatch timers run on uptime, which stops during sleep, so a timer may fire late after a
/// sleep; `TunnelSupervisor` re-checks deadlines against `now` and never acts early.
public struct SystemSupervisorClock: SupervisorClock {
    public init() {}

    public var now: ContinuousClock.Instant {
        ContinuousClock.now
    }

    public var date: Date {
        Date()
    }

    public func schedule(
        at deadline: ContinuousClock.Instant,
        on queue: DispatchQueue,
        _ action: @escaping @Sendable () -> Void
    ) -> any SupervisorTimer {
        let item = DispatchWorkItem(block: action)
        let delay = max(.zero, deadline - ContinuousClock.now)
        queue.asyncAfter(deadline: .now() + delay.dispatchInterval, execute: item)
        return WorkItemTimer(item: item)
    }
}

private struct WorkItemTimer: SupervisorTimer, @unchecked Sendable {
    let item: DispatchWorkItem

    func cancel() {
        item.cancel()
    }
}

extension Duration {
    /// Nanosecond `DispatchTimeInterval`, saturating instead of overflowing.
    var dispatchInterval: DispatchTimeInterval {
        let (seconds, attoseconds) = components
        let (whole, overflow) = seconds.multipliedReportingOverflow(by: 1_000_000_000)
        guard !overflow, whole < Int64(Int.max) - 1_000_000_000 else { return .never }
        return .nanoseconds(Int(whole + attoseconds / 1_000_000_000))
    }

    /// The duration in seconds as a `TimeInterval`.
    var timeInterval: TimeInterval {
        let (seconds, attoseconds) = components
        return TimeInterval(seconds) + TimeInterval(attoseconds) / 1e18
    }
}
