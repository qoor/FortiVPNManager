import Darwin
import Dispatch
import Foundation

/// Runs a system tool (`/sbin/route`, `/usr/sbin/networksetup`) synchronously with a fixed argv and
/// `PATH` as its only environment variable. No shell is involved.
enum SystemCommand {
    struct Result: Sendable {
        /// `.spawnFailed` when the tool could not be started; `errorOutput` then says why.
        var status: ProcessExitStatus
        var output: String
        var errorOutput: String

        var succeeded: Bool { status == .exited(code: 0) }
    }

    /// Spawns `executable`, waits for it and collects its output. A tool still running after 10
    /// seconds is killed (SIGKILL), so a hung tool cannot block the caller.
    static func run(_ executable: String, _ arguments: [String]) -> Result {
        let process = Process()
        process.executableURL = URL(filePath: executable)
        process.arguments = arguments
        process.environment = ["PATH": "/usr/bin:/bin:/usr/sbin:/sbin"]
        process.standardInput = FileHandle.nullDevice
        let output = Pipe()
        let errorOutput = Pipe()
        // Foundation leaves pipes inheritable; a VPN client the Helper entry spawns meanwhile must
        // not hold them open. The tool still gets its ends, which Process duplicates onto 1 and 2.
        for pipe in [output, errorOutput] {
            for end in [pipe.fileHandleForReading, pipe.fileHandleForWriting] {
                _ = fcntl(end.fileDescriptor, F_SETFD, FD_CLOEXEC)
            }
        }
        process.standardOutput = output
        process.standardError = errorOutput
        do {
            try process.run()
        } catch {
            let error = error as NSError
            // Process reports a missing or non-executable file as a Cocoa error, and a failed
            // posix_spawn with its errno.
            let code = error.domain == NSPOSIXErrorDomain ? Int32(error.code) : ENOENT
            return Result(
                status: .spawnFailed(errno: code),
                output: "",
                errorOutput: "\(String(cString: strerror(code))) (errno \(code))"
            )
        }
        // Process reaps the tool on its own, so this may run just after the reap; XNU hands out pids
        // in sequence, so the pid is not reused within that moment.
        let killer = DispatchWorkItem {
            if process.isRunning { kill(process.processIdentifier, SIGKILL) }
        }
        DispatchQueue.global().asyncAfter(deadline: .now() + .seconds(10), execute: killer)
        // The tools used here print a few lines at most, far below the pipe buffer, so reading the
        // two pipes one after the other cannot block the tool.
        let outputData = output.fileHandleForReading.readDataToEndOfFile()
        let errorOutputData = errorOutput.fileHandleForReading.readDataToEndOfFile()
        // A tool that closed its pipes but still runs is killed while this waits.
        process.waitUntilExit()
        killer.cancel()
        let status: ProcessExitStatus = switch process.terminationReason {
        case .exit: .exited(code: process.terminationStatus)
        case .uncaughtSignal: .signaled(signal: process.terminationStatus)
        @unknown default: .unknown
        }
        return Result(
            status: status,
            output: String(decoding: outputData, as: UTF8.self),
            errorOutput: String(decoding: errorOutputData, as: UTF8.self)
        )
    }
}

/// How a command ended.
enum ProcessExitStatus: Sendable, Equatable {
    case exited(code: Int32)
    case signaled(signal: Int32)
    case spawnFailed(errno: Int32)
    /// The status could not be collected.
    case unknown
}
