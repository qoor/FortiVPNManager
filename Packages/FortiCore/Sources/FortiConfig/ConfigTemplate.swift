import Foundation

/// Renders the fixed, commented `config.yaml` template (DESIGN §5.7, config.yaml schema).
///
/// Strings are always emitted as double-quoted YAML scalars with escapes, so
/// `ConfigCodec.decode(render(c))` returns `c` for every config that passes ``AppConfig/validate()``.
/// Values that fail validation are still emitted as well-formed YAML (and then rejected on decode).
public enum ConfigTemplate {
    public static func render(_ config: AppConfig) -> String {
        let vpn = config.vpn
        let connection = config.connection
        let proactive = connection.proactiveReconnect
        let afterSleep = connection.reconnectAfterSleep
        let schedule = connection.schedule
        let days = AppConfig.Connection.Schedule.normalizedDays(schedule.days).map(\.rawValue).joined(separator: ", ")
        return """
            # FortiVPNManager configuration file
            #
            # - Passwords (VPN password, Gmail app password) and the Google sign-in live in the macOS login Keychain, not in this file.
            # - Saving in Settings makes the app rewrite this file. Comments you added and unknown keys are removed then.
            # - Do not write leading zeros in numbers (0443 is an error). Write numbers and true/false without quotes.
            # - This file contains the server and account details, so do not commit it to a public repository (dotfiles etc.).
            # - Changes take effect from the next connection. The session max time and proactive reconnect margin are recalculated immediately.
            schema_version: \(AppConfig.currentSchemaVersion)
            vpn:
              # VPN gateway address (host name or IPv4 address).
              host: \(quoted(vpn.host))
              # Filled in on first launch from an existing openfortivpn config, if any (e.g. 8443). 443 is the default when the file has no value.
              port: \(vpn.port)
              # VPN username. The password is stored in the Keychain per 'username@server address'.
              username: \(quoted(vpn.username))
              # Authentication realm. Leave it empty ("") if not needed.
              realm: \(quoted(vpn.realm))
              # SHA-256 digests of the server certificate. The app fills these in on the first connection and manages them; no need to edit by hand.
              # When the certificate changes, the app automatically adds the new value at the front (up to \(ConfigLimits.maxTrustedCerts) entries).
              trusted_certs:\(trustedCerts(vpn.trustedCerts))
            otp:
              # Gmail (Google Workspace) address that receives the OTP emails. The app password is in the Keychain.
              gmail_address: \(quoted(config.otp.gmailAddress))
              # How the app reads the OTP emails: "imap" (Gmail app password) or "gmail_api" (Gmail API with Google sign-in; falls back to IMAP when the sign-in fails and an app password is stored).
              method: \(quoted(config.otp.method.rawValue))
              # OAuth client ID of the Google Cloud desktop app used by "gmail_api". Set by importing the client JSON in Settings; its client secret is in the Keychain.
              google_client_id: \(quoted(config.otp.googleClientID))
            connection:
              # Connect automatically when the app starts.
              auto_connect_on_launch: \(connection.autoConnectOnLaunch)
              # Time until the server forcibly ends the session (minutes, \(range(ConfigLimits.sessionMaxMinutes))).
              session_max_minutes: \(connection.sessionMaxMinutes)
              # When enabled is true, reconnect ahead of time, margin_minutes minutes (\(range(ConfigLimits.marginMinutes)), less than session_max_minutes) before the session max time runs out.
              proactive_reconnect: { enabled: \(proactive.enabled), margin_minutes: \(proactive.marginMinutes) }
              # When enabled is true, reconnect on wake after sleeping for at least min_sleep_minutes minutes (\(range(ConfigLimits.minSleepMinutes))) while connected.
              reconnect_after_sleep: { enabled: \(afterSleep.enabled), min_sleep_minutes: \(afterSleep.minSleepMinutes) }
              # Scheduled hours. When enabled is true, connect at start ("HH:mm") on the listed days (mon, tue, wed, thu, fri, sat, sun); start must be earlier than end.
              # The end never disconnects. After it, reconnects wait while you are away (screen locked or off, or the Mac asleep) and stop once you have been away for away_minutes minutes (\(range(ConfigLimits.awayMinutes))).
              schedule: { enabled: \(schedule.enabled), days: [\(days)], start: \(quoted(schedule.start.description)), end: \(quoted(schedule.end.description)), away_minutes: \(schedule.awayMinutes) }

            """
    }

    private static func range(_ range: ClosedRange<Int>) -> String {
        "\(range.lowerBound)-\(range.upperBound)"
    }

    private static func trustedCerts(_ digests: [String]) -> String {
        if digests.isEmpty { return " []" }
        return digests.map { "\n    - \(quoted($0))" }.joined()
    }

    /// YAML double-quoted scalar. Escapes `"` and `\`, every character the decoder rejects in string
    /// fields (controls, U+2028/2029, BOM, noncharacters) and everything libyaml cannot read raw, so the
    /// result is always one well-formed line.
    static func quoted(_ value: String) -> String {
        var out = "\""
        for scalar in value.unicodeScalars {
            switch scalar {
            case "\"": out += "\\\""
            case "\\": out += "\\\\"
            case "\0": out += "\\0"
            case "\u{07}": out += "\\a"
            case "\u{08}": out += "\\b"
            case "\t": out += "\\t"
            case "\n": out += "\\n"
            case "\u{0B}": out += "\\v"
            case "\u{0C}": out += "\\f"
            case "\r": out += "\\r"
            case "\u{1B}": out += "\\e"
            case "\u{85}": out += "\\N"
            case "\u{2028}": out += "\\L"
            case "\u{2029}": out += "\\P"
            default:
                if ConfigText.isDisallowedInField(scalar) || !ConfigText.isReadableByLibYAML(scalar) {
                    out += hexEscape(scalar.value)
                } else {
                    out.unicodeScalars.append(scalar)
                }
            }
        }
        out += "\""
        return out
    }

    private static func hexEscape(_ value: UInt32) -> String {
        let (prefix, width) = value <= 0xFF ? ("\\x", 2) : value <= 0xFFFF ? ("\\u", 4) : ("\\U", 8)
        let hex = String(value, radix: 16, uppercase: true)
        return prefix + String(repeating: "0", count: max(0, width - hex.count)) + hex
    }
}
