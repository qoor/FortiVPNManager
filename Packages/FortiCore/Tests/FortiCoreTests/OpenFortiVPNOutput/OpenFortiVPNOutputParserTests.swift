@testable import FortiCore
import FortiShared
import Testing

@Suite struct OpenFortiVPNOutputParserTests {
    static let digest = "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11"
    static let otherDigest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

    private func parse(_ lines: [String], source: OutputSource = .stdout) -> [OpenFortiVPNSignal] {
        var parser = OpenFortiVPNOutputParser()
        return lines.flatMap { parser.parse($0, source: source) }
    }

    /// Gateway certificate rejection block, tunnel.c:912-934.
    static func certificateBlock(_ digest: String) -> [String] {
        [
            "ERROR:  Gateway certificate validation failed, and the certificate digest is not in the local whitelist. If you trust it, rerun with:",
            "ERROR:      --trusted-cert \(digest)",
            "ERROR:  or add this line to your configuration file:",
            "ERROR:      trusted-cert = \(digest)",
            "ERROR:  Gateway certificate:",
            "ERROR:      subject:",
            "ERROR:          C=US",
            "ERROR:          O=Fortinet",
            "ERROR:          CN=FGT60FTK00000000",
            "ERROR:      issuer:",
            "ERROR:          CN=fortinet-subca2001",
            "ERROR:      sha256 digest:",
            "ERROR:          \(digest)",
        ]
    }

    // MARK: Milestones and simple lines

    @Test(arguments: [
        ("INFO:   Connected to gateway.", OpenFortiVPNSignal.connectedToGateway),
        ("INFO:   Authenticated.", .authenticated),
        ("INFO:   Tunnel is up and running.", .tunnelUp),
        ("INFO:   Interface ppp0 is UP.", .interfaceUp(name: "ppp0")),
        ("INFO:   Interface ppp12 is UP.", .interfaceUp(name: "ppp12")),
        ("INFO:   Setting new routes...", .routesTouched),
        ("INFO:   Adding VPN nameservers...", .nameserversAdded),
        ("INFO:   Setting ppp0 interface down.", .interfaceDown(name: "ppp0")),
        ("INFO:   Removing VPN nameservers...", .nameserversRemoved),
        ("ERROR:  Could not authenticate to gateway. Please check the password, client certificate, etc.", .authenticationFailed),
        ("ERROR:  Could not authenticate to gateway.", .authenticationFailed),
        (
            "ERROR:  Could not authenticate to the gateway. Please make sure tunnel mode is allowed by the gateway, check the realm, etc.",
            .tunnelModeRejected
        ),
    ])
    func recognisesLine(line: String, expected: OpenFortiVPNSignal) {
        #expect(parse([line]) == [expected])
    }

    @Test(arguments: [
        "",
        "VPN account password: ",
        "Two-factor authentication token: ",
        "Please enter one-time password: ",
        "Connected to gateway.",  // no log.c prefix
        "INFO:  Connected to gateway.",  // 7-byte prefix
        "INFO:   Connected to gateway. ",
        "INFO:   connected to gateway.",
        "INFO:   Remote gateway has allocated a VPN.",
        "INFO:   Negotiation complete.",
        "INFO:   Closed connection to gateway.",
        "INFO:   Logged out.",
        "INFO:   Terminated pppd.",
        "INFO:   Restoring routes...",
        "INFO:   Interface  is UP.",
        "INFO:   Interface ppp0 eth0 is UP.",
        "WARN:   Removing wrong route to vpn server...",
        "add host 203.0.113.10: gateway 192.168.1.1",
        "route: writing to routing socket: File exists",
        "  (raw) INFO:   Tunnel is up and running.",
        "ERROR:  /usr/sbin/pppd: No such file or directory.",
        "ERROR:  pppd: something new",
    ])
    func ignoresUnknownLines(line: String) {
        #expect(parse([line]).isEmpty)
    }

    @Test func ignoresHelperMetaLines() {
        #expect(parse(["INFO:   Connected to gateway.", "Modem hangup"], source: .helper).isEmpty)
    }

    @Test func doesNotTreatOpenFortiVPNLinesOnStderrAsSignals() {
        #expect(parse(["INFO:   Connected to gateway.", "ERROR:      trusted-cert = \(Self.digest)"], source: .stderr).isEmpty)
    }

    // MARK: Got addresses

    @Test(arguments: [
        (
            "[10.212.134.200], ns [10.0.0.53, 10.0.0.54]",
            TunnelAddresses(ip: "10.212.134.200", dnsServers: ["10.0.0.53", "10.0.0.54"])
        ),
        (
            "[10.212.134.200], ns [10.0.0.53, 10.0.0.54], ns_suffix [corp.example.com]",
            TunnelAddresses(ip: "10.212.134.200", dnsServers: ["10.0.0.53", "10.0.0.54"], dnsSuffix: "corp.example.com")
        ),
        (
            "[10.1.2.3], ns [10.0.0.53, 0.0.0.0]",
            TunnelAddresses(ip: "10.1.2.3", dnsServers: ["10.0.0.53"])
        ),
        (
            "[10.1.2.3], ns [0.0.0.0, 0.0.0.0], ns_suffix [a.example;b.example]",
            TunnelAddresses(ip: "10.1.2.3", dnsServers: [], dnsSuffix: "a.example;b.example")
        ),
        (
            "[10.1.2.3], ns [0.0.0.0, 0.0.0.0], ns_suffix []",
            TunnelAddresses(ip: "10.1.2.3", dnsServers: [])
        ),
    ])
    func parsesGotAddresses(payload: String, expected: TunnelAddresses) {
        var parser = OpenFortiVPNOutputParser()
        #expect(parser.parse("INFO:   Got addresses: \(payload)", source: .stdout) == [.addressesAssigned(expected)])
        #expect(parser.tunnelAddresses == expected)
    }

    @Test(arguments: [
        "[10.1.2], ns [10.0.0.53, 0.0.0.0]",
        "[999.1.2.3], ns [10.0.0.53, 0.0.0.0]",
        "[10.1.2.3]",
        "[10.1.2.3], ns [10.0.0.53, 0.0.0.0",
        "[10.1.2.3], ns [dns.example, 0.0.0.0]",
        "[10.1.2.3], ns [10.0.0.53, 0.0.0.0], something [x]",
        "10.1.2.3, ns [10.0.0.53, 0.0.0.0]",
    ])
    func rejectsMalformedGotAddresses(payload: String) {
        var parser = OpenFortiVPNOutputParser()
        #expect(parser.parse("INFO:   Got addresses: \(payload)", source: .stdout).isEmpty)
        #expect(parser.tunnelAddresses == nil)
    }

    @Test func keepsTheLatestAddresses() {
        var parser = OpenFortiVPNOutputParser()
        _ = parser.parse("INFO:   Got addresses: [10.1.2.3], ns [0.0.0.0, 0.0.0.0]", source: .stdout)
        _ = parser.parse("INFO:   Got addresses: [10.1.2.4], ns [0.0.0.0, 0.0.0.0]", source: .stdout)
        #expect(parser.tunnelAddresses?.ip == "10.1.2.4")
    }

    // MARK: Certificate rejection

    @Test func extractsTheDigestFromTheTrustedCertLineOnly() {
        var parser = OpenFortiVPNOutputParser()
        let signals = Self.certificateBlock(Self.digest).flatMap { parser.parse($0, source: .stdout) }
        #expect(signals == [.certificateRejected(digest: Self.digest)])
        #expect(parser.rejectedCertificateDigests == [Self.digest])
    }

    @Test func dedupesARepeatedBlock() {
        // The block is printed again when openfortivpn reconnects to log out (tunnel.c:1455).
        var parser = OpenFortiVPNOutputParser()
        let lines = Self.certificateBlock(Self.digest) + ["INFO:   Closed connection to gateway."]
            + Self.certificateBlock(Self.digest) + ["INFO:   Could not log out."]
        let signals = lines.flatMap { parser.parse($0, source: .stdout) }
        #expect(signals == [.certificateRejected(digest: Self.digest)])
        #expect(parser.rejectedCertificateDigests == [Self.digest])
    }

    @Test func keepsDistinctDigestsInOrder() {
        var parser = OpenFortiVPNOutputParser()
        let lines = Self.certificateBlock(Self.otherDigest) + Self.certificateBlock(Self.digest)
            + Self.certificateBlock(Self.otherDigest)
        let signals = lines.flatMap { parser.parse($0, source: .stdout) }
        #expect(signals == [.certificateRejected(digest: Self.otherDigest), .certificateRejected(digest: Self.digest)])
        #expect(parser.rejectedCertificateDigests == [Self.otherDigest, Self.digest])
    }

    @Test func recognisesARejectionAfterAuthentication() {
        // E.g. the second TLS connection after VPN allocation (tunnel.c:1403).
        let lines = ["INFO:   Connected to gateway.", "INFO:   Authenticated.", "INFO:   Remote gateway has allocated a VPN."]
            + Self.certificateBlock(Self.digest)
        #expect(parse(lines) == [.connectedToGateway, .authenticated, .certificateRejected(digest: Self.digest)])
    }

    @Test func normalisesAnUppercaseDigest() {
        #expect(parse(["ERROR:      trusted-cert = \(Self.digest.uppercased())"]) == [.certificateRejected(digest: Self.digest)])
    }

    @Test(arguments: [
        String(OpenFortiVPNOutputParserTests.digest.dropLast()),
        OpenFortiVPNOutputParserTests.digest + "0",
        String(OpenFortiVPNOutputParserTests.digest.dropLast()) + "g",
        "",
    ])
    func rejectsMalformedDigests(candidate: String) {
        #expect(parse(["ERROR:      trusted-cert = \(candidate)"]).isEmpty)
    }

    // MARK: Routes

    @Test func reportsRoutesRestoredOnlyOnceRestoreReturned() {
        var parser = OpenFortiVPNOutputParser()
        #expect(parser.parse("INFO:   Setting new routes...", source: .stdout) == [.routesTouched])
        #expect(parser.routesTouched && !parser.routesRestored)
        let duringRestore = [
            "INFO:   Setting ppp0 interface down.",
            "INFO:   Restoring routes...",
            "delete host 203.0.113.10: gateway 192.168.1.1",
            "DEBUG:  /sbin/route -n add -net 0.0.0.0 -netmask 0.0.0.0 192.168.1.1",
            "WARN:   Could not restore default route (File exists). Already restored?",
            "ERROR:  /sbin/route: No such file or directory.",
        ]
        #expect(duringRestore.flatMap { parser.parse($0, source: .stdout) } == [.interfaceDown(name: "ppp0")])
        #expect(!parser.routesRestored)
        #expect(parser.parse("INFO:   Removing VPN nameservers...", source: .stdout) == [.routesRestored, .nameserversRemoved])
        #expect(parser.routesTouched && parser.routesRestored)
        #expect(parser.parse("INFO:   Terminated pppd.", source: .stdout).isEmpty)
    }

    @Test(arguments: [
        ("ERROR:  pppd: The link was terminated by the modem hanging up.", [OpenFortiVPNSignal.routesRestored, .pppdExited(.exitStatus(16))]),
        ("INFO:   Terminated pppd.", [.routesRestored]),
        ("ERROR:  waitpid: No child processes", [.routesRestored]),
    ])
    func completesRestoreOnTheNextInfoOrErrorLine(line: String, expected: [OpenFortiVPNSignal]) {
        // With set-dns = 0 there is no "Removing VPN nameservers..." line.
        var parser = OpenFortiVPNOutputParser()
        _ = parser.parse("INFO:   Setting new routes...", source: .stdout)
        _ = parser.parse("INFO:   Restoring routes...", source: .stdout)
        #expect(parser.parse(line, source: .stdout) == expected)
    }

    @Test func leavesRoutesUnrestoredWhenTheRunEndsMidRestore() {
        var parser = OpenFortiVPNOutputParser()
        for line in ["INFO:   Setting new routes...", "INFO:   Tunnel is up and running.", "INFO:   Restoring routes..."] {
            _ = parser.parse(line, source: .stdout)
        }
        #expect(parser.routesTouched && !parser.routesRestored)
    }

    @Test func leavesRoutesUnrestoredWhenTheTunnelNeverCameUp() {
        // SIGTERM while on_ppp_if_up runs: state never reaches STATE_UP, so on_ppp_if_down is
        // skipped (tunnel.c:1442-1444).
        var parser = OpenFortiVPNOutputParser()
        let lines = [
            "INFO:   Interface ppp0 is UP.",
            "INFO:   Setting new routes...",
            "INFO:   Cancelling threads...",
            "INFO:   Cleanup, joining threads...",
            "INFO:   pppd: The link was terminated by the modem hanging up.",
            "INFO:   Terminated pppd.",
        ]
        #expect(lines.flatMap { parser.parse($0, source: .stdout) } == [
            .interfaceUp(name: "ppp0"), .routesTouched, .pppdExited(.exitStatus(16)),
        ])
        #expect(parser.routesTouched && !parser.routesRestored)
    }

    // MARK: pppd exit (openfortivpn stdout)

    @Test(arguments: ["INFO:   ", "ERROR:  "])
    func classifiesTheModemHangupLineWithoutItsPrefix(prefix: String) {
        // tunnel.c:535-550: INFO after SIGTERM/SIGINT, ERROR otherwise.
        #expect(parse(["\(prefix)pppd: The link was terminated by the modem hanging up."]) == [.pppdExited(.exitStatus(16))])
    }

    @Test func mapsEveryPPPDMessageToItsExitStatus() {
        let messages = OpenFortiVPNOutputParser.pppdExitMessages
        #expect(messages.count == 20)
        for (status, message) in messages.enumerated() {
            #expect(parse(["ERROR:  pppd: \(message)"]) == [.pppdExited(.exitStatus(Int32(status)))])
        }
    }

    @Test(arguments: [
        ("ERROR:  pppd: Returned an unknown exit status code: 42", PPPDExit.exitStatus(42)),
        ("ERROR:  pppd: Returned an unexpected exit status code: 7", .exitStatus(7)),
        ("ERROR:  pppd: terminated by signal: Killed: 9", .signaled("Killed: 9")),
        ("ERROR:  pppd: The PPP negotiation failed, that is, it didn't reach the point where at least one network protocol (e.g. IP) was running.", .exitStatus(10)),
    ])
    func parsesOtherPPPDExitLines(line: String, expected: PPPDExit) {
        #expect(parse([line]) == [.pppdExited(expected)])
    }

    // MARK: pppd stderr

    @Test(arguments: [
        ("Modem hangup", PPPDNotice.modemHangup),
        ("Connection terminated.", .connectionTerminated),
        ("LCP terminated by peer", .terminatedByPeer(protocolName: "LCP", reason: nil)),
        ("LCP terminated by peer (Session timeout)", .terminatedByPeer(protocolName: "LCP", reason: "Session timeout")),
        ("IPCP terminated by peer", .terminatedByPeer(protocolName: "IPCP", reason: nil)),
        ("Terminating on signal 15.", .terminatingOnSignal(15)),
        ("Terminating on signal 2", .terminatingOnSignal(2)),
        ("Hangup (SIGHUP)", .hangup),
        ("Serial link appears to be disconnected.", .serialLinkDisconnected),
    ])
    func recognisesPPPDNotices(message: String, expected: PPPDNotice) {
        for line in [message, "Fri Apr  5 11:31:24 2024 : \(message)", "Thu Oct 15 12:12:05 2026 : \(message)"] {
            #expect(parse([line], source: .stderr) == [.pppdNotice(expected)], "\(line)")
        }
    }

    @Test(arguments: [
        "Thu Oct 15 09:12:04 2026 : Using interface ppp0",
        "Thu Oct 15 09:12:04 2026 : publish_entry SCDSet() failed: Success!",
        "Thu Oct 15 09:12:04 2026 : local  IP address 10.212.134.200",
        "Fri Apr  5 11:31:24 2024 : ",
        "Fri Apr  5 11:31:24 2024 :",
        "Fri Apr  5 11:31:24 2024 : Modem hangup again",
        "route: writing to routing socket: not in table",
        "execv: No such file or directory",
        " terminated by peer",
        "LCP terminated by peer: bye",
        "Terminating on signal fifteen.",
    ])
    func ignoresOtherStderrLines(line: String) {
        #expect(parse([line], source: .stderr).isEmpty)
    }

    @Test func doesNotReadPPPDNoticesFromStdout() {
        #expect(parse(["Modem hangup", "Connection terminated."], source: .stdout).isEmpty)
    }

    @Test(arguments: [
        ("Fri Apr  5 11:31:24 2024 : Modem hangup", "Modem hangup"),
        ("Thu Oct 15 09:12:04 2026 : x", "x"),
        ("Fri Apr 5 11:31:24 2024 : Modem hangup", "Fri Apr 5 11:31:24 2024 : Modem hangup"),
        ("Modem hangup", "Modem hangup"),
    ])
    func stripsOnlyACLocaleTimestamp(line: String, expected: String) {
        #expect(OpenFortiVPNOutputParser.stripPPPDTimestamp(Substring(line)) == expected)
    }

    // MARK: Raw tunnel dump

    @Test func acceptsOnlyTeardownSignalsAfterARawTunnelDump() {
        // io.c:473-474 and 421-429: up to 250 raw bytes, newlines included, follow on stdout.
        var parser = OpenFortiVPNOutputParser()
        let setup = ["INFO:   Connected to gateway.", "INFO:   Authenticated.", "INFO:   Setting new routes...", "INFO:   Tunnel is up and running."]
        #expect(setup.flatMap { parser.parse($0, source: .stdout) } == [.connectedToGateway, .authenticated, .routesTouched, .tunnelUp])
        let dump = [
            "ERROR:  Received bad header from gateway:",
            "  (hex) 48 54 54 50 2f 31",
            "  (raw) HTTP/1.1 200 OK",
            "ERROR:      trusted-cert = \(Self.otherDigest)",
            "ERROR:  Could not authenticate to gateway. Please check the password, client certificate, etc.",
            "INFO:   Got addresses: [10.9.9.9], ns [10.9.9.53, 0.0.0.0]",
            "INFO:   Authenticated.",
            "INFO:   Tunnel is up and running.",
            "INFO:   Setting new routes...",
        ]
        #expect(dump.flatMap { parser.parse($0, source: .stdout) }.isEmpty)
        #expect(parser.rejectedCertificateDigests.isEmpty)
        #expect(parser.tunnelAddresses == nil)
        let teardown = [
            "INFO:   Cancelling threads...",
            "INFO:   Setting ppp0 interface down.",
            "INFO:   Restoring routes...",
            "INFO:   Removing VPN nameservers...",
            "ERROR:  pppd: The link was terminated by the modem hanging up.",
        ]
        #expect(teardown.flatMap { parser.parse($0, source: .stdout) } == [
            .interfaceDown(name: "ppp0"), .routesRestored, .nameserversRemoved, .pppdExited(.exitStatus(16)),
        ])
        #expect(parser.routesTouched && parser.routesRestored)
    }
}
