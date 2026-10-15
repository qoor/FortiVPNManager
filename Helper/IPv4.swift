import Darwin
import Synchronization

/// Dotted-quad IPv4 helpers.
enum IPv4 {
    /// True for a dotted-quad address `inet_pton` accepts (never an option-like or shorthand string).
    static func isAddress(_ text: String) -> Bool {
        var address = in_addr()
        return inet_pton(AF_INET, text, &address) == 1
    }

    /// First IPv4 address of `host` via `getaddrinfo` (blocking; may wait for DNS), or `nil`.
    static func resolve(_ host: String) -> String? {
        var hints = addrinfo()
        hints.ai_family = AF_INET
        hints.ai_socktype = SOCK_STREAM
        var list: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(host, nil, &hints, &list) == 0, let list else { return nil }
        defer { freeaddrinfo(list) }

        var entry: UnsafeMutablePointer<addrinfo>? = list
        while let current = entry {
            if current.pointee.ai_family == AF_INET, let socketAddress = current.pointee.ai_addr {
                var address = socketAddress.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { $0.pointee.sin_addr }
                var text = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
                if inet_ntop(AF_INET, &address, &text, socklen_t(text.count)) != nil {
                    return String(decoding: text.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
                }
            }
            entry = current.pointee.ai_next
        }
        return nil
    }
}

/// The gateway's IPv4 address for one run: resolved in the background at connect time (DNS may
/// not work once the tunnel broke) and read by `RouteRepair` on the supervisor's queue.
final class GatewayAddress: Sendable {
    private let storage = Mutex<String?>(nil)

    var ip: String? {
        storage.withLock { $0 }
    }

    func set(_ ip: String) {
        storage.withLock { $0 = ip }
    }
}
