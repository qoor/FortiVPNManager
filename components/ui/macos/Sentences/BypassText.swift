import Foundation

extension AppInternal.BypassRulesSection {
    /// "Ethernet · 10.20.161.20/24". Whether the rules apply on it, the status line says.
    var currentNetworkText: String {
        guard let network = currentNetwork else { return String(localized: "Unknown") }
        let prefix = network.prefixLength.map { "/\($0)" } ?? ""
        return "\(network.serviceName) · \(network.address)\(prefix)"
    }

    /// Set once the retries of a failed write ran out.
    var stoppedRetryingMessage: String? {
        guard case let .failed(_, _, retries?) = status else { return nil }
        return String(localized: "Stopped after \(retries) failed retries. Tries again when the network or rules change.")
    }
}

extension AppInternal.BypassNotice {
    var message: String {
        switch self {
        case let .rowProblem(row, problem): problem.message(for: row)
        case let .applyFailed(error): error.message
        }
    }
}

extension AppInternal.RowProblem {
    func message(for row: String) -> String {
        switch self {
        case let .invalid(problem): problem.message(for: row)
        case let .duplicate(entry): String(localized: "\(entry) is already in the list.")
        case let .overlap(other): String(localized: "'\(preview(row))' overlaps \(other), which is already in the list.")
        }
    }

    /// The short form beside the row, which already shows the entry.
    var rowMessage: String {
        switch self {
        case let .invalid(problem): problem.rowMessage
        case .duplicate: String(localized: "This address is already in the list.")
        case let .overlap(other): String(localized: "Overlaps \(other).")
        }
    }
}

extension AppInternal.AddressProblem {
    /// The short form beside the row; `message(for:)` is the full one.
    var rowMessage: String {
        switch self {
        case .malformed: String(localized: "Not an IPv4 address or CIDR block.")
        case let .prefixOutOfRange(_, minimum, maximum): String(localized: "Use a prefix length from /\(minimum) to /\(maximum).")
        case .nonContiguousMask: String(localized: "The subnet mask isn't contiguous.")
        case .unroutable: String(localized: "Loopback, multicast and reserved addresses can't be routed.")
        case let .hostBitsSet(network): String(localized: "Did you mean \(network)?")
        }
    }
}

extension AppInternal.BypassStatus {
    func line(time: TimeStyle) -> (text: String, style: NoticeStyle) {
        switch self {
        case .checking:
            return (String(localized: "Checking the routes…"), .info)
        case .noConfigFile:
            return (String(localized: "The settings file doesn't exist yet. Apply creates it."), .info)
        case let .applied(serviceName, router, onLinkTargets, notImported, kept):
            var parts = [String(localized: "Applied to \(serviceName) (router \(router)).")]
            if onLinkTargets > 0 {
                parts.append(String(localized: "\(onLinkTargets) target(s) are on this network and need no route."))
            }
            if notImported > 0 {
                parts.append(String(localized: "List full: \(notImported) route(s) added elsewhere weren't imported."))
            }
            return (Self.adding(kept: kept, to: parts), .success)
        case let .outsideNetworks(kept):
            let parts = [String(localized: "Not applied: outside the networks above. Everything goes through the VPN.")]
            return (Self.adding(kept: kept, to: parts), .info)
        case let .noNetworksListed(kept):
            let parts = [String(localized: "No networks listed, so the rules apply nowhere.")]
            return (Self.adding(kept: kept, to: parts), .info)
        case .configInvalid:
            return (String(localized: "The settings file has an error, so the routes are left as they are."), .warning)
        case .noNetwork:
            return (String(localized: "No network with an IPv4 router. Applies once one connects."), .warning)
        case let .failed(failure, retryInSeconds, _):
            guard let retryInSeconds else { return (failure.message, .error) }
            let retry = String(localized: "Trying again in \(time.spelledOut(seconds: retryInSeconds)).")
            return (failure.message + " " + retry, .error)
        }
    }

    private static func adding(kept: Int, to parts: [String]) -> String {
        guard kept > 0 else { return parts.joined(separator: " ") }
        return (parts + [String(localized: "\(kept) route(s) through other gateways are left as they are.")])
            .joined(separator: " ")
    }
}

extension AppInternal.BypassFailure {
    var message: String {
        switch self {
        case .routesNotKept:
            String(localized: "macOS did not keep the routes that were written.")
        case .networkChanged:
            String(localized: "The network or its routes changed while the routes were being written.")
        case let .helperRefused(message):
            String(localized: "The Helper refused the route list (\(message)).")
        case let .settingsNotChanged(message):
            Self.settingsNotChanged(detail: message)
        case let .importNotSaved(error):
            String(localized: "Couldn't save the imported routes to the settings file: \(error.message)")
        case let .helperUnreachable(failure):
            String(localized: "Couldn't apply the routes: \(failure.message)")
        case let .helperBuildMismatch(helperBuild, appBuild):
            Self.settingsNotChanged(detail: "Helper build \(helperBuild) differs from app build \(appBuild)")
        }
    }

    /// A build mismatch has no sentence of its own, only the app's English detail.
    private static func settingsNotChanged(detail: String) -> String {
        String(localized: "Couldn't change the macOS network settings (\(detail)).")
    }
}

extension AppInternal.UserNotification {
    /// Notifications of the same kind replace each other.
    var identifier: String {
        switch self {
        case .exceptionRulesNotApplied: "exception-rules-write-failed"
        }
    }

    var title: String {
        switch self {
        case .exceptionRulesNotApplied: String(localized: "Exception rules aren't being applied")
        }
    }

    var body: String {
        switch self {
        case let .exceptionRulesNotApplied(retries, failure):
            String(localized: "Writing the routes still failed after \(retries) retries: \(failure.message)")
        }
    }
}
