import CoreGraphics
import Darwin
import Foundation
import SystemConfiguration

/// Point-in-time facts about the network and the screen, as the system reports them.
nonisolated enum SystemFacts {
    /// The interface the routing table uses for `destination`, from `/sbin/route -n get` (fixed argv,
    /// no shell).
    static func routeInterface(to destination: String, reply: @escaping @Sendable (String?) -> Void) {
        run("/sbin/route", ["-n", "get", destination]) { output in
            reply(output.flatMap(parseRouteInterface))
        }
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

    /// Every address of every interface, from getifaddrs.
    static func interfaceAddresses() -> [AppInternal.InterfaceAddress] {
        var head: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&head) == 0, let first = head else { return [] }
        defer { freeifaddrs(first) }
        var result: [AppInternal.InterfaceAddress] = []
        var cursor: UnsafeMutablePointer<ifaddrs>? = first
        while let entry = cursor {
            cursor = entry.pointee.ifa_next
            guard let rawName = entry.pointee.ifa_name else { continue }
            var ipv4: String?
            if let address = entry.pointee.ifa_addr, address.pointee.sa_family == sa_family_t(AF_INET) {
                ipv4 = address.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { ipv4String($0.pointee.sin_addr) }
            }
            result.append(AppInternal.InterfaceAddress(
                name: String(cString: rawName),
                up: entry.pointee.ifa_flags & UInt32(IFF_UP) != 0,
                ipv4: ipv4
            ))
        }
        return result
    }

    /// The Router of `State:/Network/Global/IPv4`, the primary network's.
    static func primaryRouter() -> String? {
        guard let store = SCDynamicStoreCreate(nil, "FortiVPNManager" as CFString, nil, nil),
              let value = SCDynamicStoreCopyValue(store, "State:/Network/Global/IPv4" as CFString) as? [String: Any],
              let router = value[kSCPropNetIPv4Router as String] as? String,
              !router.isEmpty
        else { return nil }
        return router
    }

    /// Whether the session's screen is locked, and whether the displays sleep. Right after an unlock
    /// the session may still read as locked for a moment.
    static func screenState() -> AppInternal.PortResponse {
        .screenState(locked: isScreenLocked(), displaysAsleep: CGDisplayIsAsleep(CGMainDisplayID()) != 0)
    }

    /// `CGSSessionScreenIsLocked` is only present in the session dictionary while the screen is locked.
    private static func isScreenLocked() -> Bool {
        guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
        return (session["CGSSessionScreenIsLocked"] as? Bool) ?? false
    }

    private static func ipv4String(_ address: in_addr) -> String? {
        var address = address
        var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
        guard inet_ntop(AF_INET, &address, &buffer, socklen_t(buffer.count)) != nil else { return nil }
        return String(decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }

    /// Runs a fixed command with only PATH and hands over its standard output when it exits with 0.
    /// The output is tiny (well below the pipe buffer), so it is read after the process exits.
    private static func run(_ executable: String, _ arguments: [String], reply: @escaping @Sendable (String?) -> Void) {
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
            reply(succeeded ? String(decoding: data, as: UTF8.self) : nil)
        }
        do {
            try process.run()
        } catch {
            reply(nil)
            return
        }
        DispatchQueue.global().asyncAfter(deadline: .now() + 5) { @Sendable in
            if process.isRunning { process.terminate() }
        }
    }
}
