import AppKit
import FortiCore
import Network

/// Sleep/wake and network path observation for the connection reducer (DESIGN §5.5, §6.3).
///
/// Emits `.willSleep`, `.didWake`, exactly one `.wakeSettled` per completed wake, and `.pathChanged`
/// 3 s after the last `NWPathMonitor` update. The underlay is the interface (and its IPv4 source
/// address) the kernel routes the gateway through, excluding ppp*/utun*/`.other`.
/// System callbacks are delivered on the main queue and handled synchronously on the main actor.
final class SystemMonitor {
    /// Receives the reducer events, on the main actor.
    var onEvent: ((ConnectionEvent) -> Void)?
    /// Whether openfortivpn is still running (the helper's `HelperStatus.childAlive`).
    var processAlive: (() async -> Bool)?

    private(set) var gatewayHost = ""
    private var resolvedGateway: (host: String, ip: String)?
    private let pathMonitor = NWPathMonitor()
    private var latestPath: NWPath?
    private var workspaceObservers: [NSObjectProtocol] = []
    private var sleptAt: Date?
    private var pathTask: Task<Void, Never>?
    private var wakeTask: Task<Void, Never>?
    private var started = false

    static let pathDebounce: Duration = .seconds(3)
    static let settlePollInterval: Duration = .seconds(1)
    static let lateSettlePollInterval: Duration = .seconds(2)

    /// The VPN gateway (`vpn.host`) whose route defines the underlay.
    func setGatewayHost(_ host: String) {
        guard host != gatewayHost else { return }
        gatewayHost = host
        resolvedGateway = nil
    }

    func start() {
        guard !started else { return }
        started = true
        let center = NSWorkspace.shared.notificationCenter
        workspaceObservers = [
            center.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { @Sendable [weak self] _ in
                MainActor.assumeIsolated { self?.systemWillSleep() }
            },
            center.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { @Sendable [weak self] _ in
                MainActor.assumeIsolated { self?.systemDidWake() }
            },
        ]
        pathMonitor.pathUpdateHandler = { @Sendable [weak self] path in
            MainActor.assumeIsolated { self?.pathUpdated(path) }
        }
        pathMonitor.start(queue: .main)
    }

    // MARK: - Sleep and wake

    private func systemWillSleep() {
        sleptAt = Date()
        wakeTask?.cancel()
        onEvent?(.willSleep)
    }

    private func systemDidWake() {
        let sleptFor = sleptAt.map { max(0, Date().timeIntervalSince($0)) } ?? 0
        sleptAt = nil
        onEvent?(.didWake(sleptFor: sleptFor))
        wakeTask?.cancel()
        wakeTask = Task { [weak self] in await self?.settleAfterWake() }
    }

    /// Waits until the path is satisfied and a Router is set (at most 30 s), then reports what survived.
    private func settleAfterWake() async {
        let deadline = Date().addingTimeInterval(RetryPolicy.wakeSettleTimeout)
        var settled = networkSettled()
        while !settled, Date() < deadline {
            do { try await Task.sleep(for: Self.settlePollInterval) } catch { return }
            settled = networkSettled()
        }
        let alive = await processAlive?() ?? false
        let underlay = settled ? await currentUnderlay() : nil
        guard !Task.isCancelled else { return }
        onEvent?(.wakeSettled(WakeObservation(
            processAlive: alive,
            pppUp: NetworkProbe.isPPPInterfaceUp(),
            underlay: underlay,
            pathSatisfied: settled
        )))
        guard !settled else { return }
        // The path may already be satisfied (no NWPath update will follow), so report it as soon as
        // the Router appears; a reducer waiting for the network resumes on that.
        while !networkSettled() {
            do { try await Task.sleep(for: Self.lateSettlePollInterval) } catch { return }
        }
        await reportPath()
    }

    private func networkSettled() -> Bool {
        latestPath?.status == .satisfied && NetworkProbe.hasIPv4Router()
    }

    // MARK: - Path

    private func pathUpdated(_ path: NWPath) {
        latestPath = path
        pathTask?.cancel()
        pathTask = Task { [weak self] in
            do { try await Task.sleep(for: Self.pathDebounce) } catch { return }
            await self?.reportPath()
        }
    }

    private func reportPath() async {
        let satisfied = latestPath?.status == .satisfied
        let underlay = satisfied ? await currentUnderlay() : nil
        guard !Task.isCancelled else { return }
        onEvent?(.pathChanged(PathSnapshot(isSatisfied: satisfied, underlay: underlay)))
    }

    /// Interface and source IPv4 the kernel uses for the gateway, unless it is a tunnel or `.other`.
    private func currentUnderlay() async -> Underlay? {
        guard let ip = await gatewayIPv4(),
              let name = await NetworkProbe.routeInterface(to: ip),
              !NetworkProbe.isTunnelInterface(name)
        else { return nil }
        if latestPath?.availableInterfaces.first(where: { $0.name == name })?.type == .other {
            return nil
        }
        return Underlay(interfaceName: name, sourceAddress: NetworkProbe.firstIPv4Address(of: name))
    }

    /// The gateway address, resolved once per host: while the tunnel is up, DNS may answer through the
    /// VPN with a different (internal) address.
    private func gatewayIPv4() async -> String? {
        let host = gatewayHost
        guard !host.isEmpty else { return nil }
        if let resolvedGateway, resolvedGateway.host == host { return resolvedGateway.ip }
        guard let ip = await NetworkProbe.resolveIPv4(host) else { return nil }
        if host == gatewayHost { resolvedGateway = (host, ip) }
        return ip
    }
}
