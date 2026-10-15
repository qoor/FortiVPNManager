import Foundation
import FortiCore

/// Wall-clock timers for the reducer's `scheduleTimer` / `cancelTimer` effects: at most one pending
/// task per `ConnectionTimer`, each sleeping until its `Date`. Scheduling an ID again replaces it.
/// After a wake the remaining delays are recomputed from the wall clock (`rearmAll`).
final class ConnectionTimers {
    /// Called on the main actor when a timer is due, with the generation it was scheduled with.
    var onFire: ((ConnectionTimer, UInt64) -> Void)?

    private struct Pending {
        var date: Date
        var generation: UInt64
        /// Identifies the task currently armed for this entry (a replaced task must not fire).
        var token: UInt64
        var task: Task<Void, Never>
    }

    private var pending: [ConnectionTimer: Pending] = [:]
    private var nextToken: UInt64 = 0

    func schedule(_ timer: ConnectionTimer, at date: Date, generation: UInt64) {
        pending[timer]?.task.cancel()
        nextToken += 1
        let token = nextToken
        pending[timer] = Pending(date: date, generation: generation, token: token, task: arm(timer, at: date, token: token))
    }

    func cancel(_ timer: ConnectionTimer) {
        pending.removeValue(forKey: timer)?.task.cancel()
    }

    /// Restarts every pending timer from the wall clock (sleep or a clock change can skew task sleeps).
    func rearmAll() {
        for (timer, entry) in pending {
            schedule(timer, at: entry.date, generation: entry.generation)
        }
    }

    private func arm(_ timer: ConnectionTimer, at date: Date, token: UInt64) -> Task<Void, Never> {
        Task { [weak self] in
            let delay = date.timeIntervalSinceNow
            if delay > 0 {
                do { try await Task.sleep(for: .seconds(delay)) } catch { return }
            }
            self?.fire(timer, token: token)
        }
    }

    private func fire(_ timer: ConnectionTimer, token: UInt64) {
        guard let entry = pending[timer], entry.token == token else { return }
        pending[timer] = nil
        onFire?(timer, entry.generation)
    }
}
