extension AppInternal {
  /// What the runtime asks of the OS (components/contract/src/app_internal/port.rs).
  enum PortRequest: Decodable, Equatable, Sendable {
    case helperRegistration
    case registerHelper
    /// A connection whose Helper answers with the app's build counts as verified. Retried once on
    /// a fresh connection when the connection fails: a Helper replaced by a reinstall exits
    /// instead of answering.
    case helperBuild
    case callHelper(call: HelperCall)

    case routeInterface(destination: String)
    case primaryRouter
    case screenState

    case openBrowser(url: String)
    case loginItemRegistration
    case setLoginItem(enabled: Bool)

    /// The official FortiClient's SSL VPN connections; Windows imports its settings from them.
    case fortiClientConnections

    case startUpdateWaiter(installId: UInt64)
  }

  enum PortResponse: Encodable, Sendable {
    case registration(status: RegistrationStatus)
    case registered
    case registrationFailed(error: RegistrationError)
    case helperBuild(build: String)
    /// The Helper's reply as received; nil for an empty (success) reply.
    case helperReply(json: String?)
    case helperReplyFlag(value: Bool)
    case helperUnreachable(failure: HelperChannelFailure)
    /// A call that needs a verified connection was not sent.
    case helperBuildMismatch(helperBuild: String)

    case routeInterface(interface: String?)
    case primaryRouter(router: String?)
    case screenState(locked: Bool, displaysAsleep: Bool)

    case browserOpened(opened: Bool)

    case fortiClientConnections(connections: [FortiClientConnection])

    case updateWaiterStarted
    case updateWaiterFailed(description: String)
  }

  /// A FortiClient SSL VPN connection: its name and its `Server` value, usually `host:port`.
  struct FortiClientConnection: Encodable, Equatable, Sendable {
    var name: String
    var server: String
  }

  /// Carried out without an answer.
  enum PortAction: Decodable, Equatable, Sendable {
    case log(level: LogLevel, category: LogCategory, message: String)
    /// Replaces an earlier notification of the same kind. The UI's string code builds the text;
    /// the OS integration only posts it.
    case postNotification(notification: UserNotification)
    case holdActivity(hold: Bool)
    case openHelperApprovalSettings
    case useHelperTeam(teamId: String?)
    case releaseUpdateWaiter(installerProcessId: UInt32)
    case stopUpdateWaiter
    /// The runtime's last message. Also comes unasked once an update's installer started: then
    /// the app quits.
    case approveQuit
  }

  struct SecretStoreFailure: Decodable, Equatable, Sendable {
    var description: String?
    var code: Int32
  }

  /// One call on the Helper over the one connection the OS integration keeps (see
  /// components/contract/src/app_internal/port.rs). JSON payloads and the attempt and reason
  /// values are relayed as they are, never read.
  /// - `connect`, `submitOtp`, `readBypassRoutes`, `writeBypassRoutes` and the update calls go
  ///   only over a verified connection; ask for the build first when it is not verified, and
  ///   answer `helperBuildMismatch` if it differs.
  /// - `status`, `readBypassRoutes` and `writeBypassRoutes` are retried once on a fresh connection
  ///   when the connection fails; the other calls never are.
  /// - Rust sends the next call only after the previous one was answered, except the route and
  ///   update calls, which may run alongside.
  enum HelperCall: Decodable, Equatable, Sendable {
    case connect(json: String)
    case submitOtp(attempt: String, code: String)
    case disconnect(attempt: String, reason: String)
    case status
    /// After the reply, drop the connection: that clears the verified mark and reports
    /// `SystemEvent.helperConnectionLost`, which Rust expects.
    case exitWhenIdle
    case readBypassRoutes
    case writeBypassRoutes(json: String)
    case prepareUpdate(json: String)
    case installUpdate(json: String)
    case readPreparedUpdate
    case discardPreparedUpdate
  }

  enum HelperChannelFailure: Codable, Equatable, Sendable {
    case appNotSigned
    case connection(description: String)
    case connectionLost
    case noReply
    case unreadableReply(description: String)
  }

  enum RegistrationStatus: String, Encodable, Sendable {
    case notRegistered
    case enabled
    case requiresApproval
    case notFound
  }

  enum RegistrationError: Codable, Equatable, Sendable {
    case notInstalledCopy
    case failed(description: String)
  }

  enum LogLevel: String, Decodable, Sendable {
    case info
    case notice
    case error
  }

  /// The unified-log category.
  enum LogCategory: String, Decodable, Sendable {
    case connection
    case readiness
    case config
    case state
    case logfile
    case `import`
    case oauth
    case otp
    case bypassRoutes = "bypass-routes"
    case loginItem = "login-item"
    case update
  }

  enum UserNotification: Decodable, Equatable, Sendable {
    case exceptionRulesNotApplied(retries: Int, failure: BypassFailure)
    case updateAvailable(version: String)
  }
}
