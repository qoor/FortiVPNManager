import Darwin
import Dispatch
import FortiCore
import FortiHelperCore
import FortiShared
import Foundation

/// The helper's single source of truth (DESIGN §5.2 "control"): at most one openfortivpn run (a
/// `TunnelSupervisor`), the connection that owns it, and whether the helper is on its way out.
///
/// All state is confined to `queue`. XPC handlers call in with `queue.sync`. Supervisor callbacks
/// hop over with `queue.async`, never `sync` (the controller calls into the supervisor
/// synchronously); since both callbacks go through the same serial queue, events and output reach
/// the app in the supervisor's order.
final class TunnelController: @unchecked Sendable {
    private struct Run {
        /// Distinguishes runs in callbacks (the app chooses attempt IDs).
        let serial: UInt64
        let attemptID: AttemptID
        let owner: ClientLink
        let supervisor: TunnelSupervisor
        let configPath: String
        let gateway: GatewayAddress
        /// What the pidfile records.
        var processes: [ProcessIdentity] = []
    }

    /// How long SIGTERM waits for the run to finish (SIGTERM → SIGKILL takes 10 s of it).
    private static let shutdownLimit: DispatchTimeInterval = .seconds(15)

    private let queue = DispatchQueue(label: "com.qoor.FortiVPNManager.Helper.controller")
    private let updateWatcher: SelfUpdateWatcher

    private var run: Run?
    private var runSerial: UInt64 = 0
    /// Output `seq` continues across runs so the app's log stays ordered.
    private var nextOutputSeq: UInt64 = 1
    /// Owner of the most recent run: its pending callbacks (`.exited`) are flushed before exiting.
    private var lastOwner: ClientLink?
    /// Set once the helper decided to exit; no new run or connection is accepted afterwards.
    private var isExiting = false

    init(updateWatcher: SelfUpdateWatcher) {
        self.updateWatcher = updateWatcher
    }

    // MARK: - XPC requests

    /// Starts a run. `nil` means accepted; a failed spawn is reported later as `.exited`.
    func connect(_ request: ConnectRequest, from client: ClientLink) -> HelperError? {
        queue.sync { startRun(request, owner: client) }
    }

    func submitOTP(attemptID: String, code: String, from client: ClientLink) -> HelperError? {
        queue.sync { () -> HelperError? in
            let target: Run
            switch controllableRun(attemptID, by: client) {
            case .success(let run): target = run
            case .failure(let error): return error
            }
            do throws(HelperError) {
                try target.supervisor.submitOTP(code)
            } catch {
                return error
            }
            return nil
        }
    }

    /// Asks the run to stop (empty `attemptID`: whatever runs). Completion arrives as `.exited`.
    func disconnect(attemptID: String, reason: String, from client: ClientLink) -> HelperError? {
        queue.sync { () -> HelperError? in
            if attemptID.isEmpty && run == nil { return nil }
            let target: Run
            switch controllableRun(attemptID, by: client) {
            case .success(let run): target = run
            case .failure(let error): return error
            }
            let stopReason = StopReason(rawValue: reason) ?? .userDisconnect
            HelperLog.helper.notice("run \(target.attemptID, privacy: .public): stop requested (\(stopReason.rawValue, privacy: .public))")
            target.supervisor.stop(reason: stopReason)
            return nil
        }
    }

    func status() -> HelperStatus {
        let binary = BinaryLocator.locate()
        return queue.sync {
            var status = HelperStatus(
                buildID: HelperBuild.id,
                activeAttemptID: run?.attemptID,
                childAlive: run?.supervisor.processID != nil,
                openfortivpnPath: nil,
                openfortivpnVersion: nil,
                binaryProblem: nil
            )
            switch binary {
            case .success(let located):
                status.openfortivpnPath = located.path
                status.openfortivpnVersion = located.version
            case .failure(let problem):
                status.openfortivpnPath = problem.path
                status.binaryProblem = problem.message
            }
            return status
        }
    }

    /// Exits shortly after replying `true` if no run is active (the app wants launchd to start
    /// the current binary); `false` otherwise.
    func exitWhenIdle() -> Bool {
        queue.sync {
            guard run == nil else { return false }
            if !isExiting {
                isExiting = true
                HelperLog.helper.notice("exiting on request so launchd starts the current binary")
                // Leave time for the reply to go out.
                queue.asyncAfter(deadline: .now() + .milliseconds(500)) { exit(EXIT_SUCCESS) }
            }
            return true
        }
    }

    // MARK: - Lifecycle

    /// Called for every new connection before it is accepted. Starts exiting if the binary was
    /// replaced and nothing runs; returns `false` if the connection should be refused.
    func admitNewConnection() -> Bool {
        queue.sync {
            exitIfReplaced()
            return !isExiting
        }
    }

    /// The connection that owns the run went away (app quit or crashed): stop the run.
    func clientInvalidated(_ client: ClientLink) {
        queue.async { [self] in
            guard let run, run.owner === client else { return }
            HelperLog.helper.notice("run \(run.attemptID, privacy: .public): owning connection invalidated")
            run.supervisor.stop(reason: .clientInvalidated)
        }
    }

    /// `SelfUpdateWatcher` noticed a replaced binary.
    func binaryReplaced() {
        queue.async { [self] in exitIfReplaced() }
    }

    /// SIGTERM: stops the run, waits for its `.exited` (at most `shutdownLimit`), then exits.
    func shutdown() {
        queue.async { [self] in
            HelperLog.helper.notice("SIGTERM received")
            isExiting = true
            guard let run else {
                exitAfterFlushing()
                return
            }
            run.supervisor.stop(reason: .helperShutdown)
            queue.asyncAfter(deadline: .now() + Self.shutdownLimit) {
                HelperLog.helper.error("the run did not finish after SIGTERM; exiting anyway")
                exit(EXIT_SUCCESS)
            }
        }
    }

    // MARK: - Runs (on `queue`)

    private func startRun(_ request: ConnectRequest, owner: ClientLink) -> HelperError? {
        guard !isExiting else {
            return HelperError(.busy, "the Helper is shutting down; try again shortly")
        }
        if let run {
            return run.owner.uid == owner.uid
                ? HelperError(.busy, "openfortivpn is already running")
                : HelperError(.notAuthorized, "a VPN run started by another user is active")
        }

        let binary: OpenFortiVPNBinary
        switch BinaryLocator.locate() {
        case .success(let located): binary = located
        case .failure(let problem): return HelperError(.binaryUnavailable, problem.message)
        }
        if !binary.isTestedVersion {
            HelperLog.helper.warning("openfortivpn \(binary.version ?? "unknown", privacy: .public) is not 1.24.x; its output may not be parsed correctly")
        }

        let config: String
        do {
            config = try ConfigRenderer.render(request)
        } catch {
            return HelperError(.invalidRequest, error.description)
        }
        let configPath: String
        do {
            try RunDirectory.prepare()
            configPath = try RunDirectory.writeConfig(config)
        } catch {
            HelperLog.helper.error("cannot write the openfortivpn config: \(error.description, privacy: .public)")
            return HelperError(.internalError, "could not create the openfortivpn config file: \(error.description)")
        }

        runSerial &+= 1
        let serial = runSerial
        let gateway = GatewayAddress()
        let supervisor: TunnelSupervisor
        do {
            supervisor = try TunnelSupervisor(
                attemptID: request.attemptID,
                executable: .openfortivpn(path: binary.path, configPath: configPath),
                password: request.password,
                firstOutputSeq: nextOutputSeq,
                routeRepair: { RouteRepair.repair(gatewayIP: gateway.ip) },
                onEvent: { [weak self] event in
                    guard let self else { return }
                    queue.async { self.handle(event, serial: serial) }
                },
                onOutput: { [weak self] lines in
                    guard let self else { return }
                    queue.async { self.forward(lines, serial: serial) }
                }
            )
        } catch {
            RunDirectory.remove(configPath)
            return error
        }

        run = Run(
            serial: serial,
            attemptID: request.attemptID,
            owner: owner,
            supervisor: supervisor,
            configPath: configPath,
            gateway: gateway
        )
        do {
            try supervisor.start()
        } catch {
            run = nil
            RunDirectory.remove(configPath)
            return error
        }
        HelperLog.helper.notice("run \(request.attemptID, privacy: .public) started for uid \(owner.uid) with openfortivpn \(binary.version ?? "unknown", privacy: .public)")
        resolveGateway(request.host, into: gateway, serial: serial)
        return nil
    }

    /// The active run if `attemptID` names it (or is empty) and `client` runs as the run's owner uid.
    private func controllableRun(_ attemptID: String, by client: ClientLink) -> Result<Run, HelperError> {
        guard let run, attemptID.isEmpty || UUID(uuidString: attemptID) == run.attemptID else {
            return .failure(HelperError(.unknownAttempt, "not the active connection attempt"))
        }
        guard run.owner.uid == client.uid else {
            return .failure(HelperError(.notAuthorized, "the VPN run belongs to another user"))
        }
        return .success(run)
    }

    private func forward(_ lines: [OutputLine], serial: UInt64) {
        guard let run, run.serial == serial else { return }
        run.owner.send(lines)
    }

    private func handle(_ event: HelperEvent, serial: UInt64) {
        guard var current = run, current.serial == serial else { return }
        current.owner.send(event)
        switch event.kind {
        case .milestone(.spawned, _):
            if let pid = current.supervisor.processID, let process = ProcessIdentity.ofRootProcess(pid) {
                current.processes = [process]
                run = current
                saveRecord(current)
                HelperLog.helper.notice("run \(current.attemptID, privacy: .public): openfortivpn running as pid \(pid)")
            }
        case .milestone(.tunnelUp, _):
            if let pid = current.supervisor.processID {
                let recorded = current.processes
                let pppd = ProcessIdentity.children(of: pid, runningAt: ProcessIdentity.pppdPath)
                current.processes.append(contentsOf: pppd.filter { !recorded.contains($0) })
                run = current
                saveRecord(current)
            }
            HelperLog.helper.notice("run \(current.attemptID, privacy: .public): tunnel up")
        case .milestone:
            break
        case .exited(let facts):
            finishRun(current, facts: facts)
        }
    }

    private func finishRun(_ finished: Run, facts: ExitFacts) {
        RunDirectory.remove(finished.configPath)
        RunDirectory.removeRecord()
        nextOutputSeq = max(nextOutputSeq, facts.lastOutputSeq &+ 1)
        lastOwner = finished.owner
        run = nil
        HelperLog.helper.notice("""
            run \(finished.attemptID, privacy: .public) exited: \(String(describing: facts.exitStatus), privacy: .public), \
            initiator \(String(describing: facts.stopInitiator), privacy: .public), \
            furthest \(String(describing: facts.furthestMilestone), privacy: .public), \
            route repair \(facts.routeRepairPerformed)
            """)
        if isExiting {
            exitAfterFlushing()
        } else {
            exitIfReplaced()
        }
    }

    /// Resolves the gateway in the background so a slow DNS answer never holds up `queue`.
    private func resolveGateway(_ host: String, into gateway: GatewayAddress, serial: UInt64) {
        DispatchQueue.global(qos: .utility).async { [weak self] in
            guard let ip = IPv4.resolve(host) else {
                HelperLog.routes.info("gateway has no IPv4 address; its host route will not be repaired")
                return
            }
            gateway.set(ip)
            guard let self else { return }
            queue.async { self.gatewayResolved(serial: serial) }
        }
    }

    private func gatewayResolved(serial: UInt64) {
        guard let run, run.serial == serial, !run.processes.isEmpty else { return }
        saveRecord(run)
    }

    private func saveRecord(_ run: Run) {
        do {
            try RunDirectory.writeRecord(RunRecord(processes: run.processes, gatewayIP: run.gateway.ip))
        } catch {
            HelperLog.helper.error("cannot write the pidfile: \(error.description, privacy: .public)")
        }
    }

    // MARK: - Exiting (on `queue`)

    private func exitIfReplaced() {
        guard !isExiting, run == nil, updateWatcher.isReplaced else { return }
        HelperLog.helper.notice("executable replaced at \(self.updateWatcher.path, privacy: .public); exiting so launchd starts the new build")
        exitAfterFlushing()
    }

    /// Exits once the callbacks already sent to the last run's owner have left (at most 1 s).
    private func exitAfterFlushing() {
        isExiting = true
        guard let lastOwner else { exit(EXIT_SUCCESS) }
        let exitNow: @Sendable () -> Void = { exit(EXIT_SUCCESS) }
        lastOwner.afterPendingSends(exitNow)
        queue.asyncAfter(deadline: .now() + .seconds(1), execute: exitNow)
    }
}
