import Darwin
import Dispatch
import FortiHelperCore
import FortiShared
import Foundation
import Synchronization
import Testing

@Suite struct ProcessRunnerTests {
    /// Spawns `/bin/sh -c script`.
    private func spawnShell(_ script: String, environment: [String: String] = fixturePATH) throws -> SpawnedProcess {
        try ProcessRunner.spawn(executable: "/bin/sh", arguments: ["/bin/sh", "-c", script], environment: environment)
    }

    /// Reads `fd` to EOF and closes it.
    private func readAll(_ fd: Int32) -> String {
        var data = [UInt8]()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = read(fd, &buffer, buffer.count)
            if count > 0 {
                data += buffer[..<count]
            } else if count < 0 && errno == EINTR {
                continue
            } else {
                break
            }
        }
        close(fd)
        return String(decoding: data, as: UTF8.self)
    }

    private func closeAll(_ process: SpawnedProcess) {
        close(process.stdinWrite)
        close(process.stdoutRead)
        close(process.stderrRead)
    }

    @Test func pipesAreConnectedToStandardStreams() throws {
        let process = try spawnShell("echo out; echo err >&2; IFS= read -r line; echo \"got:$line\"; exit 3")
        let input = Array("hello\n".utf8)
        #expect(write(process.stdinWrite, input, input.count) == input.count)
        close(process.stdinWrite)
        #expect(readAll(process.stdoutRead) == "out\ngot:hello\n")
        #expect(readAll(process.stderrRead) == "err\n")
        #expect(ProcessRunner.reap(process.pid) == .exited(code: 3))
    }

    @Test func environmentIsExactlyWhatWasPassed() throws {
        let process = try ProcessRunner.spawn(
            executable: "/usr/bin/env",
            arguments: ["env"],
            environment: ["PATH": ExecutableSpec.helperPATH, "FVM_TEST": "a=b c"]
        )
        close(process.stdinWrite)
        let output = readAll(process.stdoutRead)
        close(process.stderrRead)
        #expect(ProcessRunner.reap(process.pid) == .exited(code: 0))
        #expect(output.split(separator: "\n").sorted() == ["FVM_TEST=a=b c", "PATH=/usr/bin:/bin:/usr/sbin:/sbin"])
    }

    @Test func argv0IsTakenFromArguments() throws {
        let process = try ProcessRunner.spawn(executable: "/bin/sh", arguments: ["custom-argv0", "-c", "echo \"$0\""], environment: [:])
        close(process.stdinWrite)
        #expect(readAll(process.stdoutRead) == "custom-argv0\n")
        close(process.stderrRead)
        #expect(ProcessRunner.reap(process.pid) == .exited(code: 0))
    }

    @Test func onlyStandardDescriptorsAreInherited() throws {
        // A descriptor without FD_CLOEXEC would normally survive exec.
        let original = open("/dev/null", O_RDONLY | O_CLOEXEC)
        let leaked = fcntl(original, F_DUPFD, 40)
        close(original)
        #expect(leaked >= 40)
        defer { close(leaked) }
        #expect(fcntl(leaked, F_GETFD) & FD_CLOEXEC == 0)
        let process = try spawnShell("for fd in 0 1 2 \(leaked); do if [ -e /dev/fd/$fd ]; then echo \"$fd open\"; else echo \"$fd closed\"; fi; done")
        close(process.stdinWrite)
        #expect(readAll(process.stdoutRead) == "0 open\n1 open\n2 open\n\(leaked) closed\n")
        close(process.stderrRead)
        #expect(ProcessRunner.reap(process.pid) == .exited(code: 0))
    }

    @Test func parentSideDescriptorsAreCloseOnExec() throws {
        let process = try spawnShell("exit 0")
        for fd in [process.stdinWrite, process.stdoutRead, process.stderrRead] {
            #expect(fd > STDERR_FILENO)
            #expect(fcntl(fd, F_GETFD) & FD_CLOEXEC != 0)
        }
        #expect(ProcessRunner.reap(process.pid) == .exited(code: 0))
        closeAll(process)
    }

    @Test func childLeadsItsOwnProcessGroup() throws {
        let process = try ProcessRunner.spawn(executable: "/bin/sleep", arguments: ["sleep", "5"], environment: [:])
        defer { closeAll(process) }
        #expect(getpgid(process.pid) == process.pid)
        #expect(getpgid(process.pid) != getpgrp())
        kill(process.pid, SIGKILL)
        #expect(ProcessRunner.reap(process.pid) == .signaled(signal: SIGKILL))
    }

    /// A child spawned from a dispatch worker thread that blocks SIGTERM, by a process that
    /// ignores SIGTERM, still gets the default disposition and an empty mask (SETSIGDEF/SETSIGMASK).
    @Test func childSignalStateIsReset() async throws {
        let result = Mutex<Result<SpawnedProcess, SpawnError>?>(nil)
        await withSIGTERMIgnored {
            await onBackgroundQueueBlockingSIGTERM {
                let spawned = Result { () throws(SpawnError) -> SpawnedProcess in
                    try ProcessRunner.spawn(executable: "/bin/sh", arguments: ["/bin/sh", "-c", "kill -TERM $$; echo survived"], environment: fixturePATH)
                }
                result.withLock { $0 = spawned }
            }
        }
        let process = try #require(result.withLock { $0 }).get()
        close(process.stdinWrite)
        #expect(readAll(process.stdoutRead) == "")
        close(process.stderrRead)
        #expect(ProcessRunner.reap(process.pid) == .signaled(signal: SIGTERM))
    }

    @Test func writingToAClosedStdinFailsWithEPIPEInsteadOfSIGPIPE() throws {
        let process = try spawnShell("exit 0")
        #expect(ProcessRunner.reap(process.pid) == .exited(code: 0))
        let bytes = Array("late\n".utf8)
        let result = write(process.stdinWrite, bytes, bytes.count)
        let error = errno
        #expect(result == -1)
        #expect(error == EPIPE)
        closeAll(process)
    }

    @Test(arguments: [
        ("/nonexistent/openfortivpn", ["openfortivpn"], [String: String](), ENOENT),
        ("/bin/sh", [], [:], EINVAL),
        ("/bin/sh", ["sh", "a\0b"], [:], EINVAL),
        ("/bin/sh", ["sh"], ["A=B": "x"], EINVAL),
        ("/bin/sh", ["sh"], ["": "x"], EINVAL),
        ("/bin/sh", ["sh"], ["PATH": "x\0y"], EINVAL),
        ("/dev/null", ["null"], [:], EACCES),
    ])
    func spawnFailures(executable: String, arguments: [String], environment: [String: String], expected: Int32) {
        #expect(throws: SpawnError(code: expected)) {
            try ProcessRunner.spawn(executable: executable, arguments: arguments, environment: environment)
        }
        #expect(SpawnError(code: expected).exitStatus == .spawnFailed(errno: expected))
    }

    @Test func monitorExitReportsStatusAfterBeforeReap() async throws {
        let process = try spawnShell("exit 7")
        defer { closeAll(process) }
        let queue = DispatchQueue(label: "test.monitor")
        let order = Mutex<[String]>([])
        let source = ProcessRunner.monitorExit(
            of: process.pid,
            on: queue,
            beforeReap: {
                // Not reaped yet: the zombie still exists.
                order.withLock { $0.append("beforeReap:\(kill(process.pid, 0) == 0)") }
            },
            handler: { status in order.withLock { $0.append("\(status)") } }
        )
        defer { source.cancel() }
        let deadline = ContinuousClock.now + .seconds(5)
        while order.withLock({ $0.count }) < 2, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(5))
        }
        #expect(order.withLock { $0 } == ["beforeReap:true", "\(ProcessExitStatus.exited(code: 7))"])
    }

    @Test func monitorExitReportsAChildThatAlreadyExited() async throws {
        let process = try spawnShell("exit 5")
        defer { closeAll(process) }
        // Let it become a zombie before watching.
        try await Task.sleep(for: .milliseconds(100))
        let status = Mutex<ProcessExitStatus?>(nil)
        let source = ProcessRunner.monitorExit(of: process.pid, on: DispatchQueue(label: "test.zombie")) { reaped in
            status.withLock { $0 = reaped }
        }
        defer { source.cancel() }
        let deadline = ContinuousClock.now + .seconds(5)
        while status.withLock({ $0 }) == nil, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(5))
        }
        #expect(status.withLock { $0 } == .exited(code: 5))
    }

    @Test(arguments: [
        (Int32(0x0000), ProcessExitStatus.exited(code: 0)),
        (0x0100, .exited(code: 1)),
        (0xFE00, .exited(code: 254)),
        (0xFF00, .exited(code: 255)),
        (0x000F, .signaled(signal: SIGTERM)),
        (0x0009, .signaled(signal: SIGKILL)),
        (0x0086, .signaled(signal: SIGABRT)),  // core dumped
    ])
    func waitStatusDecoding(status: Int32, expected: ProcessExitStatus) {
        #expect(ProcessRunner.exitStatus(fromWaitStatus: status) == expected)
    }

    @Test func reapOfANonChildReportsUnknownExit() {
        #expect(ProcessRunner.reap(1) == .unknown)
    }
}
