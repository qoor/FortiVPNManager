import Darwin

/// A root process as recorded in the pidfile. The start time and executable path tell it apart
/// from an unrelated process that later reused the pid.
struct ProcessIdentity: Codable, Sendable, Equatable {
    var pid: Int32
    var startSeconds: UInt64
    var startMicroseconds: UInt64
    var path: String

    static let pppdPath = "/usr/sbin/pppd"

    /// The live process `pid`, or `nil` if it is gone, a zombie, unreadable or not running as root.
    static func ofRootProcess(_ pid: pid_t) -> ProcessIdentity? {
        var info = proc_bsdinfo()
        let size = Int32(MemoryLayout<proc_bsdinfo>.size)
        guard pid > 0,
              proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size,
              info.pbi_uid == 0
        else { return nil }

        var buffer = [CChar](repeating: 0, count: Int(MAXPATHLEN) * 4)
        let length = proc_pidpath(pid, &buffer, UInt32(buffer.count))
        guard length > 0 else { return nil }
        let path = String(decoding: buffer.prefix(Int(length)).map { UInt8(bitPattern: $0) }, as: UTF8.self)
        return ProcessIdentity(pid: pid, startSeconds: info.pbi_start_tvsec, startMicroseconds: info.pbi_start_tvusec, path: path)
    }

    /// Root children of `parent` whose executable is `path` (pppd under openfortivpn).
    static func children(of parent: pid_t, runningAt path: String) -> [ProcessIdentity] {
        var pids = [pid_t](repeating: 0, count: 64)
        let count = pids.withUnsafeMutableBytes { proc_listchildpids(parent, $0.baseAddress, Int32($0.count)) }
        guard count > 0 else { return [] }
        return pids.prefix(min(Int(count), pids.count))
            .filter { $0 > 0 }
            .compactMap(ofRootProcess)
            .filter { $0.path == path }
    }

    /// Whether the recorded process is still running: same pid, start time and path, uid 0.
    var isRunning: Bool {
        Self.ofRootProcess(pid) == self
    }
}
