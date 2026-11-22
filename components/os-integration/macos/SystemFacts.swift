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
    guard unsafe getifaddrs(&head) == 0, let first = unsafe head else { return [] }
    defer { unsafe freeifaddrs(first) }
    var result: [AppInternal.InterfaceAddress] = []
    var cursor: UnsafeMutablePointer<ifaddrs>? = unsafe first
    while let entry = unsafe cursor {
      unsafe cursor = unsafe entry.pointee.ifa_next
      guard let rawName = unsafe entry.pointee.ifa_name else { continue }
      var ipv4: String?
      if let address = unsafe entry.pointee.ifa_addr,
        unsafe address.pointee.sa_family == sa_family_t(AF_INET)
      {
        ipv4 = unsafe address.withMemoryRebound(to: sockaddr_in.self, capacity: 1) {
          unsafe ipv4String($0.pointee.sin_addr)
        }
      }
      unsafe result.append(
        AppInternal.InterfaceAddress(
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
      let value = SCDynamicStoreCopyValue(store, "State:/Network/Global/IPv4" as CFString)
        as? [String: Any],
      let router = value[kSCPropNetIPv4Router as String] as? String,
      !router.isEmpty
    else { return nil }
    return router
  }

  /// Whether the session's screen is locked, and whether the displays sleep. Right after an unlock
  /// the session may still read as locked for a moment.
  static func screenState() -> AppInternal.PortResponse {
    .screenState(
      locked: isScreenLocked(), displaysAsleep: CGDisplayIsAsleep(CGMainDisplayID()) != 0)
  }

  /// `CGSSessionScreenIsLocked` is only present in the session dictionary while the screen is locked.
  private static func isScreenLocked() -> Bool {
    guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
    return (session["CGSSessionScreenIsLocked"] as? Bool) ?? false
  }

  private static func ipv4String(_ address: in_addr) -> String {
    let value = UInt32(bigEndian: address.s_addr)
    return [24, 16, 8, 0].map { String((value >> $0) & 0xFF) }.joined(separator: ".")
  }

  /// Runs a fixed command with only PATH and hands over its standard output when it exits with 0.
  /// The output is tiny (well below the pipe buffer), so it is read after the process exits.
  private static func run(
    _ executable: String, _ arguments: [String], reply: @escaping @Sendable (String?) -> Void
  ) {
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
