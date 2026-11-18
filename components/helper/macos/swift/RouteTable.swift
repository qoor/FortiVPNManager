import Darwin
import Foundation

/// The routing table: read whole per family with `netstat -rn -f inet` or `-f inet6`, changed one
/// route at a time with `/sbin/route -n` (macOS has no command that changes several at once). route
/// exits 0 even when the kernel refuses a change, so only reading the table again tells what
/// happened. Every address that reaches an argv was checked as a dotted quad or an IPv6 address.
enum RouteTable {
    static let routePath = "/sbin/route"
    static let netstatPath = "/usr/sbin/netstat"

    enum Family: String, Sendable {
        case inet
        case inet6
    }

    /// A route through a gateway, in dotted quads or IPv6 addresses.
    struct Route: Hashable, Sendable {
        let family: Family
        let destination: String
        let gateway: String
        let prefixLength: Int

        /// `nil` unless all three are dotted quads and the mask is contiguous.
        init?(destination: String, subnetMask: String, gateway: String) {
            guard let mask = IPv4Network.parseDottedQuad(Substring(subnetMask)),
                  IPv4Network.parseDottedQuad(Substring(destination)) != nil,
                  mask.nonzeroBitCount == 32 - mask.trailingZeroBitCount
            else { return nil }
            self.init(destination: destination, prefixLength: mask.nonzeroBitCount, gateway: gateway)
        }

        init?(destination: String, prefixLength: Int, gateway: String) {
            if IPv4Network.parseDottedQuad(Substring(destination)) != nil {
                guard (0...32).contains(prefixLength), IPv4Network.parseDottedQuad(Substring(gateway)) != nil else {
                    return nil
                }
                family = .inet
                self.destination = destination
                self.gateway = gateway
            } else {
                guard (0...128).contains(prefixLength),
                      let destination = RouteTable.canonicalIPv6(destination),
                      let gateway = RouteTable.ipv6Gateway(Substring(gateway))
                else { return nil }
                family = .inet6
                self.destination = destination
                self.gateway = gateway
            }
            self.prefixLength = prefixLength
        }

        init?(_ route: BypassRoute) {
            self.init(destination: route.destination, subnetMask: route.subnetMask, gateway: route.gateway)
        }

        private var destinationArguments: [String] {
            switch family {
            case .inet:
                let mask = UInt32.max << (32 - prefixLength)
                let subnetMask = "\(mask >> 24).\(mask >> 16 & 0xff).\(mask >> 8 & 0xff).\(mask & 0xff)"
                return prefixLength == 32
                    ? ["-host", destination]
                    : ["-net", destination, "-netmask", subnetMask]
            case .inet6:
                return prefixLength == 128
                    ? ["-inet6", "-host", destination]
                    : ["-inet6", "-net", "\(destination)/\(prefixLength)"]
            }
        }

        /// After `route -n`.
        var addArguments: [String] { ["add"] + destinationArguments + [gateway] }
        var deleteArguments: [String] { ["delete"] + destinationArguments + [gateway] }
        var changeArguments: [String] { ["change"] + destinationArguments + [gateway] }

        /// Whether `row` is this route as `add` puts it in the table: static, for every interface,
        /// with this destination, prefix and gateway. Not a copy the system made of another route
        /// for one host (W), nor a route bound to one interface (I).
        func matches(_ row: Row) -> Bool {
            row.flags.contains("S") && !row.flags.contains("W") && !row.flags.contains("I")
                && row.destination == destination && row.prefixLength == prefixLength
                && row.gateway == gateway
        }
    }

    /// One route of `netstat -rn`.
    struct Row: Sendable, Equatable {
        /// The network address in a dotted quad or an IPv6 address, `0.0.0.0` or `::` for the
        /// default route.
        var destination: String
        var prefixLength: Int
        /// As printed: an address, `link#N` or a hardware address.
        var gateway: String
        /// `U`p, `G`ateway, `H`ost, `S`tatic, `W`as cloned, `I`nterface scoped, ...
        var flags: Set<Character>
        var interface: String
    }

    /// The table, or `nil` when it cannot be read.
    static func read(_ family: Family) -> [Row]? {
        let result = SystemCommand.run(netstatPath, ["-rn", "-f", family.rawValue])
        guard result.succeeded else {
            HelperLog.routes.error("netstat -rn -f \(family.rawValue, privacy: .public) failed (\(String(describing: result.status), privacy: .public)): \(result.errorOutput, privacy: .public)")
            return nil
        }
        return parse(table: result.output, family: family)
    }

    /// The routes of `netstat -rn -f inet`'s or `-f inet6`'s output, after its header line. netstat
    /// writes an IPv4 network without its trailing zero octets, and without its prefix length when
    /// that is the address class's (`127` is 127.0.0.0/8, `192.168.1` is 192.168.1.0/24); a host
    /// route, flag `H`, as a whole address.
    static func parse(table output: String, family: Family) -> [Row] {
        var rows: [Row] = []
        var header = false
        for line in output.split(whereSeparator: \.isNewline) {
            let fields = line.split(whereSeparator: \.isWhitespace)
            guard header else {
                header = fields.first == "Destination"
                continue
            }
            guard fields.count >= 4 else { continue }
            let flags = Set(fields[2])
            let parsed = switch family {
            case .inet: destination(fields[0], host: flags.contains("H"))
            case .inet6: ipv6Destination(fields[0])
            }
            guard let (destination, prefixLength) = parsed else { continue }
            let gateway = switch family {
            case .inet: String(fields[1])
            case .inet6: ipv6Gateway(fields[1]) ?? String(fields[1])
            }
            rows.append(Row(
                destination: destination, prefixLength: prefixLength, gateway: gateway,
                flags: flags, interface: String(fields[3])
            ))
        }
        return rows
    }

    private static func destination(_ text: Substring, host: Bool) -> (String, Int)? {
        if text == "default" { return ("0.0.0.0", 0) }
        let parts = text.split(separator: "/", maxSplits: 1, omittingEmptySubsequences: false)
        let octets = parts[0].split(separator: ".", omittingEmptySubsequences: false)
        guard (1...4).contains(octets.count),
              octets.allSatisfy({ !$0.isEmpty && $0.allSatisfy(\.isASCII) && UInt8($0) != nil })
        else { return nil }
        let address = (octets.map(String.init) + Array(repeating: "0", count: 4 - octets.count))
            .joined(separator: ".")
        if parts.count == 2 {
            guard let length = Int(parts[1]), (0...32).contains(length) else { return nil }
            return (address, length)
        }
        if host { return octets.count == 4 ? (address, 32) : nil }
        let first = Int(octets[0]) ?? 0
        return (address, first < 128 ? 8 : first < 192 ? 16 : 24)
    }

    private static func ipv6Destination(_ text: Substring) -> (String, Int)? {
        if text == "default" { return ("::", 0) }
        let parts = text.split(separator: "/", maxSplits: 1, omittingEmptySubsequences: false)
        guard let address = canonicalIPv6(parts[0].prefix { $0 != "%" }) else { return nil }
        guard parts.count == 2 else { return (address, 128) }
        guard let length = Int(parts[1]), (0...128).contains(length) else { return nil }
        return (address, length)
    }

    static func ipv6Gateway(_ text: Substring) -> String? {
        let address = text.prefix { $0 != "%" }
        guard let canonical = canonicalIPv6(address) else { return nil }
        guard address.count < text.count else { return canonical }
        let scope = text.dropFirst(address.count + 1)
        guard !scope.isEmpty, scope.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else { return nil }
        return "\(canonical)%\(scope)"
    }

    static func canonicalIPv6(_ text: some StringProtocol) -> String? {
        var address = in6_addr()
        guard inet_pton(AF_INET6, String(text), &address) == 1 else { return nil }
        return withUnsafeTemporaryAllocation(of: CChar.self, capacity: Int(INET6_ADDRSTRLEN)) { buffer in
            inet_ntop(AF_INET6, &address, buffer.baseAddress, socklen_t(buffer.count)).map { String(cString: $0) }
        }
    }

    struct Entry: Sendable {
        var destination: String
        var interface: String?
        /// `UP`, `GATEWAY`, `HOST`, `STATIC`, ...
        var flags: Set<String>
    }

    /// The route `route -n get` reports for `destination`, or `nil` if there is none. A missing
    /// route ("not in table") still exits with status 0, so the output decides.
    static func lookup(_ destination: String) -> Entry? {
        let result = SystemCommand.run(routePath, ["-n", "get", destination])
        guard result.succeeded else { return nil }
        return parse(lookup: result.output)
    }

    @discardableResult
    static func add(_ route: Route) -> Bool {
        change(route.addArguments)
    }

    @discardableResult
    static func add(_ route: Route, replacingCopiesIn table: [Row]) -> Bool {
        // A copy of the default route the system made for this host, for every interface, makes
        // the add fail; the system makes another when it needs one.
        for copy in table where copy.flags.contains("W") && !copy.flags.contains("I")
            && copy.destination == route.destination && copy.prefixLength == route.prefixLength {
            if let copy = Route(destination: copy.destination, prefixLength: route.prefixLength, gateway: copy.gateway) {
                delete(copy)
            }
        }
        return add(route)
    }

    @discardableResult
    static func delete(_ route: Route) -> Bool {
        change(route.deleteArguments)
    }

    /// Runs `route -n <arguments>`. The kernel's refusal shows only as "route: writing to routing
    /// socket: …" on stderr, so stderr output counts as failure. Callers that need certainty read
    /// the table again.
    @discardableResult
    static func change(_ arguments: [String]) -> Bool {
        let result = SystemCommand.run(routePath, ["-n"] + arguments)
        let succeeded = result.succeeded && result.errorOutput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        if !succeeded {
            let command = arguments.joined(separator: " ")
            let message = (result.errorOutput + result.output).trimmingCharacters(in: .whitespacesAndNewlines)
            HelperLog.routes.error("route -n \(command, privacy: .public) failed (\(String(describing: result.status), privacy: .public)): \(message, privacy: .public)")
        }
        return succeeded
    }

    /// Parses the `key: value` lines of `route -n get`.
    static func parse(lookup output: String) -> Entry? {
        var fields: [String: String] = [:]
        for line in output.split(whereSeparator: \.isNewline) {
            guard let colon = line.firstIndex(of: ":") else { continue }
            let key = line[..<colon].trimmingCharacters(in: .whitespaces)
            fields[key] = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
        }
        guard let destination = fields["destination"], !destination.isEmpty else { return nil }
        let flags = fields["flags"].map { value in
            Set(value.trimmingCharacters(in: CharacterSet(charactersIn: "<>")).split(separator: ",").map(String.init))
        } ?? []
        return Entry(destination: destination, interface: fields["interface"], flags: flags)
    }
}
