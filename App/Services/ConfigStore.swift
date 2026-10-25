import Foundation
import FortiConfig
import FortiCore
import FortiShared
import Observation
import os

/// What changed in `ConfigStore`; delivered after the store's properties were updated.
struct ConfigChange {
    var old: AppConfig
    var new: AppConfig
    /// `loadError` appeared or went away.
    var validityChanged: Bool
}

/// `~/.config/FortiVPNManager/config.yaml` (DESIGN §5.7): polled every 2 s and on app activation,
/// validated by `ConfigCodec`, with the last-known-good config kept while the file is broken.
@Observable
final class ConfigStore {
    let fileURL: URL
    /// Last-known-good config (defaults when the file does not exist).
    private(set) var config = AppConfig()
    private(set) var fileExists = false
    /// Why the file on disk is not in use; `config` is the last-known-good value meanwhile.
    private(set) var loadError: ConfigStoreError?
    /// Non-fatal findings: unknown keys, dropped digests, permissions, hard links.
    private(set) var warnings: [ConfigWarning] = []

    /// Localized message with line/column when known, e.g. "Could not read the config file (…). Line 3, column 9: …".
    var loadErrorMessage: String? { loadError?.message }
    var isValid: Bool { loadError == nil }

    /// Called on the main actor whenever `config` or the validity changed (hand edit, UI save, pin).
    @ObservationIgnored var onChange: ((ConfigChange) -> Void)?

    @ObservationIgnored private var watcher: ConfigFileWatcher
    @ObservationIgnored private var pollTask: Task<Void, Never>?
    @ObservationIgnored private let fileStore = ConfigFileStore()
    @ObservationIgnored private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "config")

    static let pollInterval: Duration = .seconds(2)

    init(fileURL: URL = ConfigFileStore.defaultFileURL(homeDirectory: FileManager.default.homeDirectoryForCurrentUser)) {
        self.fileURL = fileURL
        watcher = ConfigFileWatcher(url: fileURL)
    }

    /// Reads the file once now, then polls every 2 s.
    func start() {
        poll()
        guard pollTask == nil else { return }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: Self.pollInterval)
                self?.poll()
            }
        }
    }

    /// Whether `config.yaml` exists on disk right now (a dangling symlink counts as missing).
    func existsOnDisk() -> Bool {
        do {
            return try fileStore.fingerprint(of: fileURL) != nil
        } catch {
            return true  // Something is there; never treat it as a fresh install.
        }
    }

    /// Stat-polls the file; re-reads and re-validates only when its bytes changed.
    func poll() {
        let change: ConfigFileChange
        do {
            change = try watcher.poll()
        } catch {
            logger.error("config read failed: \(error.message, privacy: .public)")
            apply(config, exists: true, error: error, warnings: warnings)
            return
        }
        switch change {
        case .unchanged:
            break
        case .missing:
            apply(AppConfig(), exists: false, error: nil, warnings: [])
        case let .modified(contents):
            do {
                let result = try ConfigCodec.decode(contents.data)
                apply(result.config, exists: true, error: nil, warnings: result.warnings + contents.warnings)
            } catch {
                logger.error("config rejected: \(error.displayMessage, privacy: .public)")
                apply(config, exists: true, error: .invalidFile(path: contents.fingerprint.realpath, error),
                      warnings: contents.warnings)
            }
        }
    }

    /// Read-modify-write for UI saves: re-reads the latest file, applies `mutate`, writes the template.
    /// When `vpn.host` or `vpn.port` changes and `mutate` left `vpn.trusted_certs` alone, the pins are
    /// cleared (they belong to the old gateway; the next connection learns the new one).
    @discardableResult
    func update(_ mutate: (inout AppConfig) -> Void) throws(ConfigStoreError) -> AppConfig {
        try write(clearingPinsOnGatewayChange: true, mutate)
    }

    /// Effect `.persistTrustedCerts`: writes `vpn.trusted_certs` with exactly `digests`.
    func pinTrustedCerts(_ digests: [String]) {
        guard fileExists else {
            logger.error("not pinning certificates: config.yaml does not exist")
            return
        }
        do {
            try write(clearingPinsOnGatewayChange: false) { $0.vpn.trustedCerts = digests }
            logger.notice("pinned \(digests.count) gateway certificate digest(s)")
        } catch {
            logger.error("pinning certificates failed: \(error.message, privacy: .public)")
        }
    }

    /// Writes `mutate`d config; the import sets its own pins, so it skips the automatic clearing.
    @discardableResult
    func write(clearingPinsOnGatewayChange: Bool, _ mutate: (inout AppConfig) -> Void) throws(ConfigStoreError) -> AppConfig {
        let result = try fileStore.update(at: fileURL) { config in
            let before = config.vpn
            mutate(&config)
            let gatewayChanged = config.vpn.host != before.host || config.vpn.port != before.port
            if clearingPinsOnGatewayChange, gatewayChanged, config.vpn.trustedCerts == before.trustedCerts {
                config.vpn.trustedCerts = []
            }
        }
        guard result.didWrite else { return result.config }
        watcher.noteOwnWrite(result)
        apply(result.config, exists: true, error: nil, warnings: result.contents.warnings)
        return result.config
    }

    private func apply(_ newConfig: AppConfig, exists: Bool, error: ConfigStoreError?, warnings newWarnings: [ConfigWarning]) {
        let old = config
        let validityChanged = (loadError == nil) != (error == nil)
        if config != newConfig { config = newConfig }
        if fileExists != exists { fileExists = exists }
        if loadError != error { loadError = error }
        if warnings != newWarnings { warnings = newWarnings }
        if old != newConfig || validityChanged {
            onChange?(ConfigChange(old: old, new: newConfig, validityChanged: validityChanged))
        }
    }
}

extension ConfigStore {
    /// `otp.method`: how OTP emails are read (`imap` or `gmail_api`).
    var otpMethod: AppConfig.OTP.Method { config.otp.method }

    /// Writes `otp.method` (read-modify-write like every UI save).
    func setOTPMethod(_ method: AppConfig.OTP.Method) throws(ConfigStoreError) {
        try update { $0.otp.method = method }
    }
}

extension VPNProfile {
    /// The reducer's view of `vpn.*`. An empty realm means "no realm".
    init(_ vpn: AppConfig.VPN) {
        self.init(
            host: vpn.host,
            port: vpn.port,
            username: vpn.username,
            realm: vpn.realm.isEmpty ? nil : vpn.realm,
            trustedCerts: vpn.trustedCerts
        )
    }
}

extension ConnectionSettings {
    /// The reducer's view of `connection.*` (auto-connect is read once at launch instead).
    init(_ connection: AppConfig.Connection) {
        self.init(
            sessionMaxMinutes: connection.sessionMaxMinutes,
            proactiveReconnectEnabled: connection.proactiveReconnect.enabled,
            proactiveMarginMinutes: connection.proactiveReconnect.marginMinutes,
            reconnectAfterSleepEnabled: connection.reconnectAfterSleep.enabled,
            minSleepMinutes: connection.reconnectAfterSleep.minSleepMinutes,
            schedule: connection.schedule.enabled ? ConnectionSchedule(connection.schedule) : nil
        )
    }
}

extension ConnectionSchedule {
    /// The reducer's view of an enabled `connection.schedule`.
    init(_ schedule: AppConfig.Connection.Schedule) {
        self.init(
            weekdays: Set(schedule.days.map(\.calendarWeekday)),
            startMinute: schedule.start.minutes,
            endMinute: schedule.end.minutes,
            awayMinutes: schedule.awayMinutes
        )
    }
}
