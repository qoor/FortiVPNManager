import Foundation
import FortiConfig
import FortiCore
import FortiShared
import os

/// What an openfortivpn config import did (Account tab, first launch). Holds no secret.
struct OpenFortiVPNImportReport: Equatable {
    var sourceURL: URL
    var host: String
    var port: Int
    var username: String
    var realm: String
    /// Digests now in `vpn.trusted_certs`.
    var trustedCertCount: Int
    /// The source file holds a plaintext password: warn and offer "Show in Finder" (it is never modified).
    var containsPlaintextPassword: Bool
    /// The password was saved in the Keychain under `username@host`.
    var passwordStored: Bool
    /// Why the password could not be saved, if there was one.
    var passwordError: String?
    /// Keys openfortivpn does not know (e.g. `insecure-cert`); openfortivpn stops reading at the first one.
    var unknownKeys: [String]
    /// Valid openfortivpn options FortiVPNManager does not carry over.
    var ignoredKeys: [String]
    /// 1-based line numbers that could not be used.
    var invalidLines: [Int]
}

enum OpenFortiVPNImportError: LocalizedError, Equatable {
    case sourceNotFound(path: String)
    case unreadable(path: String, reason: String)
    case missingHost(path: String)
    case config(ConfigStoreError)

    var errorDescription: String? {
        switch self {
        case let .sourceNotFound(path): String(localized: "openfortivpn config file not found: \(path)")
        case let .unreadable(path, reason): String(localized: "Could not read the openfortivpn config file (\(path)): \(reason)")
        case let .missingHost(path): String(localized: "The openfortivpn config file has no host: \(path)")
        case let .config(error): error.message
        }
    }
}

/// Imports an existing openfortivpn config into `config.yaml` and the Keychain (DESIGN §5.6 Account tab):
/// automatically on the first launch, and on demand from the Account tab.
///
/// host, port (`effectivePort`: the file's value, 443 only when it has none), username, realm and up
/// to four `trusted-cert` digests go to `vpn.*`; the password goes to the Keychain (`username@host`).
/// Gmail and connection options are kept. The source file is only read.
struct FirstRunImporter {
    static let maxSourceBytes = 64 * 1024

    let configStore: ConfigStore
    let credentials: CredentialStore
    var homeDirectory = FileManager.default.homeDirectoryForCurrentUser

    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "import")

    init(configStore: ConfigStore, credentials: CredentialStore) {
        self.configStore = configStore
        self.credentials = credentials
    }

    /// The first existing file of `OpenFortiVPNConfigReader.defaultCandidatePaths`.
    func defaultSource() -> URL? {
        OpenFortiVPNConfigReader.defaultCandidatePaths(home: homeDirectory)
            .first { FileManager.default.fileExists(atPath: $0.path) }
    }

    /// First launch: imports when `config.yaml` does not exist yet and an openfortivpn config does.
    /// `nil` when there was nothing to import.
    func importOnFirstLaunch() -> Result<OpenFortiVPNImportReport, OpenFortiVPNImportError>? {
        guard !configStore.existsOnDisk(), let source = defaultSource() else { return nil }
        do {
            return .success(try self.import(from: source))
        } catch {
            return .failure(error)
        }
    }

    /// "Import Existing openfortivpn Config". Replaces `vpn.*`; pins are merged when the gateway is unchanged
    /// and replaced otherwise.
    func `import`(from source: URL) throws(OpenFortiVPNImportError) -> OpenFortiVPNImportReport {
        let data: Data
        do {
            data = try FileIO.readRegularFile(source, limit: Self.maxSourceBytes)
        } catch {
            throw Self.importError(error, path: source.path)
        }
        let parsed = OpenFortiVPNConfigReader.parse(data: data)
        guard let host = parsed.host else { throw .missingHost(path: source.path) }
        let port = parsed.effectivePort
        let digests = TrustedCertPins.normalized(parsed.trustedCerts)

        let config: AppConfig
        do {
            config = try configStore.write(clearingPinsOnGatewayChange: false) { config in
                let sameGateway = config.vpn.host == host && config.vpn.port == port
                config.vpn.host = host
                config.vpn.port = port
                if let username = parsed.username { config.vpn.username = username }
                config.vpn.realm = parsed.realm ?? ""
                config.vpn.trustedCerts = sameGateway
                    ? TrustedCertPins.merging(digests, into: config.vpn.trustedCerts)
                    : Array(digests.prefix(TrustedCertPins.limit))
            }
        } catch {
            throw .config(error)
        }

        var passwordStored = false
        var passwordError: String?
        if let password = parsed.password {
            if config.vpn.username.isEmpty {
                passwordError = String(localized: "The password was not saved to the Keychain because there is no username.")
            } else {
                do {
                    try credentials.setVPNPassword(password, username: config.vpn.username, host: config.vpn.host)
                    passwordStored = true
                } catch {
                    passwordError = error.localizedDescription
                }
            }
        }

        let report = OpenFortiVPNImportReport(
            sourceURL: source,
            host: config.vpn.host,
            port: config.vpn.port,
            username: config.vpn.username,
            realm: config.vpn.realm,
            trustedCertCount: config.vpn.trustedCerts.count,
            containsPlaintextPassword: parsed.password != nil,
            passwordStored: passwordStored,
            passwordError: passwordError,
            unknownKeys: parsed.unknownKeys,
            ignoredKeys: parsed.ignoredKeys,
            invalidLines: parsed.invalidLines
        )
        logger.notice("""
            imported \(source.path, privacy: .public): port \(report.port), \(report.trustedCertCount) pin(s), \
            password stored: \(passwordStored), unknown keys: \(report.unknownKeys, privacy: .public)
            """)
        return report
    }

    private static func importError(_ error: FileIO.ReadError, path: String) -> OpenFortiVPNImportError {
        switch error {
        case .notFound: .sourceNotFound(path: path)
        case .notRegularFile: .unreadable(path: path, reason: String(localized: "Not a regular file."))
        case let .tooLarge(limit): .unreadable(path: path, reason: String(localized: "Larger than \(limit / 1024) KiB."))
        case let .io(ioError): .unreadable(path: path, reason: ioError.localizedDescription)
        }
    }
}
