import Dispatch
import FortiHelperCore
import FortiShared
import SystemConfiguration

/// Reads and replaces the additional IPv4 routes of the primary network service with networksetup.
/// macOS keeps them in the service's settings and installs them whenever the service is active, so
/// they survive reboots and VPN reconnects. networksetup runs with a fixed argv; every address in
/// it was checked as dotted-quad IPv4 first.
enum BypassRoutes {
    static let networksetupPath = "/usr/sbin/networksetup"

    /// Every call runs here, one at a time and off the XPC connection's queue, so tunnel control
    /// messages on the same connection never wait behind networksetup.
    static let queue = DispatchQueue(label: "com.qoor.FortiVPNManager.Helper.routes")

    /// Call on ``queue``.
    static func read() -> BypassRoutesReply {
        dispatchPrecondition(condition: .onQueue(queue))
        switch primaryService() {
        case .failure(let error): return .failure(error)
        case .success(let service): return snapshot(of: service)
        }
    }

    /// Applies `request` (see `BypassRoutesRequest`) and replies with the routes read back. Call on ``queue``.
    static func write(_ request: BypassRoutesRequest) -> BypassRoutesReply {
        dispatchPrecondition(condition: .onQueue(queue))
        let service: PrimaryService
        switch primaryService() {
        case .failure(let error): return .failure(error)
        case .success(let found): service = found
        }
        let expected = request.expected
        guard service.id == expected.serviceID, service.name == expected.serviceName,
              service.router == expected.router, service.address == expected.address,
              service.subnetMask == expected.subnetMask
        else {
            return .failure(HelperError(.wrongPhase, "the active network service changed; read the routes again"))
        }
        let current: [AdditionalRoute]
        switch routes(of: service) {
        case .failure(let error): return .failure(error)
        case .success(let routes): current = routes
        }
        guard current.count == expected.routes.count, Set(current) == Set(expected.routes) else {
            return .failure(HelperError(.wrongPhase, "the routes changed since they were read; read them again"))
        }
        let existing = Set(current)
        guard request.routes.count <= current.count + IPv4Network.maxCount,
              Set(request.routes).count == request.routes.count,
              request.routes.allSatisfy({ existing.contains($0) || ($0.gateway == service.router && $0.network != nil) })
        else {
            return .failure(HelperError(.invalidRequest, "the route list has repeats, too many routes, or routes this helper does not write"))
        }
        let result = SystemCommand.run(
            networksetupPath, NetworkSetupRoutes.setArguments(service: service.name, routes: request.routes))
        if let message = NetworkSetupRoutes.reportedError(in: result.output + "\n" + result.errorOutput)
            ?? (result.succeeded ? nil : "networksetup exited with \(result.status)") {
            HelperLog.routes.error("setting \(request.routes.count) additional route(s) on \(service.name, privacy: .public) failed: \(message, privacy: .public)")
            return .failure(HelperError(.internalError, message))
        }
        HelperLog.routes.notice("set \(request.routes.count) additional route(s) on \(service.name, privacy: .public)")
        return snapshot(of: service)
    }

    private struct PrimaryService {
        var id: String
        var name: String
        var router: String
        var address: String
        var subnetMask: String
    }

    private static func snapshot(of service: PrimaryService) -> BypassRoutesReply {
        switch routes(of: service) {
        case .failure(let error):
            return .failure(error)
        case .success(let routes):
            return .success(BypassRoutesSnapshot(
                serviceID: service.id, serviceName: service.name, router: service.router,
                address: service.address, subnetMask: service.subnetMask, routes: routes))
        }
    }

    private static func routes(of service: PrimaryService) -> Result<[AdditionalRoute], HelperError> {
        let result = SystemCommand.run(networksetupPath, ["-getadditionalroutes", service.name])
        guard result.succeeded else {
            let message = "networksetup -getadditionalroutes exited with \(result.status): \(result.errorOutput)"
            HelperLog.routes.error("\(message, privacy: .public)")
            return .failure(HelperError(.internalError, message))
        }
        do throws(NetworkSetupRoutes.ParseFailure) {
            return .success(try NetworkSetupRoutes.parse(result.output))
        } catch {
            let message = switch error {
            case .reportedError(let line): line
            case .unexpectedLine(let line): "unexpected networksetup output: \(line)"
            }
            HelperLog.routes.error("reading the additional routes of \(service.name, privacy: .public) failed: \(message, privacy: .public)")
            return .failure(HelperError(.internalError, message))
        }
    }

    /// The service of `State:/Network/Global/IPv4` with its IPv4 router, this Mac's first IPv4 address
    /// and mask on it, and its name in the network settings (what networksetup takes). A tunnel
    /// interface never counts as primary.
    private static func primaryService() -> Result<PrimaryService, HelperError> {
        guard let store = SCDynamicStoreCreate(nil, FortiIdentifiers.helperBundleID as CFString, nil, nil),
              let global = SCDynamicStoreCopyValue(store, "State:/Network/Global/IPv4" as CFString) as? [String: Any],
              let serviceID = global[kSCDynamicStorePropNetPrimaryService as String] as? String,
              let interface = global[kSCDynamicStorePropNetPrimaryInterface as String] as? String,
              !interface.hasPrefix("ppp"), !interface.hasPrefix("utun")
        else {
            return .failure(HelperError(.wrongPhase, "no active network service"))
        }
        let serviceKey = "State:/Network/Service/\(serviceID)/IPv4" as CFString
        guard let router = global[kSCPropNetIPv4Router as String] as? String,
              IPv4Network.parseDottedQuad(Substring(router)) != nil,
              let state = SCDynamicStoreCopyValue(store, serviceKey) as? [String: Any],
              let address = (state[kSCPropNetIPv4Addresses as String] as? [String])?.first,
              let subnetMask = (state[kSCPropNetIPv4SubnetMasks as String] as? [String])?.first,
              IPv4Network.parseDottedQuad(Substring(address)) != nil,
              IPv4Network.parseDottedQuad(Substring(subnetMask)) != nil
        else {
            return .failure(HelperError(.wrongPhase, "the active network service has no IPv4 router or address"))
        }
        guard let preferences = SCPreferencesCreate(nil, FortiIdentifiers.helperBundleID as CFString, nil),
              let service = SCNetworkServiceCopy(preferences, serviceID as CFString),
              let name = SCNetworkServiceGetName(service) as String?,
              !name.isEmpty, !name.hasPrefix("-"), !name.unicodeScalars.contains(where: { $0.value < 0x20 })
        else {
            return .failure(HelperError(.internalError, "the active network service \(serviceID) has no usable name"))
        }
        return .success(PrimaryService(id: serviceID, name: name, router: router, address: address, subnetMask: subnetMask))
    }
}
