import Foundation

/// An IPv4 host or network in CIDR form, as the exception rules use it: written
/// `10.77.161.16` (one host, /32) or `10.77.170.0/24` (a network, no bits set after the prefix).
/// Exception targets (`routes.bypass_vpn`) and the networks where they apply (`routes.networks`)
/// are both lists of these. Encoded as that text; decoding re-validates it.
public struct IPv4Network: Codable, Sendable, Hashable, CustomStringConvertible {
    public enum ParseError: Error, Sendable, Equatable {
        /// Not `a.b.c.d` or `a.b.c.d/n` in plain decimal (octets 0-255 and the prefix without leading zeros).
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

    public static let prefixLengths: ClosedRange<Int> = 8...32
    /// Most entries one list may hold.
    public static let maxCount = 100

    /// The network address, host byte order.
    public let address: UInt32
    public let prefixLength: Int

    /// Parses `a.b.c.d` or `a.b.c.d/n`; `a.b.c.d/32` is the same as `a.b.c.d`. No surrounding spaces.
    public init(parsing text: String) throws(ParseError) {
        let parts = text.split(separator: "/", maxSplits: 1, omittingEmptySubsequences: false)
        guard let address = Self.parseDottedQuad(parts[0]) else { throw .malformed }
        var prefixLength = 32
        if parts.count == 2 {
            guard let value = Self.parseDecimal(parts[1], maxDigits: 2) else { throw .malformed }
            prefixLength = value
        }
        try self.init(address: address, prefixLength: prefixLength)
    }

    /// A route destination as `networksetup` reports it: dotted address and dotted subnet mask.
    public init(address text: String, subnetMask: String) throws(ParseError) {
        let (address, prefixLength) = try Self.parse(address: text, subnetMask: subnetMask)
        try self.init(address: address, prefixLength: prefixLength)
    }

    /// The network an interface address belongs to, e.g. `10.71.161.0/24` for `10.71.161.20` with
    /// `255.255.255.0` (the host bits are cleared instead of rejected).
    public init(enclosing text: String, subnetMask: String) throws(ParseError) {
        let (address, prefixLength) = try Self.parse(address: text, subnetMask: subnetMask)
        try self.init(address: address & Self.mask(prefixLength), prefixLength: prefixLength)
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

    /// Dotted-quad address, e.g. `10.77.170.0`.
    public var addressText: String { Self.dottedQuad(address) }
    /// Dotted subnet mask, e.g. `255.255.255.0`.
    public var subnetMask: String { Self.dottedQuad(Self.mask(prefixLength)) }

    /// `10.77.161.16` for a host, `10.77.170.0/24` for a network.
    public var description: String {
        prefixLength == 32 ? addressText : "\(addressText)/\(prefixLength)"
    }

    /// Whether `address` (host byte order) is inside this network.
    public func contains(_ address: UInt32) -> Bool {
        address & Self.mask(prefixLength) == self.address
    }

    /// Whether all of `other` is inside this network.
    public func contains(_ other: IPv4Network) -> Bool {
        other.prefixLength >= prefixLength && contains(other.address)
    }

    /// Whether the two share any address (one contains the other).
    public func overlaps(_ other: IPv4Network) -> Bool {
        contains(other) || other.contains(self)
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        let text = try container.decode(String.self)
        do {
            try self.init(parsing: text)
        } catch {
            throw DecodingError.dataCorruptedError(in: container, debugDescription: "not an IPv4 host or network: \(text)")
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(description)
    }

    /// `a.b.c.d` with four decimal octets 0-255 and no leading zeros (never shorthand or octal forms).
    public static func parseDottedQuad(_ text: Substring) -> UInt32? {
        let octets = text.split(separator: ".", omittingEmptySubsequences: false)
        guard octets.count == 4 else { return nil }
        var value: UInt32 = 0
        for octet in octets {
            guard let number = parseDecimal(octet, maxDigits: 3), number <= 255 else { return nil }
            value = value << 8 | UInt32(number)
        }
        return value
    }

    public static func dottedQuad(_ value: UInt32) -> String {
        "\(value >> 24).\(value >> 16 & 0xFF).\(value >> 8 & 0xFF).\(value & 0xFF)"
    }

    /// Number of leading ones when `mask` is contiguous (ones, then zeros); `nil` otherwise.
    public static func prefixLength(ofMask mask: UInt32) -> Int? {
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
public struct AdditionalRoute: Codable, Sendable, Hashable {
    public var destination: String
    public var subnetMask: String
    public var gateway: String

    public init(destination: String, subnetMask: String, gateway: String) {
        self.destination = destination
        self.subnetMask = subnetMask
        self.gateway = gateway
    }

    /// `network` routed through `gateway`.
    public init(_ network: IPv4Network, gateway: String) {
        self.init(destination: network.addressText, subnetMask: network.subnetMask, gateway: gateway)
    }

    /// The destination as an exception target, or `nil` when it cannot be one (e.g. a /4 or loopback).
    public var network: IPv4Network? {
        try? IPv4Network(address: destination, subnetMask: subnetMask)
    }
}

/// The primary network service (the one macOS sends default traffic through) and its additional routes.
public struct BypassRoutesSnapshot: Codable, Sendable, Equatable {
    /// SCPreferences service ID; stable across renames.
    public var serviceID: String
    /// `networksetup` name of the service, e.g. `Ethernet` or `Wi-Fi`.
    public var serviceName: String
    /// IPv4 router of the service; exception routes go through it.
    public var router: String
    /// This Mac's IPv4 address on the service and its subnet mask.
    public var address: String
    public var subnetMask: String
    public var routes: [AdditionalRoute]

    public init(
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

    /// The network this Mac is on, e.g. `10.71.161.0/24`.
    public var localNetwork: IPv4Network? {
        try? IPv4Network(enclosing: address, subnetMask: subnetMask)
    }
}

/// `setBypassRoutes`: the complete additional route list for the service in `expected`. The helper
/// writes it only while the primary service, its router and its current routes are still exactly
/// `expected`, and only with routes through that router or routes the service already has.
public struct BypassRoutesRequest: Codable, Sendable, Equatable {
    /// What the app read and planned from.
    public var expected: BypassRoutesSnapshot
    public var routes: [AdditionalRoute]

    public init(expected: BypassRoutesSnapshot, routes: [AdditionalRoute]) {
        self.expected = expected
        self.routes = routes
    }
}

/// Reply of `bypassRoutes` and `setBypassRoutes`.
public enum BypassRoutesReply: Codable, Sendable, Equatable {
    case success(BypassRoutesSnapshot)
    case failure(HelperError)
}
