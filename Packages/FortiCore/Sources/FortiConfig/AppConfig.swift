import FortiShared
import Foundation

/// Non-secret settings stored in `~/.config/FortiVPNManager/config.yaml`.
///
/// Passwords are never part of this model; they live in the login Keychain. Defaults match the
/// fixed template written by ``ConfigTemplate``. The YAML file is always read through
/// ``ConfigCodec`` (strict validation) and written through ``ConfigTemplate``.
public struct AppConfig: Sendable, Equatable {
    /// The `schema_version` this build reads and writes.
    public static let currentSchemaVersion = 1

    public var vpn: VPN
    public var otp: OTP
    public var connection: Connection
    public var routes: Routes

    public init(
        vpn: VPN = VPN(),
        otp: OTP = OTP(),
        connection: Connection = Connection(),
        routes: Routes = Routes()
    ) {
        self.vpn = vpn
        self.otp = otp
        self.connection = connection
        self.routes = routes
    }

    enum CodingKeys: String {
        case schemaVersion = "schema_version"
        case vpn
        case otp
        case connection
        case routes
    }

    /// `vpn:` section.
    public struct VPN: Sendable, Equatable {
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

        enum CodingKeys: String {
            case host
            case port
            case username
            case realm
            case trustedCerts = "trusted_certs"
        }
    }

    /// `otp:` section.
    public struct OTP: Sendable, Equatable {
        /// How the app reads the OTP emails (`otp.method`).
        public enum Method: String, Sendable, CaseIterable {
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
        /// OAuth client ID of an imported Google Cloud "Desktop app" client, used by ``Method/gmailAPI``
        /// instead of the built-in one. Not a secret; its client secret is in the Keychain. Empty when no
        /// client was imported.
        public var googleClientID: String

        public init(gmailAddress: String = "", method: Method = .imap, googleClientID: String = "") {
            self.gmailAddress = gmailAddress
            self.method = method
            self.googleClientID = googleClientID
        }

        enum CodingKeys: String {
            case gmailAddress = "gmail_address"
            case method
            case googleClientID = "google_client_id"
        }
    }

    /// `connection:` section.
    public struct Connection: Sendable, Equatable {
        public var autoConnectOnLaunch: Bool
        /// Server-enforced session lifetime in minutes.
        public var sessionMaxMinutes: Int
        public var proactiveReconnect: ProactiveReconnect
        public var reconnectAfterSleep: ReconnectAfterSleep
        public var schedule: Schedule

        public init(
            autoConnectOnLaunch: Bool = true,
            sessionMaxMinutes: Int = 180,
            proactiveReconnect: ProactiveReconnect = ProactiveReconnect(),
            reconnectAfterSleep: ReconnectAfterSleep = ReconnectAfterSleep(),
            schedule: Schedule = Schedule()
        ) {
            self.autoConnectOnLaunch = autoConnectOnLaunch
            self.sessionMaxMinutes = sessionMaxMinutes
            self.proactiveReconnect = proactiveReconnect
            self.reconnectAfterSleep = reconnectAfterSleep
            self.schedule = schedule
        }

        enum CodingKeys: String {
            case autoConnectOnLaunch = "auto_connect_on_launch"
            case sessionMaxMinutes = "session_max_minutes"
            case proactiveReconnect = "proactive_reconnect"
            case reconnectAfterSleep = "reconnect_after_sleep"
            case schedule
        }

        /// `connection.proactive_reconnect`: reconnect `marginMinutes` before the session limit.
        public struct ProactiveReconnect: Sendable, Equatable {
            public var enabled: Bool
            public var marginMinutes: Int

            public init(enabled: Bool = true, marginMinutes: Int = 5) {
                self.enabled = enabled
                self.marginMinutes = marginMinutes
            }

            enum CodingKeys: String {
                case enabled
                case marginMinutes = "margin_minutes"
            }
        }

        /// `connection.reconnect_after_sleep`: reconnect after waking from a sleep of at least `minSleepMinutes`.
        public struct ReconnectAfterSleep: Sendable, Equatable {
            public var enabled: Bool
            public var minSleepMinutes: Int

            public init(enabled: Bool = false, minSleepMinutes: Int = 30) {
                self.enabled = enabled
                self.minSleepMinutes = minSleepMinutes
            }

            enum CodingKeys: String {
                case enabled
                case minSleepMinutes = "min_sleep_minutes"
            }
        }

        /// `connection.schedule`: when `enabled`, connect at `start` on `days`. The end never
        /// disconnects; after it, reconnects wait while the user is away and stop once the user
        /// has been away for `awayMinutes`.
        public struct Schedule: Sendable, Equatable {
            /// Weekday spelling in the file, in display order (Monday first).
            public enum Day: String, Sendable, CaseIterable {
                case mon, tue, wed, thu, fri, sat, sun

                /// `Calendar` weekday number (1 = Sunday … 7 = Saturday).
                public var calendarWeekday: Int {
                    switch self {
                    case .sun: 1
                    case .mon: 2
                    case .tue: 3
                    case .wed: 4
                    case .thu: 5
                    case .fri: 6
                    case .sat: 7
                    }
                }
            }

            public var enabled: Bool
            /// Selected weekdays without repeats, in ``Day/allCases`` order (see ``normalizedDays(_:)``).
            public var days: [Day]
            /// Must be earlier than `end`.
            public var start: TimeOfDay
            public var end: TimeOfDay
            public var awayMinutes: Int

            public init(
                enabled: Bool = false,
                days: [Day] = [.mon, .tue, .wed, .thu, .fri],
                start: TimeOfDay = TimeOfDay(hour: 9, minute: 0),
                end: TimeOfDay = TimeOfDay(hour: 18, minute: 0),
                awayMinutes: Int = 120
            ) {
                self.enabled = enabled
                self.days = Self.normalizedDays(days)
                self.start = start
                self.end = end
                self.awayMinutes = awayMinutes
            }

            /// Removes repeats and sorts into ``Day/allCases`` order.
            public static func normalizedDays(_ days: some Sequence<Day>) -> [Day] {
                let set = Set(days)
                return Day.allCases.filter(set.contains)
            }

            enum CodingKeys: String {
                case enabled
                case days
                case start
                case end
                case awayMinutes = "away_minutes"
            }
        }
    }

    /// `routes:` section, the exception rules. Entries of one list never overlap.
    public struct Routes: Sendable, Equatable {
        /// Networks where the rules apply: only while this Mac's IPv4 address is inside one of them.
        /// Elsewhere everything goes through the VPN. Never filled in by the app.
        public var networks: [IPv4Network]
        /// Exception targets, hosts and networks that skip the VPN there: kept as the additional
        /// routes of the primary network service, through its router, whether or not the VPN is connected.
        public var bypassVPN: [IPv4Network]

        public init(networks: [IPv4Network] = [], bypassVPN: [IPv4Network] = []) {
            self.networks = networks
            self.bypassVPN = bypassVPN
        }

        enum CodingKeys: String {
            case networks
            case bypassVPN = "bypass_vpn"
        }
    }
}

/// A wall-clock time of day with minute precision, written `HH:mm` (`00:00`-`23:59`).
public struct TimeOfDay: Sendable, Hashable, Comparable, CustomStringConvertible {
    /// Minutes after midnight, `0..<1440`.
    public let minutes: Int

    /// `hour` 0-23, `minute` 0-59.
    public init(hour: Int, minute: Int) {
        minutes = hour * 60 + minute
    }

    /// `H:mm` or `HH:mm`, hour `0`-`23`, minute `00`-`59`; `nil` for anything else.
    public init?(parsing text: String) {
        let parts = text.split(separator: ":", omittingEmptySubsequences: false)
        guard parts.count == 2, (1...2).contains(parts[0].count), parts[1].count == 2,
              parts.allSatisfy({ $0.utf8.allSatisfy { (0x30...0x39).contains($0) } }),
              let hour = Int(parts[0]), let minute = Int(parts[1]), hour < 24, minute < 60
        else { return nil }
        minutes = hour * 60 + minute
    }

    public var hour: Int { minutes / 60 }
    public var minute: Int { minutes % 60 }

    /// `HH:mm`.
    public var description: String {
        let hh = hour < 10 ? "0\(hour)" : "\(hour)"
        let mm = minute < 10 ? "0\(minute)" : "\(minute)"
        return "\(hh):\(mm)"
    }

    public static func < (lhs: TimeOfDay, rhs: TimeOfDay) -> Bool {
        lhs.minutes < rhs.minutes
    }
}

/// Limits enforced by ``ConfigCodec`` and ``AppConfig/validate()``.
public enum ConfigLimits {
    /// Maximum size of `config.yaml` in bytes.
    public static let maxFileSize = 64 * 1024
    public static let port: ClosedRange<Int> = 1...65535
    public static let sessionMaxMinutes: ClosedRange<Int> = 10...1440
    /// Also must be strictly less than `session_max_minutes`.
    public static let marginMinutes: ClosedRange<Int> = 1...60
    public static let minSleepMinutes: ClosedRange<Int> = 1...1440
    /// `connection.schedule.away_minutes`.
    public static let awayMinutes: ClosedRange<Int> = 1...1440
    /// Number of certificate digests the app keeps in `trusted_certs` (stated in the template;
    /// FortiCore's `TrustedCertPins.limit` enforces it).
    public static let maxTrustedCerts = 4
    /// Entries in each of `routes.networks` and `routes.bypass_vpn`.
    public static let maxRouteEntries = IPv4Network.maxCount
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
            (ConfigPath.awayMinutes, connection.schedule.awayMinutes, ConfigLimits.awayMinutes),
        ]
        for (path, value, range) in ranges where !range.contains(value) {
            throw ConfigLoadError(.outOfRange(path: path, raw: String(value), range: range))
        }
        let margin = connection.proactiveReconnect.marginMinutes
        let session = connection.sessionMaxMinutes
        if margin >= session {
            throw ConfigLoadError(.marginNotLessThanSession(margin: margin, session: session))
        }
        let schedule = connection.schedule
        if schedule.start >= schedule.end {
            throw ConfigLoadError(.scheduleEndNotAfterStart(start: schedule.start, end: schedule.end))
        }
        for (path, list) in [(ConfigPath.routeNetworks, routes.networks), (ConfigPath.bypassVPN, routes.bypassVPN)] {
            if list.count > ConfigLimits.maxRouteEntries {
                throw ConfigLoadError(.tooManyRouteEntries(path: path, count: list.count, limit: ConfigLimits.maxRouteEntries))
            }
            for (index, entry) in list.enumerated() {
                if list[..<index].contains(entry) {
                    throw ConfigLoadError(.duplicateRouteEntry(path: path, index: index))
                }
                if let other = list[..<index].first(where: { $0.overlaps(entry) }) {
                    throw ConfigLoadError(.overlappingRouteEntry(path: path, index: index, other: other))
                }
            }
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
    static let schedule = "connection.schedule"
    static let scheduleEnabled = "connection.schedule.enabled"
    static let scheduleDays = "connection.schedule.days"
    static let scheduleStart = "connection.schedule.start"
    static let scheduleEnd = "connection.schedule.end"
    static let awayMinutes = "connection.schedule.away_minutes"
    static let routes = "routes"
    static let routeNetworks = "routes.networks"
    static let bypassVPN = "routes.bypass_vpn"
}
