import Foundation
import FortiConfig
import FortiCore
import FortiShared
import Observation
import os
import ServiceManagement

/// What the last readiness check learned about the helper (Connection tab "Helper" section).
struct HelperHealth: Equatable {
    var serviceStatus: SMAppService.Status
    /// BuildID the running helper reported; `nil` when it was not asked or did not answer.
    var build: BuildID?
    var buildMatches: Bool
    /// openfortivpn path/version/problem; only fetched when the build matches.
    var status: HelperStatus?
    /// Why the helper could not be reached, if it could not.
    var errorMessage: String?
}

/// Gathers `ReadinessInputs` (DESIGN §5.4): config validity, account fields, the Keychain items,
/// helper `.enabled` and helper BuildID == app `CFBundleVersion`. Gmail counts as ready with an IMAP
/// app password, or with `otp.method: gmail_api` and its Google authorization (client secret +
/// refresh token).
///
/// On a BuildID mismatch with no run in progress it asks the old helper to exit (launchd then starts
/// the embedded binary of this build) and checks once more.
@Observable
final class ReadinessChecker {
    private(set) var readiness: Readiness = .unknown
    private(set) var helperHealth: HelperHealth?

    /// Whether a helper restart is safe right now (no run is active). Evaluated at decision time.
    @ObservationIgnored var canRestartHelper: () -> Bool = { false }

    @ObservationIgnored private let configStore: ConfigStore
    @ObservationIgnored private let credentials: CredentialStore
    @ObservationIgnored private let installer: HelperInstaller
    @ObservationIgnored private let helper: HelperClient
    /// Whether the Gmail API can be used for an address (a usable OAuth client plus a refresh token).
    @ObservationIgnored private let gmailAPIAuthorized: (String) -> Bool
    @ObservationIgnored private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "readiness")

    init(
        configStore: ConfigStore,
        credentials: CredentialStore,
        installer: HelperInstaller,
        helper: HelperClient,
        gmailAPIAuthorized: @escaping (String) -> Bool
    ) {
        self.configStore = configStore
        self.credentials = credentials
        self.installer = installer
        self.helper = helper
        self.gmailAPIAuthorized = gmailAPIAuthorized
    }

    func check() async -> Readiness {
        let health = await checkHelper()
        // Read the config after the XPC round trip so the answer reflects the latest file.
        let config = configStore.config
        let host = config.vpn.host
        let username = config.vpn.username
        let gmail = config.otp.gmailAddress

        let inputs = ReadinessInputs(
            configValid: configStore.isValid,
            accountComplete: Self.isUsableAccount(config.vpn) && !gmail.isEmpty,
            hasVPNPassword: !host.isEmpty && !username.isEmpty
                && keychainHas { try credentials.hasVPNPassword(username: username, host: host) },
            hasGmailAppPassword: !gmail.isEmpty && hasGmailCredentials(config.otp),
            helperEnabled: health.serviceStatus == .enabled,
            helperBuildMatches: health.buildMatches
        )
        let result = inputs.readiness
        if helperHealth != health { helperHealth = health }
        if readiness != result { readiness = result }
        return result
    }

    private func checkHelper() async -> HelperHealth {
        let serviceStatus = installer.status
        var health = HelperHealth(serviceStatus: serviceStatus, buildMatches: false)
        guard serviceStatus == .enabled else { return health }
        do {
            var build = try await helper.getVersion()
            if build != helper.appBuild, canRestartHelper() {
                logger.notice("helper build \(build.value, privacy: .public) != app \(self.helper.appBuild.value, privacy: .public); restarting it")
                _ = try await helper.exitWhenIdle()
                try await Task.sleep(for: .seconds(1))
                build = try await helper.getVersion()
            }
            health.build = build
            health.buildMatches = build == helper.appBuild
            if health.buildMatches {
                health.status = try await helper.status()
            }
        } catch {
            health.errorMessage = error.localizedDescription
            logger.error("helper check failed: \(error.localizedDescription, privacy: .public)")
        }
        return health
    }

    /// Host and username are set and host/username/realm pass the helper's own checks, so a value
    /// it would refuse (hand-edited or imported) asks for the Account tab instead of failing a connect.
    private static func isUsableAccount(_ vpn: AppConfig.VPN) -> Bool {
        do throws(ConfigRenderer.ValidationError) {
            try ConfigRenderer.validateHost(vpn.host)
            try ConfigRenderer.validateUsername(vpn.username)
            try ConfigRenderer.validateRealm(vpn.realm)
            return true
        } catch {
            return false
        }
    }

    /// An IMAP app password, or (method `gmail_api`) a stored Google authorization for the address.
    private func hasGmailCredentials(_ otp: AppConfig.OTP) -> Bool {
        let address = otp.gmailAddress
        if keychainHas({ try credentials.hasGmailAppPassword(address: address) }) { return true }
        guard otp.method == .gmailAPI else { return false }
        return gmailAPIAuthorized(address)
    }

    private func keychainHas(_ lookup: () throws -> Bool) -> Bool {
        do {
            return try lookup()
        } catch {
            logger.error("keychain lookup failed: \(error.localizedDescription, privacy: .public)")
            return false
        }
    }
}
