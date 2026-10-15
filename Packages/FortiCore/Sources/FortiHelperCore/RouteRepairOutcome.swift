/// Result of the helper's route repair after openfortivpn exited without restoring its routes.
public enum RouteRepairOutcome: Sendable, Equatable {
    /// The default route and/or the gateway host route were fixed.
    case repaired
    /// The routing table was already fine.
    case notNeeded
    /// A needed change could not be made (details in the helper log).
    case failed
}
