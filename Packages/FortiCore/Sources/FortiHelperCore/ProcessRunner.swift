import Darwin
import Dispatch
import FortiShared
import Synchronization

/// A child started by `ProcessRunner.spawn`. The caller owns the three parent-side pipe ends
/// (all `O_CLOEXEC`) and must close them, and must reap the child (`ProcessRunner.monitorExit`).
public struct SpawnedProcess: Sendable, Equatable {
    public let pid: pid_t
    /// Write end of the child's stdin. Blocking, with `F_SETNOSIGPIPE`: writing after the child
    /// closed its stdin fails with `EPIPE` instead of raising `SIGPIPE`.
    public let stdinWrite: Int32
    /// Read end of the child's stdout (blocking).
    public let stdoutRead: Int32
    /// Read end of the child's stderr (blocking).
    public let stderrRead: Int32

    public init(pid: pid_t, stdinWrite: Int32, stdoutRead: Int32, stderrRead: Int32) {
        self.pid = pid
        self.stdinWrite = stdinWrite
        self.stdoutRead = stdoutRead
        self.stderrRead = stderrRead
    }
}

/// `posix_spawn` (or pipe setup) failed; nothing was started.
public struct SpawnError: Error, Sendable, Equatable, CustomStringConvertible {
    /// The `errno` value.
    public let code: Int32

    public init(code: Int32) {
        self.code = code
    }

    public var exitStatus: ProcessExitStatus {
        .spawnFailed(errno: code)
    }

    public var description: String {
        "\(String(cString: strerror(code))) (errno \(code))"
    }
}

/// Starts and reaps child processes for the root helper (DESIGN §5.2 "spawn").
///
/// The child gets a clean signal state regardless of the calling thread (dispatch worker threads
/// block most signals, and ignored dispositions survive `exec`): an empty signal mask and the
/// default action for every catchable signal. Only the three pipes are inherited, as fds 0/1/2
/// (`POSIX_SPAWN_CLOEXEC_DEFAULT`), and the child leads its own process group.
public enum ProcessRunner {
    /// Spawns `executable` with exactly `arguments` as argv (including argv[0]) and exactly
    /// `environment` as its environment.
    public static func spawn(
        executable: String,
        arguments: [String],
        environment: [String: String]
    ) throws(SpawnError) -> SpawnedProcess {
        guard !arguments.isEmpty,
              !executable.utf8.contains(0),
              arguments.allSatisfy({ !$0.utf8.contains(0) }),
              environment.allSatisfy({ key, value in
                  !key.isEmpty && !key.utf8.contains(UInt8(ascii: "=")) && !key.utf8.contains(0) && !value.utf8.contains(0)
              })
        else { throw SpawnError(code: EINVAL) }
        let environmentStrings = environment.sorted { $0.key < $1.key }.map { "\($0.key)=\($0.value)" }

        var opened: [Int32] = []
        func closeOpened() {
            for fd in opened { close(fd) }
        }
        let stdinPipe: (read: Int32, write: Int32)
        let stdoutPipe: (read: Int32, write: Int32)
        let stderrPipe: (read: Int32, write: Int32)
        do {
            stdinPipe = try makePipe(&opened)
            stdoutPipe = try makePipe(&opened)
            stderrPipe = try makePipe(&opened)
        } catch {
            closeOpened()
            throw error
        }
        // Only affects this (parent) end; the child reads from the other end.
        guard fcntl(stdinPipe.write, F_SETNOSIGPIPE, 1) == 0 else {
            let code = errno
            closeOpened()
            throw SpawnError(code: code)
        }

        var pid: pid_t = 0
        let result = withAttributes { attributes in
            withFileActions(stdin: stdinPipe.read, stdout: stdoutPipe.write, stderr: stderrPipe.write) { fileActions in
                withCStringArray(arguments) { argv in
                    withCStringArray(environmentStrings) { envp in
                        posix_spawn(&pid, executable, &fileActions, &attributes, argv, envp)
                    }
                }
            }
        }
        // The child has its own copies (or nothing was started).
        close(stdinPipe.read)
        close(stdoutPipe.write)
        close(stderrPipe.write)
        guard result == 0 else {
            close(stdinPipe.write)
            close(stdoutPipe.read)
            close(stderrPipe.read)
            throw SpawnError(code: result)
        }
        return SpawnedProcess(pid: pid, stdinWrite: stdinPipe.write, stdoutRead: stdoutPipe.read, stderrRead: stderrPipe.read)
    }

    /// Calls `handler` once on `queue` after `pid` (a child of this process) exits, with its reaped
    /// status. `beforeReap` runs on `queue` just before reaping, while the exited child is still a
    /// zombie, so its pid and process-group id cannot have been reused yet.
    ///
    /// Keep the returned source until `handler` ran; cancel it to stop watching (the child is then
    /// not reaped). A child that already exited is reported too (dispatch synthesizes the exit
    /// event for zombies).
    public static func monitorExit(
        of pid: pid_t,
        on queue: DispatchQueue,
        beforeReap: @escaping @Sendable () -> Void = {},
        handler: @escaping @Sendable (ProcessExitStatus) -> Void
    ) -> any DispatchSourceProcess {
        let source = DispatchSource.makeProcessSource(identifier: pid, eventMask: .exit, queue: queue)
        let once = OnceFlag()
        source.setEventHandler {
            guard once.claim() else { return }
            beforeReap()
            handler(reap(pid))
        }
        source.resume()
        return source
    }

    /// Blocks until `pid` can be reaped and returns its status. Only call this once the child has
    /// exited (or is about to). If something else already reaped it, reports `.unknown`.
    public static func reap(_ pid: pid_t) -> ProcessExitStatus {
        var status: Int32 = 0
        while true {
            let result = waitpid(pid, &status, 0)
            if result == pid {
                return exitStatus(fromWaitStatus: status)
            }
            if result == -1 && errno == EINTR {
                continue
            }
            return .unknown
        }
    }

    /// Decodes a `waitpid` status (`WIFEXITED`/`WEXITSTATUS`, `WIFSIGNALED`/`WTERMSIG`; Swift does
    /// not import the macros).
    public static func exitStatus(fromWaitStatus status: Int32) -> ProcessExitStatus {
        let low = status & 0x7F
        if low == 0 {
            return .exited(code: (status >> 8) & 0xFF)
        }
        return .signaled(signal: low)
    }

    // MARK: - Private

    /// Creates a pipe with both ends `FD_CLOEXEC` and above the standard fds, so the `dup2`
    /// file actions can never clobber each other.
    private static func makePipe(_ opened: inout [Int32]) throws(SpawnError) -> (read: Int32, write: Int32) {
        var fds: [Int32] = [-1, -1]
        guard pipe(&fds) == 0 else { throw SpawnError(code: errno) }
        opened.append(contentsOf: fds)
        for index in fds.indices {
            if fds[index] <= STDERR_FILENO {
                let moved = fcntl(fds[index], F_DUPFD_CLOEXEC, STDERR_FILENO + 1)
                guard moved >= 0 else { throw SpawnError(code: errno) }
                close(fds[index])
                opened.removeAll { $0 == fds[index] }
                opened.append(moved)
                fds[index] = moved
            } else if fcntl(fds[index], F_SETFD, FD_CLOEXEC) != 0 {
                throw SpawnError(code: errno)
            }
        }
        return (fds[0], fds[1])
    }

    private static func withAttributes(_ body: (inout posix_spawnattr_t?) -> Int32) -> Int32 {
        var attributes: posix_spawnattr_t?
        var result = posix_spawnattr_init(&attributes)
        guard result == 0 else { return result }
        defer { posix_spawnattr_destroy(&attributes) }

        var emptyMask = sigset_t()
        sigemptyset(&emptyMask)
        var defaultSignals = sigset_t()
        sigemptyset(&defaultSignals)
        for signal in 1..<NSIG where signal != SIGKILL && signal != SIGSTOP {
            sigaddset(&defaultSignals, signal)
        }
        let flags = POSIX_SPAWN_SETSIGMASK | POSIX_SPAWN_SETSIGDEF | POSIX_SPAWN_CLOEXEC_DEFAULT | POSIX_SPAWN_SETPGROUP
        for step in [
            { posix_spawnattr_setsigmask(&attributes, &emptyMask) },
            { posix_spawnattr_setsigdefault(&attributes, &defaultSignals) },
            { posix_spawnattr_setpgroup(&attributes, 0) },
            { posix_spawnattr_setflags(&attributes, Int16(flags)) },
        ] {
            result = step()
            guard result == 0 else { return result }
        }
        return body(&attributes)
    }

    private static func withFileActions(
        stdin: Int32,
        stdout: Int32,
        stderr: Int32,
        _ body: (inout posix_spawn_file_actions_t?) -> Int32
    ) -> Int32 {
        var fileActions: posix_spawn_file_actions_t?
        var result = posix_spawn_file_actions_init(&fileActions)
        guard result == 0 else { return result }
        defer { posix_spawn_file_actions_destroy(&fileActions) }
        for (source, target) in [(stdin, STDIN_FILENO), (stdout, STDOUT_FILENO), (stderr, STDERR_FILENO)] {
            result = posix_spawn_file_actions_adddup2(&fileActions, source, target)
            guard result == 0 else { return result }
        }
        return body(&fileActions)
    }

    /// NULL-terminated C string array valid for the duration of `body`.
    private static func withCStringArray(
        _ strings: [String],
        _ body: (UnsafePointer<UnsafeMutablePointer<CChar>?>) -> Int32
    ) -> Int32 {
        var pointers: [UnsafeMutablePointer<CChar>?] = strings.map { strdup($0) }
        defer { pointers.forEach { free($0) } }
        guard !pointers.contains(where: { $0 == nil }) else { return ENOMEM }
        pointers.append(nil)
        return pointers.withUnsafeBufferPointer { body($0.baseAddress!) }
    }
}

/// One-shot flag for handlers that may be invoked more than once.
private final class OnceFlag: Sendable {
    private let claimed = Mutex(false)

    /// True for the first caller only.
    func claim() -> Bool {
        claimed.withLock { claimed in
            defer { claimed = true }
            return !claimed
        }
    }
}
