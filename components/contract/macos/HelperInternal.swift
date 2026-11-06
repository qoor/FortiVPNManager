/// Messages between the Helper's Swift side and the Helper entry (Rust), defined in
/// components/contract/src/helper_internal.rs; case and label names are the JSON keys.
nonisolated enum HelperInternal {
    /// Sent to Rust.
    enum ToEntry: Encodable, Sendable {
        /// An app connection passed the signature check; `app` names it in later messages.
        case appConnected(app: UInt64, uid: UInt32)
        /// The connection is gone, whether the app quit, crashed or was killed.
        case appDisconnected(app: UInt64)
        /// `id` names the request in its reply.
        case request(id: UInt64, app: UInt64, request: AppRequest)
        /// Once the self-update watcher has seen the executable replaced, sent before announcing
        /// any new app connection, so Rust can refuse that connection.
        case executableReplaced
        case terminationRequested
        /// Answers `FromEntry.repairRoutes`.
        case routesRepaired(outcome: RouteRepairOutcome)
    }

    /// An app request with its values exactly as the app sent them; Rust checks them.
    enum AppRequest: Encodable, Sendable {
        /// `json` is the app's connect payload.
        case connect(json: String)
        case submitOtp(attempt: String, code: String)
        case disconnect(attempt: String, reason: String)
        case status
        case exitWhenIdle
    }

    enum RouteRepairOutcome: String, Encodable, Sendable {
        case repaired
        case notNeeded
        case failed
    }

    /// Sent by Rust. Events and output go to each app connection in the order they arrive, because
    /// the app reads a run's end from the last event, not from a reply.
    enum FromEntry: Decodable, Sendable {
        /// Leftovers of an earlier Helper are cleaned up; accept app connections from now on.
        case ready
        /// Answers a connect, submitOtp, disconnect or status request; nil is the empty reply.
        case reply(id: UInt64, json: String?)
        /// Answers an exitWhenIdle request.
        case replyFlag(id: UInt64, value: Bool)
        case sendEvent(app: UInt64, json: String)
        case sendOutput(app: UInt64, json: String)
        /// Check and repair the routing table; `gateway` is the VPN gateway's IPv4 address, if known.
        case repairRoutes(gateway: String?)
        /// Accept no more connections and exit once what was sent to apps has left.
        case exitAfterDraining
        /// Exit at once, even if something sent to apps has not left yet.
        case exitNow
        case log(level: LogLevel, category: LogCategory, message: String)
    }

    enum LogLevel: String, Decodable, Sendable {
        case info
        case notice
        case error
    }

    /// The unified-log category, kept from the Swift Helper so existing log filters still match.
    enum LogCategory: String, Decodable, Sendable {
        case helper
        case routes
    }
}
