@testable import FortiCore
import FortiShared
import Testing

@Suite struct OpenFortiVPNOutputParserTests {
    static let digest = "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11"
    static let otherDigest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

    private func parse(_ lines: [String], source: OutputSource = .stdout) -> [OpenFortiVPNSignal] {
        var parser = OpenFortiVPNOutputParser()
        return lines.compactMap { parser.parse($0, source: source) }
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
        // Password/OTP rejection: only the tunnel-mode variant is a signal.
        "ERROR:  Could not authenticate to gateway. Please check the password, client certificate, etc.",
        "ERROR:  Could not authenticate to gateway.",
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
            TunnelAddresses(ip: "10.212.134.200")
        ),
        (
            "[10.212.134.200], ns [10.0.0.53, 10.0.0.54], ns_suffix [corp.example.com]",
            TunnelAddresses(ip: "10.212.134.200")
        ),
        (
            "[10.1.2.3], ns [10.0.0.53, 0.0.0.0]",
            TunnelAddresses(ip: "10.1.2.3")
        ),
        (
            "[10.1.2.3], ns [0.0.0.0, 0.0.0.0], ns_suffix [a.example;b.example]",
            TunnelAddresses(ip: "10.1.2.3")
        ),
        (
            "[10.1.2.3], ns [0.0.0.0, 0.0.0.0], ns_suffix []",
            TunnelAddresses(ip: "10.1.2.3")
        ),
    ])
    func parsesGotAddresses(payload: String, expected: TunnelAddresses) {
        var parser = OpenFortiVPNOutputParser()
        _ = parser.parse("INFO:   Got addresses: \(payload)", source: .stdout)
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
        #expect(parser.parse("INFO:   Got addresses: \(payload)", source: .stdout) == nil)
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
        let signals = Self.certificateBlock(Self.digest).compactMap { parser.parse($0, source: .stdout) }
        #expect(signals == [.certificateRejected(digest: Self.digest)])
        #expect(parser.rejectedCertificateDigests == [Self.digest])
    }

    @Test func dedupesARepeatedBlock() {
        // The block is printed again when openfortivpn reconnects to log out (tunnel.c:1455).
        var parser = OpenFortiVPNOutputParser()
        let lines = Self.certificateBlock(Self.digest) + ["INFO:   Closed connection to gateway."]
            + Self.certificateBlock(Self.digest) + ["INFO:   Could not log out."]
        let signals = lines.compactMap { parser.parse($0, source: .stdout) }
        #expect(signals == [.certificateRejected(digest: Self.digest)])
        #expect(parser.rejectedCertificateDigests == [Self.digest])
    }

    @Test func keepsDistinctDigestsInOrder() {
        var parser = OpenFortiVPNOutputParser()
        let lines = Self.certificateBlock(Self.otherDigest) + Self.certificateBlock(Self.digest)
            + Self.certificateBlock(Self.otherDigest)
        let signals = lines.compactMap { parser.parse($0, source: .stdout) }
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
        _ = parser.parse("INFO:   Setting new routes...", source: .stdout)
        #expect(parser.routesTouched && !parser.routesRestored)
        let duringRestore = [
            "INFO:   Setting ppp0 interface down.",
            "INFO:   Restoring routes...",
            "delete host 203.0.113.10: gateway 192.168.1.1",
            "DEBUG:  /sbin/route -n add -net 0.0.0.0 -netmask 0.0.0.0 192.168.1.1",
            "WARN:   Could not restore default route (File exists). Already restored?",
            "ERROR:  /sbin/route: No such file or directory.",
        ]
        #expect(duringRestore.compactMap { parser.parse($0, source: .stdout) }.isEmpty)
        #expect(!parser.routesRestored)
        _ = parser.parse("INFO:   Removing VPN nameservers...", source: .stdout)
        #expect(parser.routesTouched && parser.routesRestored)
        #expect(parser.parse("INFO:   Terminated pppd.", source: .stdout) == nil)
    }

    @Test(arguments: [
        "ERROR:  pppd: The link was terminated by the modem hanging up.",
        "INFO:   Terminated pppd.",
        "ERROR:  waitpid: No child processes",
    ])
    func completesRestoreOnTheNextInfoOrErrorLine(line: String) {
        // With set-dns = 0 there is no "Removing VPN nameservers..." line.
        var parser = OpenFortiVPNOutputParser()
        _ = parser.parse("INFO:   Setting new routes...", source: .stdout)
        _ = parser.parse("INFO:   Restoring routes...", source: .stdout)
        #expect(!parser.routesRestored)
        _ = parser.parse(line, source: .stdout)
        #expect(parser.routesRestored)
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
        for line in lines {
            _ = parser.parse(line, source: .stdout)
        }
        #expect(parser.routesTouched && !parser.routesRestored)
    }

    // MARK: Raw tunnel dump

    @Test func tracksOnlyRouteRestorationAfterARawTunnelDump() {
        // io.c:473-474 and 421-429: up to 250 raw bytes, newlines included, follow on stdout.
        var parser = OpenFortiVPNOutputParser()
        let setup = ["INFO:   Connected to gateway.", "INFO:   Authenticated.", "INFO:   Setting new routes...", "INFO:   Tunnel is up and running."]
        #expect(setup.compactMap { parser.parse($0, source: .stdout) } == [.connectedToGateway, .authenticated, .tunnelUp])
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
        #expect(dump.compactMap { parser.parse($0, source: .stdout) }.isEmpty)
        #expect(parser.rejectedCertificateDigests.isEmpty)
        #expect(parser.tunnelAddresses == nil)
        let teardown = [
            "INFO:   Cancelling threads...",
            "INFO:   Setting ppp0 interface down.",
            "INFO:   Restoring routes...",
            "INFO:   Removing VPN nameservers...",
            "ERROR:  pppd: The link was terminated by the modem hanging up.",
        ]
        #expect(teardown.compactMap { parser.parse($0, source: .stdout) }.isEmpty)
        #expect(parser.routesTouched && parser.routesRestored)
    }
}
