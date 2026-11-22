import Foundation

@MainActor
final class UpdateWaiter {
  private var process: Process?

  static let script = """
    while kill -0 "$1" 2>/dev/null; do sleep 1; done
    [ -e "$2" ] || exit 0
    until [ -e "$3" ]; do sleep 1; done
    shift 3
    exec "$@"
    """

  static func arguments(appPID: pid_t, installId: UInt64) -> [String] {
    let record = "\(UpdateRecords.directory)/\(installId)"
    return [
      "/bin/sh", "-c", script, "sh", String(appPID),
      record + UpdateRecords.startedSuffix, record + UpdateRecords.resultSuffix,
      "/usr/bin/open", "/Applications/FortiVPNManager.app",
    ]
  }

  func start(installId: UInt64) -> AppInternal.PortResponse {
    let arguments = Self.arguments(appPID: getpid(), installId: installId)
    let process = Process()
    process.executableURL = URL(filePath: arguments[0])
    process.arguments = Array(arguments.dropFirst())
    process.standardInput = FileHandle.nullDevice
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    do {
      try process.run()
    } catch {
      return .updateWaiterFailed(description: error.localizedDescription)
    }
    self.process = process
    return .updateWaiterStarted
  }

  func stop() {
    guard let process else { return }
    self.process = nil
    process.terminate()
    process.waitUntilExit()
  }
}
