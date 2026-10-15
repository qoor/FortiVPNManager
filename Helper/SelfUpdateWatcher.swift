import Darwin
import Dispatch
import Foundation

/// Detects that the helper's own executable was replaced (the app bundle was reinstalled).
///
/// A running helper whose on-disk binary changed can no longer pass the app's code-signing check
/// (the static code no longer matches what is running), so it must exit once idle and let launchd
/// start the new binary on the next XPC request. `TunnelController` decides when exiting is safe.
final class SelfUpdateWatcher: @unchecked Sendable {
    // @unchecked: every stored property is immutable; the timer is only configured in `start`.
    private struct Fingerprint: Equatable {
        let inode: ino_t
        let seconds: Int
        let nanoseconds: Int
    }

    let path: String
    private let original: Fingerprint?
    private let timer: any DispatchSourceTimer

    init() {
        path = Self.executablePath()
        original = Self.fingerprint(of: path)
        timer = DispatchSource.makeTimerSource(queue: DispatchQueue(label: "com.qoor.FortiVPNManager.Helper.selfupdate"))
    }

    /// True when the file at our executable path is no longer the binary we were launched from.
    var isReplaced: Bool {
        guard let original else { return false }
        return Self.fingerprint(of: path) != original
    }

    /// Calls `onReplaced` every 5 s once the binary was replaced. Call once.
    func start(onReplaced: @escaping @Sendable () -> Void) {
        timer.schedule(deadline: .now() + 5, repeating: 5)
        timer.setEventHandler { [weak self] in
            if self?.isReplaced == true {
                onReplaced()
            }
        }
        timer.resume()
    }

    private static func executablePath() -> String {
        var buffer = [CChar](repeating: 0, count: Int(MAXPATHLEN) * 4)
        let length = proc_pidpath(getpid(), &buffer, UInt32(buffer.count))
        guard length > 0 else { return Bundle.main.executablePath ?? CommandLine.arguments[0] }
        return String(decoding: buffer.prefix(Int(length)).map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }

    private static func fingerprint(of path: String) -> Fingerprint? {
        var info = stat()
        guard stat(path, &info) == 0 else { return nil }
        return Fingerprint(inode: info.st_ino, seconds: info.st_mtimespec.tv_sec, nanoseconds: info.st_mtimespec.tv_nsec)
    }
}
