import Foundation

struct TunnelInterface: Equatable {
    static let ifconfigPath = "/sbin/ifconfig"
    static let defaultMTU = 1412

    struct IPv4Address: Equatable {
        var address: String
        var destination: String?
    }

    var up: Bool
    var mtu: Int?
    var ipv4: [IPv4Address]
    var ipv6: [HelperInternal.IPNetwork]

    static func read(_ name: String) -> TunnelInterface? {
        let result = SystemCommand.run(ifconfigPath, [name])
        guard result.succeeded else {
            HelperLog.routes.error("ifconfig \(name, privacy: .public) failed (\(String(describing: result.status), privacy: .public)): \(result.errorOutput, privacy: .public)")
            return nil
        }
        return parse(result.output)
    }

    static func owners(of address: String) -> [String]? {
        var wanted = in_addr()
        guard inet_pton(AF_INET, address, &wanted) == 1 else {
            fatalError("the Helper entry sent \(address) as the tunnel's IPv4 address")
        }
        var list: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&list) == 0 else {
            HelperLog.routes.error("getifaddrs failed: \(String(cString: strerror(errno)), privacy: .public)")
            return nil
        }
        defer { freeifaddrs(list) }
        var owners: [String] = []
        var cursor = list
        while let entry = cursor {
            cursor = entry.pointee.ifa_next
            guard let socket = entry.pointee.ifa_addr, socket.pointee.sa_family == sa_family_t(AF_INET) else {
                continue
            }
            let found = socket.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { $0.pointee.sin_addr.s_addr }
            let name = String(cString: entry.pointee.ifa_name)
            if found == wanted.s_addr, !owners.contains(name) {
                owners.append(name)
            }
        }
        return owners
    }

    static func parse(_ output: String) -> TunnelInterface? {
        let lines = output.split(whereSeparator: \.isNewline).map { $0.split(whereSeparator: \.isWhitespace) }
        guard let first = lines.first, first.count >= 2, first[1].hasPrefix("flags=") else { return nil }
        let value = { (fields: [Substring], key: Substring) -> Substring? in
            fields.firstIndex(of: key).flatMap { $0 + 1 < fields.count ? fields[$0 + 1] : nil }
        }
        let flags = first[1].drop(while: { $0 != "<" }).dropFirst().prefix(while: { $0 != ">" }).split(separator: ",")
        var interface = TunnelInterface(
            up: flags.contains("UP"), mtu: value(first, "mtu").flatMap { Int($0) }, ipv4: [], ipv6: []
        )
        for fields in lines.dropFirst() where fields.count >= 2 {
            switch fields[0] {
            case "inet":
                interface.ipv4.append(IPv4Address(
                    address: String(fields[1]), destination: value(fields, "-->").map(String.init)
                ))
            case "inet6":
                guard let address = RouteTable.canonicalIPv6(fields[1].prefix { $0 != "%" }),
                      let prefixLength = value(fields, "prefixlen").flatMap({ Int($0) })
                else { continue }
                interface.ipv6.append(HelperInternal.IPNetwork(address: address, prefixLength: prefixLength))
            default:
                continue
            }
        }
        return interface
    }

    @discardableResult
    static func change(_ name: String, _ arguments: [String]) -> Bool {
        let result = SystemCommand.run(ifconfigPath, [name] + arguments)
        if !result.succeeded {
            let command = ([name] + arguments).joined(separator: " ")
            let message = (result.errorOutput + result.output).trimmingCharacters(in: .whitespacesAndNewlines)
            HelperLog.routes.error("ifconfig \(command, privacy: .public) failed (\(String(describing: result.status), privacy: .public)): \(message, privacy: .public)")
        }
        return result.succeeded
    }

    static func changes(
        to settings: HelperInternal.NetworkSettings, from current: TunnelInterface
    ) -> [(step: HelperInternal.NetworkStep, arguments: [String])] {
        let ipv4 = ipv4Address(settings)
        var changes: [(step: HelperInternal.NetworkStep, arguments: [String])] = []
        if !current.ipv4.contains(IPv4Address(address: ipv4, destination: ipv4)) {
            changes.append((.addresses, ["inet", ipv4, ipv4, "netmask", "255.255.255.255"]))
        }
        if let ipv6 = ipv6Network(settings), !current.ipv6.contains(ipv6) {
            changes.append((.addresses, ["inet6", "\(ipv6.address)/\(ipv6.prefixLength)"]))
        }
        if !current.up {
            changes.append((.addresses, ["up"]))
        }
        if current.mtu != mtu(settings) {
            changes.append((.mtu, ["mtu", String(mtu(settings))]))
        }
        return changes
    }

    static func removals(
        from previous: HelperInternal.NetworkSettings, to settings: HelperInternal.NetworkSettings
    ) -> [[String]] {
        var removals: [[String]] = []
        let ipv4 = ipv4Address(previous)
        if ipv4 != ipv4Address(settings) {
            removals.append(["inet", ipv4, "delete"])
        }
        if let ipv6 = ipv6Network(previous), ipv6 != ipv6Network(settings) {
            removals.append(["inet6", ipv6.address, "delete"])
        }
        return removals
    }

    func mismatch(
        with settings: HelperInternal.NetworkSettings, previous: HelperInternal.NetworkSettings?
    ) -> HelperInternal.NetworkStep? {
        if let previous {
            let oldAddress = Self.ipv4Address(previous)
            if oldAddress != Self.ipv4Address(settings), ipv4.contains(where: { $0.address == oldAddress }) {
                return .addresses
            }
            if let oldNetwork = Self.ipv6Network(previous), oldNetwork != Self.ipv6Network(settings),
               ipv6.contains(oldNetwork) {
                return .addresses
            }
        }
        return Self.changes(to: settings, from: self).first?.step
    }

    private static func mtu(_ settings: HelperInternal.NetworkSettings) -> Int {
        settings.mtu.map(Int.init) ?? defaultMTU
    }

    private static func ipv4Address(_ settings: HelperInternal.NetworkSettings) -> String {
        let address = settings.ipv4.address.address
        guard IPv4Network.parseDottedQuad(Substring(address)) != nil else {
            fatalError("the Helper entry sent \(address) as the tunnel's IPv4 address")
        }
        return address
    }

    private static func ipv6Network(_ settings: HelperInternal.NetworkSettings) -> HelperInternal.IPNetwork? {
        settings.ipv6.map { family in
            guard let address = RouteTable.canonicalIPv6(family.address.address) else {
                fatalError("the Helper entry sent \(family.address.address) as the tunnel's IPv6 address")
            }
            return HelperInternal.IPNetwork(address: address, prefixLength: family.address.prefixLength)
        }
    }
}
