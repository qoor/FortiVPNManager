import Darwin
import Dispatch
import FortiShared
import Foundation

/// Runs a system tool (`/sbin/route`, `/usr/sbin/networksetup`) synchronously with a fixed argv and
/// `PATH` as its only environment variable. No shell is involved.
enum SystemCommand {
    struct Result: Sendable {
        /// `.unknown` when the tool could not be started; `errorOutput` then says why.
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
        process.standardOutput = output
        process.standardError = errorOutput
        do {
            try process.run()
        } catch {
            return Result(status: .unknown, output: "", errorOutput: error.localizedDescription)
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
