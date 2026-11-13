extension AppInternal {
    /// What the runtime asks of the OS (components/contract/src/app_internal/port.rs).
    enum PortRequest: Decodable, Equatable, Sendable {
        case readSecret(item: SecretItem)
        /// Never reads the secret, so the OS asks the user for nothing.
        case hasSecret(item: SecretItem)
        case writeSecret(item: SecretItem, value: String)
        /// Deleting a missing item succeeds.
        case deleteSecret(item: SecretItem)

        case helperRegistration
        case registerHelper
        /// A connection whose Helper answers with the app's build counts as verified. Retried once on
        /// a fresh connection when the connection fails: a Helper replaced by a reinstall exits
        /// instead of answering.
        case helperBuild
        case callHelper(call: HelperCall)

        case routeInterface(destination: String)
        case interfaceAddresses
        case primaryRouter
        case screenState

        case openBrowser(url: String)
        case loginItemRegistration
        case setLoginItem(enabled: Bool)

        /// The official FortiClient's SSL VPN connections; Windows imports its settings from them.
        case fortiClientConnections
    }

    enum PortResponse: Encodable, Sendable {
        case secret(value: String?)
        case hasSecret(exists: Bool)
        case secretChanged
        case secretStoreFailed(failure: SecretStoreFailure)

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
        case interfaceAddresses(addresses: [InterfaceAddress])
        case primaryRouter(router: String?)
        case screenState(locked: Bool, displaysAsleep: Bool)

        case browserOpened(opened: Bool)

        case fortiClientConnections(connections: [FortiClientConnection])
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
        /// The runtime's last message.
        case approveQuit
    }

    enum SecretItem: Decodable, Equatable, Sendable {
        case vpnPassword(username: String, host: String)
        case gmailAppPassword(address: String)
        case googleRefreshToken(address: String)
        case googleClientSecret(clientId: String)
    }

    struct SecretStoreFailure: Codable, Equatable, Sendable {
        var description: String
    }

    /// One call on the Helper over the one connection the OS integration keeps (see
    /// components/contract/src/app_internal/port.rs). JSON payloads and the attempt and reason
    /// values are relayed as they are, never read.
    /// - `connect`, `submitOtp`, `readBypassRoutes` and `writeBypassRoutes` go only over a verified
    ///   connection; ask for the build first when it is not verified, and answer
    ///   `helperBuildMismatch` if it differs.
    /// - `status`, `readBypassRoutes` and `writeBypassRoutes` are retried once on a fresh connection
    ///   when the connection fails; the other calls never are.
    /// - Rust sends the next call only after the previous one was answered, except the route calls,
    ///   which may run alongside.
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

    /// An interface appears once per address.
    struct InterfaceAddress: Encodable, Sendable {
        var name: String
        var up: Bool
        var ipv4: String?
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
    }

    enum UserNotification: Decodable, Equatable, Sendable {
        case exceptionRulesNotApplied(retries: Int, failure: BypassFailure)
    }
}

extension AppInternal.SecretItem {
    /// The login Keychain service of this secret. Changing it loses the items already saved.
    var keychainService: String {
        switch self {
        case .vpnPassword: "com.qoor.FortiVPNManager.vpn"
        case .gmailAppPassword: "com.qoor.FortiVPNManager.gmail"
        case .googleRefreshToken, .googleClientSecret: "com.qoor.FortiVPNManager.google"
        }
    }

    /// The login Keychain account of this secret, kept like `keychainService`.
    var keychainAccount: String {
        switch self {
        case let .vpnPassword(username, host): "\(username)@\(host)"
        case let .gmailAppPassword(address): address
        case let .googleRefreshToken(address): "refresh-token:\(address.lowercased())"
        case let .googleClientSecret(clientId): "client-secret:\(clientId)"
        }
    }
}
