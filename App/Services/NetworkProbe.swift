import Darwin
import Foundation
import SystemConfiguration

/// Point-in-time network facts for `SystemMonitor` (DESIGN §5.5, §6.3). No state, safe off the main actor.
nonisolated enum NetworkProbe {
    /// `true` for interfaces that carry tunnels rather than the physical underlay.
    static func isTunnelInterface(_ name: String) -> Bool {
        name.hasPrefix("ppp") || name.hasPrefix("utun")
    }

    static func isIPv4Literal(_ text: String) -> Bool {
        var address = in_addr()
        return inet_pton(AF_INET, text, &address) == 1
    }

    /// The first IPv4 address of `host`, or `host` itself when it is an IPv4 literal.
    static func resolveIPv4(_ host: String) async -> String? {
        if isIPv4Literal(host) { return host }
        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .utility).async { @Sendable in
                continuation.resume(returning: lookUpIPv4(host))
            }
        }
    }

    /// The interface the kernel uses to reach `ip`, from `/sbin/route -n get <ip>` (fixed argv, no shell).
    static func routeInterface(to ip: String) async -> String? {
        guard isIPv4Literal(ip), let output = await run("/sbin/route", ["-n", "get", ip]) else { return nil }
        return parseRouteInterface(output)
    }

    /// The `interface:` field of `route -n get` output.
    static func parseRouteInterface(_ output: String) -> String? {
        for line in output.split(separator: "\n") {
            let field = line.trimmingCharacters(in: .whitespaces)
            guard field.hasPrefix("interface:") else { continue }
            let name = field.dropFirst("interface:".count).trimmingCharacters(in: .whitespaces)
            return name.isEmpty ? nil : name
        }
        return nil
    }

    static func firstIPv4Address(of interface: String) -> String? {
        interfaces().first { $0.name == interface && $0.ipv4 != nil }?.ipv4
    }

    /// Whether a `ppp*` interface exists and is UP.
    static func isPPPInterfaceUp() -> Bool {
        interfaces().contains { $0.name.hasPrefix("ppp") && $0.flags & UInt32(IFF_UP) != 0 }
    }

    /// Whether `State:/Network/Global/IPv4` names a Router (the primary network is usable).
    static func hasIPv4Router() -> Bool {
        guard let store = SCDynamicStoreCreate(nil, "FortiVPNManager" as CFString, nil, nil),
              let value = SCDynamicStoreCopyValue(store, "State:/Network/Global/IPv4" as CFString) as? [String: Any],
              let router = value[kSCPropNetIPv4Router as String] as? String
        else { return false }
        return !router.isEmpty
    }

    // MARK: - Internals

    private struct InterfaceAddress {
        var name: String
        var flags: UInt32
        var ipv4: String?
    }

    private static func interfaces() -> [InterfaceAddress] {
        var head: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&head) == 0, let first = head else { return [] }
        defer { freeifaddrs(first) }
        var result: [InterfaceAddress] = []
        var cursor: UnsafeMutablePointer<ifaddrs>? = first
        while let entry = cursor {
            cursor = entry.pointee.ifa_next
            guard let rawName = entry.pointee.ifa_name else { continue }
            var ipv4: String?
            if let address = entry.pointee.ifa_addr, address.pointee.sa_family == sa_family_t(AF_INET) {
                ipv4 = address.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { ipv4String($0.pointee.sin_addr) }
            }
            result.append(InterfaceAddress(name: String(cString: rawName), flags: entry.pointee.ifa_flags, ipv4: ipv4))
        }
        return result
    }

    private static func lookUpIPv4(_ host: String) -> String? {
        var hints = addrinfo()
        hints.ai_family = AF_INET
        hints.ai_socktype = SOCK_STREAM
        var head: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(host, nil, &hints, &head) == 0, let first = head else { return nil }
        defer { freeaddrinfo(first) }
        var cursor: UnsafeMutablePointer<addrinfo>? = first
        while let info = cursor {
            cursor = info.pointee.ai_next
            guard info.pointee.ai_family == AF_INET, let address = info.pointee.ai_addr else { continue }
            if let ip = address.withMemoryRebound(to: sockaddr_in.self, capacity: 1, { ipv4String($0.pointee.sin_addr) }) {
                return ip
            }
        }
        return nil
    }

    private static func ipv4String(_ address: in_addr) -> String? {
        var address = address
        var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
        guard inet_ntop(AF_INET, &address, &buffer, socklen_t(buffer.count)) != nil else { return nil }
        return String(decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }

    /// Runs a fixed command with a minimal environment and returns its stdout on exit status 0.
    /// The output is tiny (well below the pipe buffer), so it is read after the process exits.
    private static func run(_ executable: String, _ arguments: [String], timeout: TimeInterval = 5) async -> String? {
        await withCheckedContinuation { continuation in
            let process = Process()
            let stdout = Pipe()
            process.executableURL = URL(filePath: executable)
            process.arguments = arguments
            process.environment = ["PATH": "/usr/bin:/bin:/usr/sbin:/sbin"]
            process.standardInput = FileHandle.nullDevice
            process.standardOutput = stdout
            process.standardError = FileHandle.nullDevice
            process.terminationHandler = { @Sendable finished in
                let data = stdout.fileHandleForReading.readDataToEndOfFile()
                let succeeded = finished.terminationReason == .exit && finished.terminationStatus == 0
                continuation.resume(returning: succeeded ? String(decoding: data, as: UTF8.self) : nil)
            }
            do {
                try process.run()
            } catch {
                continuation.resume(returning: nil)
                return
            }
            DispatchQueue.global().asyncAfter(deadline: .now() + timeout) { @Sendable in
                if process.isRunning { process.terminate() }
            }
        }
    }
}
