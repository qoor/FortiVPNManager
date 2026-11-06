import Darwin
import Dispatch
import FortiHelperCore
import FortiShared
import os

/// Runs a system tool (`/sbin/route`, `/usr/sbin/networksetup`) synchronously with a fixed argv and
/// `PATH` as its only environment variable. No shell is involved.
enum SystemCommand {
    struct Result: Sendable {
        var status: ProcessExitStatus
        var output: String
        var errorOutput: String

        var succeeded: Bool { status == .exited(code: 0) }
    }

    /// Spawns `executable` (argv[0] is the path itself), waits for it and collects its output. A tool
    /// still running after 10 seconds is killed (SIGKILL), so a hung tool cannot block the caller.
    static func run(_ executable: String, _ arguments: [String]) -> Result {
        let process: SpawnedProcess
        do {
            process = try ProcessRunner.spawn(
                executable: executable,
                arguments: [executable] + arguments,
                environment: ["PATH": ExecutableSpec.helperPATH]
            )
        } catch {
            return Result(status: error.exitStatus, output: "", errorOutput: error.description)
        }
        close(process.stdinWrite)
        let pid = process.pid
        // Set once the child has exited but before it is reaped (its pid cannot be reused until then),
        // so the killer never signals another process.
        let exited = OSAllocatedUnfairLock(initialState: false)
        let killer = DispatchWorkItem {
            exited.withLock { exited in
                if !exited { _ = kill(pid, SIGKILL) }
            }
        }
        DispatchQueue.global().asyncAfter(deadline: .now() + .seconds(10), execute: killer)
        // The tools used here print a few lines at most, far below the pipe buffer, so reading the
        // two pipes one after the other cannot block the child.
        let output = readToEnd(process.stdoutRead)
        let errorOutput = readToEnd(process.stderrRead)
        close(process.stdoutRead)
        close(process.stderrRead)
        // A tool that closed its pipes but still runs is killed while this waits.
        waitForExit(pid)
        exited.withLock { $0 = true }
        killer.cancel()
        return Result(status: ProcessRunner.reap(pid), output: output, errorOutput: errorOutput)
    }

    /// Blocks until `pid` has exited, leaving it unreaped (`WNOWAIT`).
    private static func waitForExit(_ pid: pid_t) {
        var info = siginfo_t()
        while waitid(P_PID, id_t(pid), &info, WEXITED | WNOWAIT) == -1, errno == EINTR {}
    }

    /// Reads `fd` until EOF, keeping at most 64 KiB (the rest is drained and dropped).
    private static func readToEnd(_ fd: Int32) -> String {
        let limit = 64 * 1024
        var data: [UInt8] = []
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = buffer.withUnsafeMutableBytes { read(fd, $0.baseAddress, $0.count) }
            if count > 0 {
                data.append(contentsOf: buffer[..<min(count, max(0, limit - data.count))])
            } else if count < 0 && errno == EINTR {
                continue
            } else {
                break
            }
        }
        return String(decoding: data, as: UTF8.self)
    }
}
