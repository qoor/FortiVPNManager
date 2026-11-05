import FortiConfig
import FortiCore
import FortiShared
import Observation
import os

/// Exception Rules: while this Mac is inside one of `routes.networks`, keeps the primary network
/// service's additional routes equal to `routes.bypass_vpn` (through its router) and imports routes
/// added there outside the app; elsewhere removes the routes it wrote, so everything goes through the
/// VPN. Routes through other gateways are never touched. Also owns the Connection tab's drafts of both
/// lists (saved only by Apply).
@Observable
final class BypassRouteService {
    /// What the last successful sync found and did.
    struct SyncResult: Equatable {
        var snapshot: BypassRoutesSnapshot
        /// The entry of `routes.networks` this Mac is in; `nil` when the rules do not apply here.
        var matchedNetwork: IPv4Network?
        /// Routes through other gateways left as they are.
        var keptCount: Int
        var notImported: Int
        var onLinkCount: Int
    }

    enum Status: Equatable {
        /// Not read yet.
        case unknown
        /// config.yaml does not exist yet; Apply creates it.
        case noConfigFile
        case checking
        case synced(SyncResult)
        /// config.yaml does not decode: the routes are left as they are.
        case configInvalid
        /// No primary network with an IPv4 router and address right now.
        case noNetwork
        /// Localized explanation.
        case failed(String)
    }

    private(set) var status: Status = .unknown
    /// The primary network as last read (the Connection tab's "Current network" line).
    private(set) var currentNetwork: BypassRoutesSnapshot?
    /// Rows being edited; kept while the settings window is closed.
    private(set) var networksDraft = NetworkListDraft()
    private(set) var targetsDraft = NetworkListDraft()
    /// Why the last Apply could not save the lists.
    private(set) var applyError: String?
    /// Failed writes in a row (reset by a successful sync).
    private(set) var writeFailureStreak = 0
    /// A failed write is tried again shortly.
    private(set) var retryPending = false

    var hasChanges: Bool { networksDraft.hasChanges || targetsDraft.hasChanges }
    var canApply: Bool { hasChanges && networksDraft.entries != nil && targetsDraft.entries != nil }
    /// The retries ran out; the next network or rules change tries once more.
    var stoppedRetrying: Bool { writeFailureStreak > Self.maxWriteRetries }
    /// This Mac's network is known and no row names or overlaps it yet.
    var canAddCurrentNetwork: Bool {
        guard let network = currentNetwork?.localNetwork else { return false }
        return !networksDraft.hasRow(overlapping: network)
    }

    @ObservationIgnored private let configStore: ConfigStore
    @ObservationIgnored private let stateStore: StateStore
    @ObservationIgnored private let helper: HelperClient
    @ObservationIgnored private let notifier: UserNotifier
    @ObservationIgnored private let watcher = NetworkSettingsWatcher()
    @ObservationIgnored private var isSyncing = false
    @ObservationIgnored private var syncRequested = false
    @ObservationIgnored private var settleTask: Task<Void, Never>?
    @ObservationIgnored private var helperTask: Task<Void, Never>?
    @ObservationIgnored private var retryTask: Task<Void, Never>?
    @ObservationIgnored private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "bypass-routes")

    /// Network settings often change in bursts (one apply touches several keys).
    static let settleDelay: Duration = .seconds(1)
    static let writeRetryDelay: Duration = .seconds(10)
    static let maxWriteRetries = 3

    init(configStore: ConfigStore, stateStore: StateStore, helper: HelperClient, notifier: UserNotifier) {
        self.configStore = configStore
        self.stateStore = stateStore
        self.helper = helper
        self.notifier = notifier
    }

    /// Call once, after `ConfigStore.start()`.
    func start(following readiness: ReadinessChecker) {
        let routes = configStore.config.routes
        networksDraft = NetworkListDraft(saved: routes.networks)
        targetsDraft = NetworkListDraft(saved: routes.bypassVPN)
        watcher.onChange = { [weak self] in self?.syncSoon() }
        watcher.start()
        sync()
        // A helper registered, approved or updated later: sync as soon as it answers with this build.
        helperTask = Task { [weak self, readiness] in
            var wasReady = readiness.helperHealth?.buildMatches == true
            while !Task.isCancelled {
                await observationChange { _ = readiness.helperHealth }
                let ready = readiness.helperHealth?.buildMatches == true
                if ready, !wasReady { self?.sync() }
                wasReady = ready
            }
        }
    }

    /// `ConfigStore.onChange`.
    func configDidChange(_ change: ConfigChange) {
        let old = change.old.routes
        let new = change.new.routes
        if new.networks != old.networks { networksDraft.merge(saved: new.networks) }
        if new.bypassVPN != old.bypassVPN { targetsDraft.merge(saved: new.bypassVPN) }
        if new != old || change.validityChanged {
            sync()
        }
    }

    // MARK: - Drafts

    func setNetworkRows(_ rows: [String]) {
        networksDraft.setRows(rows)
        applyError = nil
    }

    func setTargetRows(_ rows: [String]) {
        targetsDraft.setRows(rows)
        applyError = nil
    }

    /// Adds the network this Mac is on now (e.g. `10.20.161.0/24`) to the networks draft.
    func addCurrentNetwork() {
        guard canAddCurrentNetwork, let network = currentNetwork?.localNetwork else { return }
        networksDraft.add(network)
        applyError = nil
    }

    func discardChanges() {
        networksDraft.discardChanges()
        targetsDraft.discardChanges()
        applyError = nil
    }

    /// Saves both drafts to the `routes` section; the resulting config change starts the sync.
    func apply() {
        guard let networks = networksDraft.entries, let targets = targetsDraft.entries else {
            applyError = networksDraft.firstProblemMessage ?? targetsDraft.firstProblemMessage
            return
        }
        do throws(ConfigStoreError) {
            let saved = try configStore.update {
                $0.routes.networks = networks
                $0.routes.bypassVPN = targets
            }
            networksDraft = NetworkListDraft(saved: saved.routes.networks)
            targetsDraft = NetworkListDraft(saved: saved.routes.bypassVPN)
            applyError = nil
        } catch {
            applyError = error.message
        }
    }

    // MARK: - Sync

    /// Reads the service's routes and brings them (and the target list) up to date.
    func sync() {
        if isSyncing {
            syncRequested = true
            return
        }
        isSyncing = true
        Task { [weak self] in
            guard let self else { return }
            repeat {
                syncRequested = false
                await syncOnce()
            } while syncRequested
            isSyncing = false
        }
    }

    private func syncSoon() {
        settleTask?.cancel()
        settleTask = Task { [weak self] in
            try? await Task.sleep(for: Self.settleDelay)
            guard !Task.isCancelled else { return }
            self?.sync()
        }
    }

    private func setStatus(_ new: Status) {
        if status != new { status = new }
    }

    private func setCurrentNetwork(_ new: BypassRoutesSnapshot?) {
        if currentNetwork != new { currentNetwork = new }
    }

    private func setRetryPending(_ new: Bool) {
        if retryPending != new { retryPending = new }
    }

    private func syncOnce() async {
        guard configStore.fileExists else {
            setStatus(.noConfigFile)
            return
        }
        guard configStore.isValid else {
            setStatus(.configInvalid)
            return
        }
        // A result already shown (and its warning) stays until this one is known.
        switch status {
        case .synced, .failed: break
        default: setStatus(.checking)
        }
        var writing = false
        do {
            let snapshot = try await helper.bypassRoutes()
            setCurrentNetwork(snapshot)
            // Read after the round trip so the plan sees the latest file.
            let routes = configStore.config.routes
            let plan = BypassRoutePlan.plan(
                networks: routes.networks,
                targets: routes.bypassVPN,
                previouslyApplied: stateStore.appliedExceptionRoutes(serviceID: snapshot.serviceID),
                snapshot: snapshot)
            if !plan.imported.isEmpty {
                // Saved before anything is recorded as applied: if this fails, the next sync still sees
                // them as added outside the app instead of removed from the list.
                writing = true
                try configStore.update { config in
                    for network in plan.imported where !config.routes.bypassVPN.contains(network) {
                        config.routes.bypassVPN.append(network)
                    }
                }
                logger.notice("added \(plan.imported.count) route(s) found on \(snapshot.serviceName, privacy: .public) to routes.bypass_vpn")
            }
            var current = snapshot
            if plan.needsWrite {
                writing = true
                current = try await helper.setBypassRoutes(BypassRoutesRequest(expected: snapshot, routes: plan.routes))
                guard current.routes.count == plan.routes.count, Set(current.routes) == Set(plan.routes) else {
                    logger.error("\(snapshot.serviceName, privacy: .public) did not keep the routes that were written")
                    writeFailed(String(localized: "macOS did not keep the routes that were written."))
                    return
                }
                logger.notice("wrote \(plan.applied.count) exception route(s) to \(snapshot.serviceName, privacy: .public) (rules \(plan.matchedNetwork == nil ? "off" : "on", privacy: .public) here)")
            }
            stateStore.setAppliedExceptionRoutes(plan.applied, serviceID: snapshot.serviceID)
            setCurrentNetwork(current)
            writeSucceeded()
            setStatus(.synced(SyncResult(
                snapshot: current, matchedNetwork: plan.matchedNetwork, keptCount: plan.keptThroughOtherGateways,
                notImported: plan.notImported, onLinkCount: plan.onLink.count)))
        } catch let error as HelperError where error.code == .wrongPhase {
            logger.notice("exception rules not applied: \(error.message, privacy: .public)")
            if writing {
                // The network or its routes changed between the read and the write.
                writeFailed(String(localized: "The network or its routes changed while the routes were being written."))
            } else {
                setCurrentNetwork(nil)
                setStatus(.noNetwork)
            }
        } catch let error as HelperError {
            logger.error("exception rules not applied: \(error.message, privacy: .public)")
            failed(error.code == .invalidRequest
                ? String(localized: "The Helper refused the route list (\(error.message)).")
                : String(localized: "Couldn't change the macOS network settings (\(error.message))."),
                writing: writing)
        } catch let error as ConfigStoreError {
            writeFailed(String(localized: "Couldn't save the imported routes to the settings file: \(error.message)"))
        } catch {
            logger.error("exception rules not applied: \(error.localizedDescription, privacy: .public)")
            failed(String(localized: "Couldn't apply the routes: \(error.localizedDescription)"), writing: writing)
        }
    }

    /// A read failure leaves nothing to retry on a timer (the next event syncs again) and makes the
    /// last known network unreliable; a write failure is retried.
    private func failed(_ message: String, writing: Bool) {
        if writing {
            writeFailed(message)
        } else {
            setCurrentNetwork(nil)
            setStatus(.failed(message))
        }
    }

    /// Retries after `writeRetryDelay`, at most `maxWriteRetries` times in a row. When the last retry
    /// fails too, posts a notification (once per streak) and stops; later failures are not retried.
    private func writeFailed(_ message: String) {
        writeFailureStreak += 1
        setStatus(.failed(message))
        retryTask?.cancel()
        retryTask = nil
        let retrying = writeFailureStreak <= Self.maxWriteRetries
        setRetryPending(retrying)
        if retrying {
            retryTask = Task { [weak self] in
                try? await Task.sleep(for: Self.writeRetryDelay)
                guard !Task.isCancelled, let self else { return }
                setRetryPending(false)
                sync()
            }
        } else if writeFailureStreak == Self.maxWriteRetries + 1 {
            logger.error("exception routes still failed after \(Self.maxWriteRetries) retries; retrying stopped")
            notifier.post(
                identifier: "exception-rules-write-failed",
                title: String(localized: "Exception rules aren't being applied"),
                body: String(localized: "Writing the routes still failed after \(Self.maxWriteRetries) retries: \(message)"))
        }
    }

    private func writeSucceeded() {
        retryTask?.cancel()
        retryTask = nil
        setRetryPending(false)
        if writeFailureStreak != 0 { writeFailureStreak = 0 }
    }
}
