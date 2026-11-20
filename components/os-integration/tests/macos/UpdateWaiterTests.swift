import Foundation
import Testing

@MainActor
struct UpdateWaiterTests {
    @Test func passesTheValuesAsArgumentsOfTheScript() {
        let record = "/var/run/com.qoor.FortiVPNManager.Helper.update/18446744073709551615"
        #expect(UpdateWaiter.arguments(appPID: 4321, installId: .max) == [
            "/bin/sh", "-c", UpdateWaiter.script, "sh", "4321", record + ".started", record + ".result",
            "/usr/bin/open", "/Applications/FortiVPNManager.app",
        ])
    }

    @Test func exitsWithoutRunningWhenTheInstallDidNotStart() async throws {
        let records = try Records()
        let waiter = try await records.run(appPID: exitedPID())
        try await waitForExit(waiter)
        #expect(!records.ran)
    }

    @Test func runsOnceTheAppExitedAndTheResultIsRecorded() async throws {
        let records = try Records()
        try records.write(".started")
        try records.write(".result")
        let waiter = try await records.run(appPID: exitedPID())
        try await waitForExit(waiter)
        #expect(records.ran)
    }

    @Test func waitsForTheAppToExit() async throws {
        let records = try Records()
        try records.write(".started")
        try records.write(".result")
        let app = try Process.run(URL(filePath: "/bin/sleep"), arguments: ["2"])
        let waiter = try records.run(appPID: app.processIdentifier)
        try await Task.sleep(for: .seconds(1))
        #expect(waiter.isRunning)
        #expect(!records.ran)
        try await waitForExit(waiter)
        #expect(records.ran)
    }

    @Test func waitsForTheResult() async throws {
        let records = try Records()
        try records.write(".started")
        let waiter = try await records.run(appPID: exitedPID())
        try await Task.sleep(for: .seconds(1.5))
        #expect(waiter.isRunning)
        #expect(!records.ran)
        try records.write(".result")
        try await waitForExit(waiter)
        #expect(records.ran)
    }

    private func exitedPID() async throws -> pid_t {
        let process = try Process.run(URL(filePath: "/usr/bin/true"), arguments: [])
        try await waitForExit(process)
        return process.processIdentifier
    }

    private func waitForExit(_ process: Process) async throws {
        for _ in 0..<100 where process.isRunning {
            try await Task.sleep(for: .milliseconds(100))
        }
        #expect(!process.isRunning)
    }
}

@MainActor
private final class Records {
    let folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
    private var record: URL { folder.appending(path: "7") }
    private var marker: URL { folder.appending(path: "ran") }

    init() throws {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    }

    deinit {
        try? FileManager.default.removeItem(at: folder)
    }

    func write(_ suffix: String) throws {
        try Data().write(to: URL(filePath: record.path + suffix))
    }

    var ran: Bool {
        FileManager.default.fileExists(atPath: marker.path)
    }

    func run(appPID: pid_t) throws -> Process {
        try Process.run(URL(filePath: "/bin/sh"), arguments: [
            "-c", UpdateWaiter.script, "sh", String(appPID), record.path + ".started", record.path + ".result",
            "/usr/bin/touch", marker.path,
        ])
    }
}
