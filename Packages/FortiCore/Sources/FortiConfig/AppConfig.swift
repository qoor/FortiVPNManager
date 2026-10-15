import Foundation

/// Non-secret settings stored in `~/.config/FortiVPNManager/config.yaml` (DESIGN §5.7).
///
/// Passwords are never part of this model; they live in the login Keychain. Defaults match the
/// fixed template written by ``ConfigTemplate``. The `Codable` conformance uses the same
/// snake_case keys as the YAML file and fills missing keys with defaults, but the YAML file itself
/// is always read through ``ConfigCodec`` (strict validation) and written through ``ConfigTemplate``.
public struct AppConfig: Codable, Sendable, Equatable {
    /// The `schema_version` this build reads and writes.
    public static let currentSchemaVersion = 1

    public var schemaVersion: Int
    public var vpn: VPN
    public var otp: OTP
    public var connection: Connection

    public init(
        schemaVersion: Int = AppConfig.currentSchemaVersion,
        vpn: VPN = VPN(),
        otp: OTP = OTP(),
        connection: Connection = Connection()
    ) {
        self.schemaVersion = schemaVersion
        self.vpn = vpn
        self.otp = otp
        self.connection = connection
    }

    enum CodingKeys: String, CodingKey, CaseIterable {
        case schemaVersion = "schema_version"
        case vpn
        case otp
        case connection
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let defaults = AppConfig()
        schemaVersion = try container.decodeIfPresent(Int.self, forKey: .schemaVersion) ?? defaults.schemaVersion
        vpn = try container.decodeIfPresent(VPN.self, forKey: .vpn) ?? defaults.vpn
        otp = try container.decodeIfPresent(OTP.self, forKey: .otp) ?? defaults.otp
        connection = try container.decodeIfPresent(Connection.self, forKey: .connection) ?? defaults.connection
    }

    /// `vpn:` section.
    public struct VPN: Codable, Sendable, Equatable {
        /// Gateway host name or IPv4 address.
        public var host: String
        public var port: Int
        public var username: String
        /// Empty string means "no realm".
        public var realm: String
        /// Lowercase hex SHA-256 digests of trusted gateway certificates, newest first.
        /// Managed by the app (never shown in the UI).
        public var trustedCerts: [String]

        public init(
            host: String = "",
            port: Int = 443,
            username: String = "",
            realm: String = "",
            trustedCerts: [String] = []
        ) {
            self.host = host
            self.port = port
            self.username = username
            self.realm = realm
            self.trustedCerts = trustedCerts
        }

        enum CodingKeys: String, CodingKey, CaseIterable {
            case host
            case port
            case username
            case realm
            case trustedCerts = "trusted_certs"
        }

        public init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            let defaults = VPN()
            host = try container.decodeIfPresent(String.self, forKey: .host) ?? defaults.host
            port = try container.decodeIfPresent(Int.self, forKey: .port) ?? defaults.port
            username = try container.decodeIfPresent(String.self, forKey: .username) ?? defaults.username
            realm = try container.decodeIfPresent(String.self, forKey: .realm) ?? defaults.realm
            trustedCerts = try container.decodeIfPresent([String].self, forKey: .trustedCerts) ?? defaults.trustedCerts
        }

        /// Pins a gateway certificate digest: normalizes it to lowercase, moves it to the front
        /// (removing an older copy) and keeps at most ``ConfigLimits/maxTrustedCerts`` entries,
        /// dropping the oldest. Returns `false` (and changes nothing) if `digest` is not a SHA-256 hex digest.
        @discardableResult
        public mutating func pinTrustedCert(_ digest: String) -> Bool {
            guard let normalized = TrustedCertDigest.normalize(digest) else { return false }
            trustedCerts.removeAll { $0 == normalized }
            trustedCerts.insert(normalized, at: 0)
            if trustedCerts.count > ConfigLimits.maxTrustedCerts {
                trustedCerts.removeLast(trustedCerts.count - ConfigLimits.maxTrustedCerts)
            }
            return true
        }
    }

    /// `otp:` section.
    public struct OTP: Codable, Sendable, Equatable {
        /// How the app reads the OTP emails (`otp.method`).
        public enum Method: String, Codable, Sendable, CaseIterable {
            /// Gmail IMAP with an app password (Keychain service `.gmail`).
            case imap
            /// Gmail REST API with a Google OAuth authorization (Keychain service `.google`). Falls back
            /// to IMAP for an attempt when the authorization fails and an app password is stored.
            case gmailAPI = "gmail_api"
        }

        /// Gmail (Google Workspace) address that receives the OTP mails. Its app password and Google
        /// authorization are in the Keychain.
        public var gmailAddress: String
        public var method: Method
        /// OAuth client ID of the Google Cloud "Desktop app" client used by ``Method/gmailAPI``.
        /// Not a secret; the client secret is in the Keychain. Empty when no client was imported.
        public var googleClientID: String

        public init(gmailAddress: String = "", method: Method = .imap, googleClientID: String = "") {
            self.gmailAddress = gmailAddress
            self.method = method
            self.googleClientID = googleClientID
        }

        enum CodingKeys: String, CodingKey, CaseIterable {
            case gmailAddress = "gmail_address"
            case method
            case googleClientID = "google_client_id"
        }

        public init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            let defaults = OTP()
            gmailAddress = try container.decodeIfPresent(String.self, forKey: .gmailAddress) ?? defaults.gmailAddress
            method = try container.decodeIfPresent(Method.self, forKey: .method) ?? defaults.method
            googleClientID = try container.decodeIfPresent(String.self, forKey: .googleClientID)
                ?? defaults.googleClientID
        }
    }

    /// `connection:` section.
    public struct Connection: Codable, Sendable, Equatable {
        public var autoConnectOnLaunch: Bool
        /// Server-enforced session lifetime in minutes.
        public var sessionMaxMinutes: Int
        public var proactiveReconnect: ProactiveReconnect
        public var reconnectAfterSleep: ReconnectAfterSleep

        public init(
            autoConnectOnLaunch: Bool = true,
            sessionMaxMinutes: Int = 180,
            proactiveReconnect: ProactiveReconnect = ProactiveReconnect(),
            reconnectAfterSleep: ReconnectAfterSleep = ReconnectAfterSleep()
        ) {
            self.autoConnectOnLaunch = autoConnectOnLaunch
            self.sessionMaxMinutes = sessionMaxMinutes
            self.proactiveReconnect = proactiveReconnect
            self.reconnectAfterSleep = reconnectAfterSleep
        }

        enum CodingKeys: String, CodingKey, CaseIterable {
            case autoConnectOnLaunch = "auto_connect_on_launch"
            case sessionMaxMinutes = "session_max_minutes"
            case proactiveReconnect = "proactive_reconnect"
            case reconnectAfterSleep = "reconnect_after_sleep"
        }

        public init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            let defaults = Connection()
            autoConnectOnLaunch = try container.decodeIfPresent(Bool.self, forKey: .autoConnectOnLaunch)
                ?? defaults.autoConnectOnLaunch
            sessionMaxMinutes = try container.decodeIfPresent(Int.self, forKey: .sessionMaxMinutes)
                ?? defaults.sessionMaxMinutes
            proactiveReconnect = try container.decodeIfPresent(ProactiveReconnect.self, forKey: .proactiveReconnect)
                ?? defaults.proactiveReconnect
            reconnectAfterSleep = try container.decodeIfPresent(ReconnectAfterSleep.self, forKey: .reconnectAfterSleep)
                ?? defaults.reconnectAfterSleep
        }

        /// `connection.proactive_reconnect`: reconnect `marginMinutes` before the session limit.
        public struct ProactiveReconnect: Codable, Sendable, Equatable {
            public var enabled: Bool
            public var marginMinutes: Int

            public init(enabled: Bool = true, marginMinutes: Int = 5) {
                self.enabled = enabled
                self.marginMinutes = marginMinutes
            }

            enum CodingKeys: String, CodingKey, CaseIterable {
                case enabled
                case marginMinutes = "margin_minutes"
            }

            public init(from decoder: any Decoder) throws {
                let container = try decoder.container(keyedBy: CodingKeys.self)
                let defaults = ProactiveReconnect()
                enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? defaults.enabled
                marginMinutes = try container.decodeIfPresent(Int.self, forKey: .marginMinutes) ?? defaults.marginMinutes
            }
        }

        /// `connection.reconnect_after_sleep`: reconnect after waking from a sleep of at least `minSleepMinutes`.
        public struct ReconnectAfterSleep: Codable, Sendable, Equatable {
            public var enabled: Bool
            public var minSleepMinutes: Int

            public init(enabled: Bool = false, minSleepMinutes: Int = 30) {
                self.enabled = enabled
                self.minSleepMinutes = minSleepMinutes
            }

            enum CodingKeys: String, CodingKey, CaseIterable {
                case enabled
                case minSleepMinutes = "min_sleep_minutes"
            }

            public init(from decoder: any Decoder) throws {
                let container = try decoder.container(keyedBy: CodingKeys.self)
                let defaults = ReconnectAfterSleep()
                enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? defaults.enabled
                minSleepMinutes = try container.decodeIfPresent(Int.self, forKey: .minSleepMinutes)
                    ?? defaults.minSleepMinutes
            }
        }
    }
}

/// Limits enforced by ``ConfigCodec`` and ``AppConfig/validate()`` (DESIGN §5.7).
public enum ConfigLimits {
    /// Maximum size of `config.yaml` in bytes (64 KiB).
    public static let maxFileSize = 64 * 1024
    public static let port: ClosedRange<Int> = 1...65535
    public static let sessionMaxMinutes: ClosedRange<Int> = 10...1440
    /// Also must be strictly less than `session_max_minutes`.
    public static let marginMinutes: ClosedRange<Int> = 1...60
    public static let minSleepMinutes: ClosedRange<Int> = 1...1440
    /// Number of pinned certificate digests kept by ``AppConfig/VPN/pinTrustedCert(_:)``.
    public static let maxTrustedCerts = 4
}

/// SHA-256 digest format used by openfortivpn's `trusted-cert` (64 lowercase hex characters).
enum TrustedCertDigest {
    /// Lowercases ASCII `A-F` and returns the digest if it is exactly 64 hex characters.
    static func normalize(_ raw: String) -> String? {
        var scalars = String.UnicodeScalarView()
        for scalar in raw.unicodeScalars {
            switch scalar.value {
            case 0x30...0x39, 0x61...0x66:  // 0-9 a-f
                scalars.append(scalar)
            case 0x41...0x46:  // A-F
                scalars.append(Unicode.Scalar(scalar.value + 0x20)!)
            default:
                return nil
            }
        }
        guard scalars.count == 64 else { return nil }
        return String(scalars)
    }

    static func isValid(_ digest: String) -> Bool {
        normalize(digest) == digest
    }
}

extension AppConfig {
    /// Checks every rule ``ConfigCodec/decode(_:)`` enforces, so a config that passes can be written
    /// with ``ConfigTemplate`` and read back unchanged — including the file size limit, measured on the
    /// rendered template. Unlike decoding, invalid `trusted_certs` entries are errors here (the app only
    /// stores normalized digests). Errors carry no location.
    public func validate() throws(ConfigLoadError) {
        if schemaVersion > AppConfig.currentSchemaVersion {
            throw ConfigLoadError(.newerSchemaVersion(schemaVersion))
        }
        if schemaVersion < 1 {
            throw ConfigLoadError(.outOfRange(path: ConfigPath.schemaVersion, raw: String(schemaVersion),
                                              range: 1...AppConfig.currentSchemaVersion))
        }
        let strings: [(String, String)] = [
            (ConfigPath.host, vpn.host),
            (ConfigPath.username, vpn.username),
            (ConfigPath.realm, vpn.realm),
            (ConfigPath.gmailAddress, otp.gmailAddress),
            (ConfigPath.googleClientID, otp.googleClientID),
        ]
        for (path, value) in strings {
            if let scalar = ConfigText.firstDisallowedScalar(inField: value) {
                throw ConfigLoadError(.invalidCharacter(path: path, scalar: scalar.value))
            }
        }
        for (index, digest) in vpn.trustedCerts.enumerated() where !TrustedCertDigest.isValid(digest) {
            throw ConfigLoadError(.invalidTrustedCert(index: index))
        }
        let ranges: [(String, Int, ClosedRange<Int>)] = [
            (ConfigPath.port, vpn.port, ConfigLimits.port),
            (ConfigPath.sessionMaxMinutes, connection.sessionMaxMinutes, ConfigLimits.sessionMaxMinutes),
            (ConfigPath.marginMinutes, connection.proactiveReconnect.marginMinutes, ConfigLimits.marginMinutes),
            (ConfigPath.minSleepMinutes, connection.reconnectAfterSleep.minSleepMinutes, ConfigLimits.minSleepMinutes),
        ]
        for (path, value, range) in ranges where !range.contains(value) {
            throw ConfigLoadError(.outOfRange(path: path, raw: String(value), range: range))
        }
        let margin = connection.proactiveReconnect.marginMinutes
        let session = connection.sessionMaxMinutes
        if margin >= session {
            throw ConfigLoadError(.marginNotLessThanSession(margin: margin, session: session))
        }
        // Last: rendering is the most expensive check. `encode` only renders the template (no validation).
        let byteCount = ConfigCodec.encode(self).count
        if byteCount > ConfigLimits.maxFileSize {
            throw ConfigLoadError(.fileTooLarge(byteCount: byteCount, limit: ConfigLimits.maxFileSize))
        }
    }
}

/// Dotted key paths used in diagnostics.
enum ConfigPath {
    static let schemaVersion = "schema_version"
    static let vpn = "vpn"
    static let host = "vpn.host"
    static let port = "vpn.port"
    static let username = "vpn.username"
    static let realm = "vpn.realm"
    static let trustedCerts = "vpn.trusted_certs"
    static let otp = "otp"
    static let gmailAddress = "otp.gmail_address"
    static let otpMethod = "otp.method"
    static let googleClientID = "otp.google_client_id"
    static let connection = "connection"
    static let autoConnectOnLaunch = "connection.auto_connect_on_launch"
    static let sessionMaxMinutes = "connection.session_max_minutes"
    static let proactiveReconnect = "connection.proactive_reconnect"
    static let proactiveEnabled = "connection.proactive_reconnect.enabled"
    static let marginMinutes = "connection.proactive_reconnect.margin_minutes"
    static let reconnectAfterSleep = "connection.reconnect_after_sleep"
    static let sleepEnabled = "connection.reconnect_after_sleep.enabled"
    static let minSleepMinutes = "connection.reconnect_after_sleep.min_sleep_minutes"
}
