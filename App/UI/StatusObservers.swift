import Foundation
import FortiCore
import FortiShared
import Observation

/// Suspends until any observable property read by `apply` changes (one `withObservationTracking` round).
func observationChange(_ apply: () -> Void) async {
    await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
        withObservationTracking(apply, onChange: { @Sendable in continuation.resume() })
    }
}

/// "Now" for the relative times in the menu and the Connection tab. Follows the controller's 30 s
/// tick and its events, and ticks every second while the retry countdown ("Waiting to reconnect (1:12)") runs.
@Observable
final class StatusClock {
    private(set) var now = Date()

    /// Runs for the app's lifetime.
    func run(following connection: ConnectionController) async {
        while !Task.isCancelled {
            now = Date()
            if let until = connection.nextRetryAt, until > now {
                try? await Task.sleep(for: .seconds(1))
            } else {
                await observationChange {
                    _ = connection.now
                    _ = connection.model
                }
            }
        }
    }
}

/// "Applies from the next connection" (DESIGN §5.6): the VPN account in config.yaml no longer matches the one the
/// running attempt or session was started with. Account edits never touch a running tunnel.
@Observable
final class AccountChangeNotice {
    private(set) var isPending = false

    /// The account the active attempt was started with.
    @ObservationIgnored private var applied: (attemptID: AttemptID, account: Account)?

    private struct Account: Equatable {
        var host: String
        var port: Int
        var username: String
        var realm: String?

        init(_ profile: VPNProfile) {
            host = profile.host
            port = profile.port
            username = profile.username
            realm = profile.realm
        }
    }

    /// Runs for the app's lifetime.
    func run(following connection: ConnectionController) async {
        while !Task.isCancelled {
            update(connection.model)
            await observationChange { _ = connection.model }
        }
    }

    private func update(_ model: ConnectionModel) {
        guard let attemptID = model.activeAttemptID else {
            applied = nil
            if isPending { isPending = false }
            return
        }
        let current = Account(model.profile)
        if applied?.attemptID != attemptID {
            applied = (attemptID, current)
        }
        let pending = applied?.account != current
        if pending != isPending { isPending = pending }
    }
}
