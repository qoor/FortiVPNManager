import FortiShared

/// A fact recognised in one openfortivpn 1.24.1 (stdout) output line.
public enum OpenFortiVPNSignal: Equatable, Sendable {
    /// TLS connection to the gateway established and its certificate accepted.
    case connectedToGateway
    case authenticated
    /// The gateway answered the tunnel request with HTTP instead of PPP (tunnel mode not allowed
    /// for this user/realm). Printed after `authenticated`.
    case tunnelModeRejected
    /// The gateway certificate was rejected; `digest` is the lowercase SHA-256 hex to pin.
    /// Reported once per distinct digest (the block is printed again by the logout attempt).
    case certificateRejected(digest: String)
    case tunnelUp
}

/// Turns assembled output lines into `OpenFortiVPNSignal`s.
///
/// Feed every `LineAssembler` line with the stream it came from (partial lines are harmless: prompts
/// never match). stdout lines must carry one of log.c's 8-byte prefixes (`"INFO:   "`,
/// `"WARN:   "`, `"ERROR:  "`, `"DEBUG:  "`, log.c:43-51); the level itself is not used, so
/// level changes between versions do not matter. stderr (pppd), `helper` and unknown lines
/// produce no signal.
///
/// The parser keeps the state needed to dedupe certificate digests and to tell when route
/// restoration has finished; use one parser per openfortivpn run.
public struct OpenFortiVPNOutputParser: Sendable {
    /// Rejected certificate digests in first-seen order, without duplicates (`ExitFacts.certRejectedDigests`).
    public private(set) var rejectedCertificateDigests: [String] = []
    /// `"Setting new routes..."` was printed; routes may have been changed.
    public private(set) var routesTouched = false
    /// Route restoration completed after the last `routesTouched`.
    public private(set) var routesRestored = false
    /// Local IPv4 address of the ppp interface from the most recent
    /// `"Got addresses: [ip], ns [ns1, ns2](, ns_suffix [suffix])"` line (io.c:500-515).
    public private(set) var tunnelIP: String?

    /// `"Restoring routes..."` seen; waiting for the next INFO/ERROR line to confirm it returned.
    private var restoringRoutes = false
    /// After `"Received bad header from gateway:"` openfortivpn dumps up to 250 bytes of raw tunnel
    /// data to stdout, newlines included (io.c:404-429, 473-474). Such text could imitate log
    /// lines, so only route restoration is still tracked for the rest of the run.
    private var afterUntrustedDump = false

    public init() {}

    public mutating func parse(_ line: String, source: OutputSource) -> OpenFortiVPNSignal? {
        switch source {
        case .stdout:
            parseOpenFortiVPNLine(Substring(line))
        case .stderr, .helper:
            nil
        }
    }

    // MARK: - stdout (openfortivpn)

    private mutating func parseOpenFortiVPNLine(_ line: Substring) -> OpenFortiVPNSignal? {
        guard let (level, body) = Self.splitLogPrefix(line) else {
            // Unprefixed stdout: /sbin/route output, the raw dump, or a prompt.
            return nil
        }
        // on_ppp_if_down (tunnel.c:157-165) and pppd_terminate (tunnel.c:483-572) only log at
        // INFO/ERROR after ipv4_restore_routes() returns; inside it, only WARN/DEBUG lines and
        // /sbin/route's own unprefixed output appear (ipv4.c:1052-1089), apart from the ERROR
        // "/sbin/route: <errno>." when the binary is missing (ipv4.c:701).
        if restoringRoutes, level == .info || level == .error, !body.hasPrefix("/sbin/route: ") {
            restoringRoutes = false
            routesRestored = true
        }
        guard let signal = classify(body), !afterUntrustedDump else { return nil }
        return signal
    }

    private mutating func classify(_ body: Substring) -> OpenFortiVPNSignal? {
        switch body {
        case "Connected to gateway.":  // tunnel.c:1377
            return .connectedToGateway
        case "Authenticated.":  // tunnel.c:1391
            return .authenticated
        case "Setting new routes...":  // tunnel.c:127
            guard !afterUntrustedDump else { return nil }
            routesTouched = true
            routesRestored = false
            return nil
        case "Tunnel is up and running.":  // tunnel.c:140
            return .tunnelUp
        case "Restoring routes...":  // tunnel.c:158; `routesRestored` is set once it returned
            restoringRoutes = true
            return nil
        case "Received bad header from gateway:":  // io.c:473
            afterUntrustedDump = true
            return nil
        default:
            break
        }
        if body.hasPrefix(Self.addressesPrefix) {  // io.c:500-515
            guard !afterUntrustedDump, let ip = Self.parseAddresses(body.dropFirst(Self.addressesPrefix.count)) else {
                return nil
            }
            tunnelIP = ip
            return nil
        }
        if body.hasPrefix("Could not authenticate to ") {
            // io.c:465 "Could not authenticate to the gateway. Please make sure tunnel mode is allowed ..."
            // tunnel.c:1386 "Could not authenticate to gateway. Please check the password, client certificate, etc."
            return body.contains("tunnel mode") ? .tunnelModeRejected : nil
        }
        if body.hasPrefix(Self.trustedCertPrefix) {  // tunnel.c:915 "    trusted-cert = %s"
            guard !afterUntrustedDump, let digest = Self.parseDigest(body.dropFirst(Self.trustedCertPrefix.count)) else {
                return nil
            }
            guard !rejectedCertificateDigests.contains(digest) else { return nil }
            rejectedCertificateDigests.append(digest)
            return .certificateRejected(digest: digest)
        }
        return nil
    }

    private enum LogLevel {
        case error, warn, info, debug
    }

    /// Splits off one of log.c's prefixes (log.c:43-51); all are exactly 8 bytes.
    private static func splitLogPrefix(_ line: Substring) -> (LogLevel, Substring)? {
        let level: LogLevel
        if line.hasPrefix("INFO:   ") {
            level = .info
        } else if line.hasPrefix("ERROR:  ") {
            level = .error
        } else if line.hasPrefix("WARN:   ") {
            level = .warn
        } else if line.hasPrefix("DEBUG:  ") {
            level = .debug
        } else {
            return nil
        }
        return (level, line.dropFirst(8))
    }

    private static let addressesPrefix = "Got addresses: "
    private static let trustedCertPrefix = "    trusted-cert = "

    /// Parses `[ip], ns [ns1, ns2]` optionally followed by `, ns_suffix [suffix]`; keeps only the IP.
    private static func parseAddresses(_ text: Substring) -> String? {
        guard text.hasPrefix("["), let ipEnd = text.firstIndex(of: "]") else { return nil }
        let ip = text[text.index(after: text.startIndex)..<ipEnd]
        guard isIPv4(ip) else { return nil }
        var rest = text[text.index(after: ipEnd)...]
        guard rest.hasPrefix(", ns ["), let nsEnd = rest.firstIndex(of: "]") else { return nil }
        let servers = rest[rest.index(rest.startIndex, offsetBy: ", ns [".count)..<nsEnd]
            .split(separator: ",")
            .map(trimmingSpaces)
        guard servers.allSatisfy(isIPv4) else { return nil }
        rest = rest[rest.index(after: nsEnd)...]
        if !rest.isEmpty {
            guard rest.hasPrefix(", ns_suffix ["), rest.hasSuffix("]") else { return nil }
        }
        return String(ip)
    }

    private static func isIPv4(_ text: Substring) -> Bool {
        let parts = text.split(separator: ".", omittingEmptySubsequences: false)
        return parts.count == 4 && parts.allSatisfy { part in
            (1...3).contains(part.count) && part.utf8.allSatisfy { (UInt8(ascii: "0")...UInt8(ascii: "9")).contains($0) } && Int(part).map { $0 <= 255 } == true
        }
    }

    /// 64 hex digits (tunnel.c:896-898 prints lowercase `%02x`).
    private static func parseDigest(_ text: Substring) -> String? {
        let digest = trimmingSpaces(text)
        guard digest.utf8.count == 64, digest.allSatisfy({ $0.isASCII && $0.isHexDigit }) else {
            return nil
        }
        return digest.lowercased()
    }

    private static func trimmingSpaces(_ text: Substring) -> Substring {
        var result = text
        while result.first == " " { result = result.dropFirst() }
        while result.last == " " { result = result.dropLast() }
        return result
    }
}
