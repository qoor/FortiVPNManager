import FortiShared

/// A fact recognised in one openfortivpn 1.24.1 (stdout) or pppd (stderr) output line.
public enum OpenFortiVPNSignal: Equatable, Sendable {
    /// TLS connection to the gateway established and its certificate accepted.
    case connectedToGateway
    case authenticated
    /// `/remote/logincheck` failed: wrong password, OTP rejected, no OTP given, ...
    case authenticationFailed
    /// The gateway answered the tunnel request with HTTP instead of PPP (tunnel mode not allowed
    /// for this user/realm). Printed after `authenticated`.
    case tunnelModeRejected
    /// The gateway certificate was rejected; `digest` is the lowercase SHA-256 hex to pin.
    /// Reported once per distinct digest (the block is printed again by the logout attempt).
    case certificateRejected(digest: String)
    case addressesAssigned(TunnelAddresses)
    case interfaceUp(name: String)
    /// openfortivpn is about to change the routing table (gateway /32 route, default route).
    case routesTouched
    case nameserversAdded
    case tunnelUp
    case interfaceDown(name: String)
    /// `ipv4_restore_routes()` has returned.
    case routesRestored
    case nameserversRemoved
    /// openfortivpn reaped pppd. Classified by text only; the INFO/ERROR level depends on whether
    /// openfortivpn itself received SIGTERM/SIGINT (tunnel.c:535-553).
    case pppdExited(PPPDExit)
    /// A link-termination message pppd wrote to stderr.
    case pppdNotice(PPPDNotice)
}

/// Addresses from `"Got addresses: [ip], ns [ns1, ns2](, ns_suffix [suffix])"` (io.c:500-515).
public struct TunnelAddresses: Equatable, Sendable {
    /// Local IPv4 address of the ppp interface.
    public var ip: String
    /// DNS servers, without the `0.0.0.0` placeholders openfortivpn prints for unset entries.
    public var dnsServers: [String]
    public var dnsSuffix: String?

    public init(ip: String, dnsServers: [String], dnsSuffix: String? = nil) {
        self.ip = ip
        self.dnsServers = dnsServers
        self.dnsSuffix = dnsSuffix
    }
}

/// How pppd ended, as reported by openfortivpn's `pppd_terminate()` (tunnel.c:483-572).
public enum PPPDExit: Equatable, Sendable {
    /// pppd exit status, recovered from the pppd(8) message table (tunnel.c:433-455) or from the
    /// numeric "unknown/unexpected exit status code" lines.
    case exitStatus(Int32)
    /// pppd was killed by a signal; the text is `strsignal()` output, e.g. `"Terminated: 15"`.
    case signaled(String)
}

/// Link-termination messages Apple's pppd 2.4.2 writes to `logfd 2` (openfortivpn passes
/// `logfd 2`, tunnel.c:350-357). Sources are apple-oss-distributions/ppp Helpers/pppd.
public enum PPPDNotice: Equatable, Sendable {
    /// `"Modem hangup"` (main.c:802, 1436): the pty closed, e.g. openfortivpn tore the tunnel down.
    case modemHangup
    /// `"Connection terminated."` (auth.c:674).
    case connectionTerminated
    /// `"<proto> terminated by peer"` / `"<proto> terminated by peer (<reason>)"` (fsm.c:684-686).
    case terminatedByPeer(protocolName: String, reason: String?)
    /// `"Terminating on signal <n>."` (main.c:1090).
    case terminatingOnSignal(Int32)
    /// `"Hangup (SIGHUP)"` (main.c:1083).
    case hangup
    /// `"Serial link appears to be disconnected."` (lcp.c:2292).
    case serialLinkDisconnected
}

/// Turns assembled output lines into `OpenFortiVPNSignal`s (DESIGN §3.3-3.6, §5.2).
///
/// Feed every `AssembledLine` with the stream it came from (partial lines are harmless: prompts
/// never match). stdout lines must carry one of log.c's 8-byte prefixes (`"INFO:   "`,
/// `"WARN:   "`, `"ERROR:  "`, `"DEBUG:  "`, log.c:43-51); the level itself is not used, so
/// level changes between versions and the INFO/ERROR `pppd:` ambiguity do not matter. stderr
/// lines are matched as pppd messages with an optional `"%c : "` timestamp. `helper` lines and
/// unknown lines produce no signal.
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
    /// The most recent `addressesAssigned` value.
    public private(set) var tunnelAddresses: TunnelAddresses?

    /// `"Restoring routes..."` seen; waiting for the next INFO/ERROR line to confirm it returned.
    private var restoringRoutes = false
    /// After `"Received bad header from gateway:"` openfortivpn dumps up to 250 bytes of raw tunnel
    /// data to stdout, newlines included (io.c:404-429, 473-474). Such text could imitate log
    /// lines, so only teardown signals are accepted for the rest of the run.
    private var afterUntrustedDump = false

    public init() {}

    public mutating func parse(_ line: String, source: OutputSource) -> [OpenFortiVPNSignal] {
        switch source {
        case .stdout:
            parseOpenFortiVPNLine(Substring(line))
        case .stderr:
            Self.pppdNotice(in: Substring(line)).map { notice -> [OpenFortiVPNSignal] in [.pppdNotice(notice)] } ?? []
        case .helper:
            []
        }
    }

    // MARK: - stdout (openfortivpn)

    private mutating func parseOpenFortiVPNLine(_ line: Substring) -> [OpenFortiVPNSignal] {
        guard let (level, body) = Self.splitLogPrefix(line) else {
            // Unprefixed stdout: /sbin/route output, the raw dump, or a prompt.
            return []
        }
        var signals: [OpenFortiVPNSignal] = []
        // on_ppp_if_down (tunnel.c:157-165) and pppd_terminate (tunnel.c:483-572) only log at
        // INFO/ERROR after ipv4_restore_routes() returns; inside it, only WARN/DEBUG lines and
        // /sbin/route's own unprefixed output appear (ipv4.c:1052-1089), apart from the ERROR
        // "/sbin/route: <errno>." when the binary is missing (ipv4.c:701).
        if restoringRoutes, level == .info || level == .error, !body.hasPrefix("/sbin/route: ") {
            restoringRoutes = false
            routesRestored = true
            signals.append(.routesRestored)
        }
        guard let signal = classify(body) else { return signals }
        if afterUntrustedDump && !Self.isTeardownSignal(signal) {
            return signals
        }
        signals.append(signal)
        return signals
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
            return .routesTouched
        case "Adding VPN nameservers...":  // tunnel.c:136
            return .nameserversAdded
        case "Tunnel is up and running.":  // tunnel.c:140
            return .tunnelUp
        case "Restoring routes...":  // tunnel.c:158; `.routesRestored` follows once it returned
            restoringRoutes = true
            return nil
        case "Removing VPN nameservers...":  // tunnel.c:163
            return .nameserversRemoved
        case "Received bad header from gateway:":  // io.c:473
            afterUntrustedDump = true
            return nil
        default:
            break
        }
        if let name = Self.infix(of: body, prefix: "Interface ", suffix: " is UP.") {  // tunnel.c:114
            return .interfaceUp(name: String(name))
        }
        if let name = Self.infix(of: body, prefix: "Setting ", suffix: " interface down.") {  // tunnel.c:155
            return .interfaceDown(name: String(name))
        }
        if body.hasPrefix(Self.addressesPrefix) {  // io.c:500-515
            guard !afterUntrustedDump, let addresses = Self.parseAddresses(body.dropFirst(Self.addressesPrefix.count)) else {
                return nil
            }
            tunnelAddresses = addresses
            return .addressesAssigned(addresses)
        }
        if body.hasPrefix("Could not authenticate to ") {
            // io.c:465 "Could not authenticate to the gateway. Please make sure tunnel mode is allowed ..."
            // tunnel.c:1386 "Could not authenticate to gateway. Please check the password, client certificate, etc."
            return body.contains("tunnel mode") ? .tunnelModeRejected : .authenticationFailed
        }
        if body.hasPrefix(Self.trustedCertPrefix) {  // tunnel.c:915 "    trusted-cert = %s"
            guard !afterUntrustedDump, let digest = Self.parseDigest(body.dropFirst(Self.trustedCertPrefix.count)) else {
                return nil
            }
            guard !rejectedCertificateDigests.contains(digest) else { return nil }
            rejectedCertificateDigests.append(digest)
            return .certificateRejected(digest: digest)
        }
        if body.hasPrefix("pppd: ") {  // tunnel.c:520-568 (PPP_DAEMON is "pppd", config.h:48)
            return Self.parsePPPDExit(body.dropFirst("pppd: ".count)).map { .pppdExited($0) }
        }
        return nil
    }

    private static func isTeardownSignal(_ signal: OpenFortiVPNSignal) -> Bool {
        switch signal {
        case .interfaceDown, .routesRestored, .nameserversRemoved, .pppdExited:
            true
        default:
            false
        }
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

    private static func infix(of body: Substring, prefix: String, suffix: String) -> Substring? {
        guard body.count > prefix.count + suffix.count, body.hasPrefix(prefix), body.hasSuffix(suffix) else {
            return nil
        }
        let name = body.dropFirst(prefix.count).dropLast(suffix.count)
        return name.contains(" ") ? nil : name
    }

    /// Parses `[ip], ns [ns1, ns2]` optionally followed by `, ns_suffix [suffix]`.
    private static func parseAddresses(_ text: Substring) -> TunnelAddresses? {
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
        var dnsSuffix: String?
        if !rest.isEmpty {
            guard rest.hasPrefix(", ns_suffix ["), rest.hasSuffix("]") else { return nil }
            let suffix = rest.dropFirst(", ns_suffix [".count).dropLast()
            dnsSuffix = suffix.isEmpty ? nil : String(suffix)
        }
        return TunnelAddresses(
            ip: String(ip),
            dnsServers: servers.filter { $0 != "0.0.0.0" }.map(String.init),
            dnsSuffix: dnsSuffix
        )
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

    /// pppd(8) exit status messages, index = status (tunnel.c:433-455, HAVE_USR_SBIN_PPPD).
    static let pppdExitMessages: [String] = [
        "Has detached, or otherwise the connection was successfully established and terminated at the peer's request.",
        "An immediately fatal error of some kind occurred, such as an essential system call failing, or running out of virtual memory.",
        "An error was detected in processing the options given, such as two mutually exclusive options being used.",
        "Is not setuid-root and the invoking user is not root.",
        "The kernel does not support PPP, for example, the PPP kernel driver is not included or cannot be loaded.",
        "Terminated because it was sent a SIGINT, SIGTERM or SIGHUP signal.",
        "The serial port could not be locked.",
        "The serial port could not be opened.",
        "The connect script failed (returned a non-zero exit status).",
        "The command specified as the argument to the pty option could not be run.",
        "The PPP negotiation failed, that is, it didn't reach the point where at least one network protocol (e.g. IP) was running.",
        "The peer system failed (or refused) to authenticate itself.",
        "The link was established successfully and terminated because it was idle.",
        "The link was established successfully and terminated because the connect time limit was reached.",
        "Callback was negotiated and an incoming call should arrive shortly.",
        "The link was terminated because the peer is not responding to echo requests.",
        "The link was terminated by the modem hanging up.",
        "The PPP negotiation failed because serial loopback was detected.",
        "The init script failed (returned a non-zero exit status).",
        "We failed to authenticate ourselves to the peer.",
    ]

    private static func parsePPPDExit(_ message: Substring) -> PPPDExit? {
        if let status = pppdExitMessages.firstIndex(where: { $0 == message }) {
            return .exitStatus(Int32(status))
        }
        // tunnel.c:523 / tunnel.c:552
        for prefix in ["Returned an unknown exit status code: ", "Returned an unexpected exit status code: "]
        where message.hasPrefix(prefix) {
            return Int32(message.dropFirst(prefix.count)).map { .exitStatus($0) }
        }
        if message.hasPrefix("terminated by signal: ") {  // tunnel.c:567-568
            return .signaled(String(message.dropFirst("terminated by signal: ".count)))
        }
        return nil
    }

    private static func trimmingSpaces(_ text: Substring) -> Substring {
        var result = text
        while result.first == " " { result = result.dropFirst() }
        while result.last == " " { result = result.dropLast() }
        return result
    }

    // MARK: - stderr (pppd)

    private static func pppdNotice(in line: Substring) -> PPPDNotice? {
        let message = stripPPPDTimestamp(line)
        switch message {
        case "Modem hangup":
            return .modemHangup
        case "Connection terminated.":
            return .connectionTerminated
        case "Hangup (SIGHUP)":
            return .hangup
        case "Serial link appears to be disconnected.":
            return .serialLinkDisconnected
        default:
            break
        }
        if message.hasPrefix("Terminating on signal ") {
            var number = message.dropFirst("Terminating on signal ".count)
            if number.hasSuffix(".") {
                number = number.dropLast()
            }
            return Int32(number).map { .terminatingOnSignal($0) }
        }
        if let range = message.firstRange(of: " terminated by peer") {
            let protocolName = message[..<range.lowerBound]
            guard !protocolName.isEmpty, !protocolName.contains(" ") else { return nil }
            let rest = message[range.upperBound...]
            if rest.isEmpty {
                return .terminatedByPeer(protocolName: String(protocolName), reason: nil)
            }
            guard rest.hasPrefix(" ("), rest.hasSuffix(")") else { return nil }
            return .terminatedByPeer(protocolName: String(protocolName), reason: String(rest.dropFirst(2).dropLast()))
        }
        return nil
    }

    /// Removes Apple pppd's `strftime("%c : ")` prefix (utils.c:755; C locale, e.g.
    /// `"Fri Apr  5 11:31:24 2024 : "`). pppd writes timestamp, message and LF separately
    /// (utils.c:756-763), so a fragment may also arrive without it.
    static func stripPPPDTimestamp(_ line: Substring) -> Substring {
        // A = ASCII letter, _ = space or digit, 9 = digit.
        let shape = Array("AAA AAA _9 99:99:99 9999 : ".utf8)
        let bytes = line.utf8
        guard bytes.count >= shape.count else { return line }
        for (expected, actual) in zip(shape, bytes) {
            let matches = switch expected {
            case UInt8(ascii: "A"): (UInt8(ascii: "A")...UInt8(ascii: "Z")).contains(actual)
                || (UInt8(ascii: "a")...UInt8(ascii: "z")).contains(actual)
            case UInt8(ascii: "9"): (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(actual)
            case UInt8(ascii: "_"): actual == UInt8(ascii: " ") || (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(actual)
            default: actual == expected
            }
            guard matches else { return line }
        }
        let start = bytes.index(bytes.startIndex, offsetBy: shape.count)
        return line[start...]
    }
}

