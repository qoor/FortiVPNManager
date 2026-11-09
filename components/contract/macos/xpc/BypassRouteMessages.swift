import Foundation

/// An IPv4 host or network that can be an exception target. The Helper writes exception routes only
/// to such destinations; the exception rules themselves are planned by the app.
struct IPv4Network: Sendable, Hashable {
    enum ParseError: Error, Sendable, Equatable {
        /// Not dotted quads in plain decimal (octets 0-255 without leading zeros).
        case malformed
        /// The prefix length is outside ``IPv4Network/prefixLengths``.
        case prefixOutOfRange(Int)
        /// The subnet mask is not a run of ones followed by zeros.
        case nonContiguousMask
        /// 0.0.0.0/8, loopback (127.0.0.0/8), or multicast, reserved and broadcast (224.0.0.0 and above).
        case unroutable
        /// Bits are set after the prefix; `network` is the same network with them cleared.
        case hostBitsSet(network: IPv4Network)
    }

    static let prefixLengths: ClosedRange<Int> = 8...32
    /// Most entries one list may hold.
    static let maxCount = 100

    /// The network address, host byte order.
    let address: UInt32
    let prefixLength: Int

    /// A route destination as `networksetup` reports it: dotted address and dotted subnet mask.
    init(address text: String, subnetMask: String) throws(ParseError) {
        let (address, prefixLength) = try Self.parse(address: text, subnetMask: subnetMask)
        try self.init(address: address, prefixLength: prefixLength)
    }

    private init(address: UInt32, prefixLength: Int) throws(ParseError) {
        guard Self.prefixLengths.contains(prefixLength) else { throw .prefixOutOfRange(prefixLength) }
        let firstOctet = address >> 24
        guard firstOctet != 0, firstOctet != 127, firstOctet < 224 else { throw .unroutable }
        let network = address & Self.mask(prefixLength)
        guard network == address else {
            throw .hostBitsSet(network: IPv4Network(validNetwork: network, prefixLength: prefixLength))
        }
        self.address = address
        self.prefixLength = prefixLength
    }

    private init(validNetwork: UInt32, prefixLength: Int) {
        address = validNetwork
        self.prefixLength = prefixLength
    }

    /// `a.b.c.d` with four decimal octets 0-255 and no leading zeros (never shorthand or octal forms).
    static func parseDottedQuad(_ text: Substring) -> UInt32? {
        let octets = text.split(separator: ".", omittingEmptySubsequences: false)
        guard octets.count == 4 else { return nil }
        var value: UInt32 = 0
        for octet in octets {
            guard let number = parseDecimal(octet, maxDigits: 3), number <= 255 else { return nil }
            value = value << 8 | UInt32(number)
        }
        return value
    }

    /// Number of leading ones when `mask` is contiguous (ones, then zeros); `nil` otherwise.
    private static func prefixLength(ofMask mask: UInt32) -> Int? {
        let length = (~mask).leadingZeroBitCount
        return Self.mask(length) == mask ? length : nil
    }

    private static func parse(address text: String, subnetMask: String) throws(ParseError) -> (UInt32, Int) {
        guard let address = parseDottedQuad(Substring(text)),
              let mask = parseDottedQuad(Substring(subnetMask))
        else { throw .malformed }
        guard let prefixLength = prefixLength(ofMask: mask) else { throw .nonContiguousMask }
        return (address, prefixLength)
    }

    private static func mask(_ prefixLength: Int) -> UInt32 {
        prefixLength == 0 ? 0 : UInt32.max << (32 - prefixLength)
    }

    /// ASCII digits only, `0` or without a leading zero.
    private static func parseDecimal(_ text: Substring, maxDigits: Int) -> Int? {
        let bytes = Array(text.utf8)
        guard (1...maxDigits).contains(bytes.count), bytes.allSatisfy({ (0x30...0x39).contains($0) }),
              bytes.count == 1 || bytes[0] != 0x30
        else { return nil }
        return bytes.reduce(0) { $0 * 10 + Int($1 - 0x30) }
    }
}

/// One additional route of a network service, as `networksetup -getadditionalroutes` prints it.
struct AdditionalRoute: Codable, Sendable, Hashable {
    var destination: String
    var subnetMask: String
    var gateway: String

    init(destination: String, subnetMask: String, gateway: String) {
        self.destination = destination
        self.subnetMask = subnetMask
        self.gateway = gateway
    }

    /// The destination as an exception target, or `nil` when it cannot be one (e.g. a /4 or loopback).
    var network: IPv4Network? {
        try? IPv4Network(address: destination, subnetMask: subnetMask)
    }
}

/// The primary network service (the one macOS sends default traffic through) and its additional routes.
struct BypassRoutesSnapshot: Codable, Sendable, Equatable {
    /// SCPreferences service ID; stable across renames.
    var serviceID: String
    /// `networksetup` name of the service, e.g. `Ethernet` or `Wi-Fi`.
    var serviceName: String
    /// IPv4 router of the service; exception routes go through it.
    var router: String
    /// This Mac's IPv4 address on the service and its subnet mask.
    var address: String
    var subnetMask: String
    var routes: [AdditionalRoute]

    init(
        serviceID: String, serviceName: String, router: String, address: String, subnetMask: String,
        routes: [AdditionalRoute]
    ) {
        self.serviceID = serviceID
        self.serviceName = serviceName
        self.router = router
        self.address = address
        self.subnetMask = subnetMask
        self.routes = routes
    }
}

/// `setBypassRoutes`: the complete additional route list for the service in `expected`. The helper
/// writes it only while the primary service, its router and its current routes are still exactly
/// `expected`, and only with routes through that router or routes the service already has.
struct BypassRoutesRequest: Codable, Sendable, Equatable {
    /// What the app read and planned from.
    var expected: BypassRoutesSnapshot
    var routes: [AdditionalRoute]

    init(expected: BypassRoutesSnapshot, routes: [AdditionalRoute]) {
        self.expected = expected
        self.routes = routes
    }
}

/// Reply of `bypassRoutes` and `setBypassRoutes`.
enum BypassRoutesReply: Codable, Sendable, Equatable {
    case success(BypassRoutesSnapshot)
    case failure(HelperError)
}
