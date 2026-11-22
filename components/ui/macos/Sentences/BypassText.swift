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
    guard case .failed(_, _, let retries?) = status else { return nil }
    return String(
      localized:
        "Stopped after \(retries) failed retries. Tries again when the network or rules change.")
  }
}

extension AppInternal.BypassNotice {
  var message: String {
    switch self {
    case .rowProblem(let row, let problem): problem.message(for: row)
    case .applyFailed(let error): error.message
    }
  }
}

extension AppInternal.RowProblem {
  func message(for row: String) -> String {
    switch self {
    case .invalid(let problem): problem.message(for: row)
    case .duplicate(let entry): String(localized: "\(entry) is already in the list.")
    case .overlap(let other):
      String(localized: "'\(preview(row))' overlaps \(other), which is already in the list.")
    case .vpnServer: String(localized: "Can't add the VPN server's address '\(preview(row))'.")
    }
  }

  /// The short form beside the row, which already shows the entry.
  var rowMessage: String {
    switch self {
    case .invalid(let problem): problem.rowMessage
    case .duplicate: String(localized: "This address is already in the list.")
    case .overlap(let other): String(localized: "Overlaps \(other).")
    case .vpnServer:
      String(localized: "The VPN server's address can't be registered as an exception target.")
    }
  }
}

extension AppInternal.AddressProblem {
  /// The short form beside the row; `message(for:)` is the full one.
  var rowMessage: String {
    switch self {
    case .malformed: String(localized: "Not an IPv4 address or CIDR block.")
    case .prefixOutOfRange(_, let minimum, let maximum):
      String(localized: "Use a prefix length from /\(minimum) to /\(maximum).")
    case .nonContiguousMask: String(localized: "The subnet mask isn't contiguous.")
    case .unroutable:
      String(localized: "Loopback, multicast and reserved addresses can't be routed.")
    case .hostBitsSet(let network): String(localized: "Did you mean \(network)?")
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
    case .applied(let serviceName, let router, let onLinkTargets):
      var parts = [String(localized: "Applied to \(serviceName) (router \(router)).")]
      if onLinkTargets > 0 {
        parts.append(
          String(localized: "\(onLinkTargets) target(s) are on this network and need no route."))
      }
      return (parts.joined(separator: " "), .success)
    case .vpnOff:
      return (String(localized: "Not applied while the VPN is off."), .info)
    case .outsideNetworks:
      return (
        String(
          localized: "Not applied: outside the networks above. Everything goes through the VPN."),
        .info
      )
    case .noNetworksListed:
      return (String(localized: "No networks listed, so the rules apply nowhere."), .info)
    case .configInvalid:
      return (
        String(localized: "The settings file has an error, so the routes are left as they are."),
        .warning
      )
    case .noNetwork:
      return (
        String(localized: "No network with an IPv4 router. Applies once one connects."), .warning
      )
    case .failed(let failure, let retryInSeconds, _):
      guard let retryInSeconds else { return (failure.message, .error) }
      let retry = String(localized: "Trying again in \(time.spelledOut(seconds: retryInSeconds)).")
      return (failure.message + " " + retry, .error)
    }
  }
}

extension AppInternal.BypassFailure {
  var message: String {
    switch self {
    case .routesNotKept:
      String(localized: "macOS did not keep the routes that were written.")
    case .networkChanged:
      String(localized: "The network or its routes changed while the routes were being written.")
    case .helperRefused(let message):
      String(localized: "The Helper refused the route list (\(message)).")
    case .settingsNotChanged(let message):
      Self.settingsNotChanged(detail: message)
    case .helperUnreachable(let failure):
      String(localized: "Couldn't apply the routes: \(failure.message)")
    case .helperBuildMismatch(let helperBuild, let appBuild):
      Self.settingsNotChanged(
        detail: "Helper build \(helperBuild) differs from app build \(appBuild)")
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
    case .updateAvailable: Self.updateAvailableIdentifier
    }
  }

  static let updateAvailableIdentifier = "update-available"

  var title: String {
    switch self {
    case .exceptionRulesNotApplied: String(localized: "Exception rules aren't being applied")
    case .updateAvailable: String(localized: "Software Update Available")
    }
  }

  var body: String {
    switch self {
    case .exceptionRulesNotApplied(let retries, let failure):
      String(
        localized: "Writing the routes still failed after \(retries) retries: \(failure.message)")
    case .updateAvailable(let version):
      String(localized: "Update to FortiVPNManager \(version) to get the latest features.")
    }
  }

  var actionTitle: String? {
    switch self {
    case .exceptionRulesNotApplied: nil
    case .updateAvailable: String(localized: "Update Now")
    }
  }
}
