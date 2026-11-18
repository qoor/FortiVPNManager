/// Messages between the Helper's Swift side and the Helper entry (Rust), defined in
/// components/contract/src/helper_internal.rs; case and label names are the JSON keys.
nonisolated enum HelperInternal {
    /// Sent to Rust.
    enum ToEntry: Encodable, Sendable {
        /// An app connection passed the signature check; `app` names it in later messages. Held
        /// until `FromEntry.admitted` answers.
        case appConnected(app: UInt64, uid: UInt32)
        /// The connection is gone, whether the app quit, crashed or was killed.
        case appDisconnected(app: UInt64)
        /// `id` names the request in its reply.
        case request(id: UInt64, app: UInt64, request: AppRequest)
        /// Once the self-update watcher has seen the executable replaced, sent before announcing
        /// any new app connection, so Rust can refuse that connection.
        case executableReplaced
        case terminationRequested
        /// Answers `FromEntry.changeBypassRoutes`.
        case bypassRoutesChanged(outcome: BypassRoutesOutcome)
        case networkChanged(outcome: NetworkOutcome)
    }

    /// An app request with its values exactly as the app sent them; Rust checks them.
    enum AppRequest: Encodable, Sendable {
        /// `json` is the app's connect payload.
        case connect(json: String)
        case submitOtp(attempt: String, code: String)
        case disconnect(attempt: String, reason: String)
        case status
        case exitWhenIdle
        case readBypassRoutes
        /// `json` is the app's exception-route request.
        case writeBypassRoutes(json: String)
    }

    /// What came of a `BypassRoutesChange`.
    enum BypassRoutesOutcome: Encodable, Sendable {
        /// `network` is the primary network as read, nil when skipped; `added` the routes to add
        /// that the table did not have and has now; `found` the routes to look up that it has;
        /// `left` the routes to remove that it still has.
        case changed(network: BypassRoutesSnapshot?, added: [BypassRoute], found: [BypassRoute], left: [AddedRoute])
        /// Nothing was changed: the table or the primary network could not be read, there is no
        /// primary network, it is not the one expected, or a route to add is not one the Helper
        /// writes.
        case refused(error: HelperError)
    }

    /// An exception route the Helper added, with the network service it went through.
    struct AddedRoute: Codable, Sendable, Hashable {
        var route: BypassRoute
        var serviceID: String
    }

    /// Sent by Rust. Events and output go to each app connection in the order they arrive, because
    /// the app reads a run's end from the last event, not from a reply.
    enum FromEntry: Decodable, Sendable {
        /// Leftovers of an earlier Helper are cleaned up; accept app connections from now on.
        case ready
        /// Answers `ToEntry.appConnected`. A refused connection is dropped without `appDisconnected`.
        case admitted(app: UInt64, accept: Bool)
        /// Answers a connect, submitOtp, disconnect, status or exception-route request; nil is the
        /// empty reply.
        case reply(id: UInt64, json: String?)
        /// Answers an exitWhenIdle request.
        case replyFlag(id: UInt64, value: Bool)
        case sendEvent(app: UInt64, json: String)
        case sendOutput(app: UInt64, json: String)
        /// Change the exception routes in the routing table.
        case changeBypassRoutes(change: BypassRoutesChange)
        case changeNetwork(change: NetworkChange)
        /// Accept no more connections and exit once what was sent to apps has left.
        case exitAfterDraining
        /// Exit at once, even if something sent to apps has not left yet.
        case exitNow
        case log(level: LogLevel, category: LogCategory, message: String)
    }

    /// Exception route work: read the routing table once, and the primary network unless `network`
    /// skips it; then remove and add; then, if anything changed, read the table once more. A route
    /// counts as in the table only as the Helper adds one: static, for every interface, with the
    /// same destination, subnet mask and gateway.
    struct BypassRoutesChange: Decodable, Sendable {
        var network: NetworkCheck
        /// Routes the Helper added, each removed where the table has it.
        var remove: [AddedRoute]
        /// Each added where the table does not have it yet; checked first, since they come from an
        /// app.
        var add: [BypassRoute]
        /// Looked up at the end; none without a network.
        var lookUp: [BypassRoute]
    }

    enum NetworkCheck: Decodable, Sendable {
        /// Only removing: the network does not matter.
        case skip
        case read
        /// Change nothing unless the primary network is still exactly `network`.
        case expect(network: BypassRoutesSnapshot)
    }

    enum NetworkChange: Decodable, Sendable {
        case lookUp(tunnel: TunnelDevice, outside: [IPNetwork])
        case apply(
            settings: NetworkSettings, previous: NetworkSettings?, remove: [OutsideRoute], add: [OutsideRoute],
            outside: [IPNetwork]
        )
        case prepare(settings: NetworkSettings, add: [OutsideRoute])
        case `switch`(settings: NetworkSettings, replaced: NetworkSettings, outside: [IPNetwork])
        case revert(changes: OutsideChanges)
        case check(settings: NetworkSettings, outside: [IPNetwork])
    }

    struct NetworkSettings: Decodable, Sendable {
        var tunnel: TunnelDevice
        var gateway: String
        var mtu: UInt32?
        var ipv4: FamilySettings
        var ipv6: FamilySettings?
        var splitExclude: [IPNetwork]
        var dnsServers: [String]
        var defaultDomains: [String]
        var splitDnsDomains: [String]
        var winsServers: [String]
    }

    struct TunnelDevice: Decodable, Sendable {
        var name: String
        var index: UInt32?
    }

    struct FamilySettings: Decodable, Sendable {
        var address: IPNetwork
        var routes: TunnelRoutes
    }

    enum TunnelRoutes: Decodable, Sendable {
        case full
        case split(include: [IPNetwork])
    }

    struct IPNetwork: Codable, Sendable, Hashable {
        var address: String
        var prefixLength: Int
    }

    struct OutsideRoute: Codable, Sendable, Hashable {
        var destination: IPNetwork
        var router: String
        var serviceID: String
    }

    struct OutsideChanges: Decodable, Sendable {
        var dnsServiceID: String
        var routes: [OutsideRoute]
    }

    enum NetworkOutcome: Encodable, Sendable {
        case lookedUp(dnsServiceID: String, missing: [OutsideRoute])
        case applied(added: [OutsideRoute], left: [OutsideRoute])
        case reverted(left: [OutsideRoute])
        case checked(complete: Bool)
        case failed(step: NetworkStep, error: String, added: [OutsideRoute], left: [OutsideRoute])
    }

    enum NetworkStep: String, Encodable, Sendable {
        case network
        case addresses
        case mtu
        case routes
        case outsideRoutes
        case dns
        case wins
    }

    enum LogLevel: String, Decodable, Sendable {
        case info
        case notice
        case error
    }

    /// The unified-log category.
    enum LogCategory: String, Decodable, Sendable {
        case helper
        case routes
    }
}
