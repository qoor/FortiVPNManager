import Darwin
import FortiHelperCore
import FortiShared

/// Runs a system tool (`/sbin/route`) synchronously with a fixed argv and `PATH` as its only
/// environment variable. No shell is involved.
enum SystemCommand {
    struct Result: Sendable {
        var status: ProcessExitStatus
        var output: String
        var errorOutput: String

        var succeeded: Bool { status == .exited(code: 0) }
    }

    /// Spawns `executable` (argv[0] is the path itself), waits for it and collects its output.
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
        // The tools used here print a few lines at most, far below the pipe buffer, so reading the
        // two pipes one after the other cannot block the child.
        let output = readToEnd(process.stdoutRead)
        let errorOutput = readToEnd(process.stderrRead)
        close(process.stdoutRead)
        close(process.stderrRead)
        return Result(status: ProcessRunner.reap(process.pid), output: output, errorOutput: errorOutput)
    }

    /// Reads `fd` until EOF, keeping at most `limit` bytes (the rest is drained and dropped).
    private static func readToEnd(_ fd: Int32, limit: Int = 64 * 1024) -> String {
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
