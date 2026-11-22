import Foundation

@MainActor
final class UpdateWaiter {
  private var pid: pid_t?

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
    var attributes: posix_spawnattr_t?
    unsafe posix_spawnattr_init(&attributes)
    defer { unsafe posix_spawnattr_destroy(&attributes) }
    unsafe posix_spawnattr_setflags(
      &attributes, Int16(POSIX_SPAWN_SETSID | POSIX_SPAWN_CLOEXEC_DEFAULT))
    var files: posix_spawn_file_actions_t?
    unsafe posix_spawn_file_actions_init(&files)
    defer { unsafe posix_spawn_file_actions_destroy(&files) }
    unsafe posix_spawn_file_actions_addopen(&files, STDIN_FILENO, "/dev/null", O_RDONLY, 0)
    unsafe posix_spawn_file_actions_addopen(&files, STDOUT_FILENO, "/dev/null", O_WRONLY, 0)
    unsafe posix_spawn_file_actions_addopen(&files, STDERR_FILENO, "/dev/null", O_WRONLY, 0)
    let arguments =
      unsafe Self.arguments(appPID: getpid(), installId: installId).map { unsafe strdup($0) }
      + [nil]
    let environment =
      unsafe ProcessInfo.processInfo.environment.map { unsafe strdup("\($0)=\($1)") } + [nil]
    defer {
      for unsafe pointer in unsafe arguments + environment {
        unsafe free(pointer)
      }
    }
    var pid: pid_t = 0
    let status = unsafe posix_spawn(&pid, "/bin/sh", &files, &attributes, arguments, environment)
    guard status == 0 else {
      return unsafe .updateWaiterFailed(description: String(cString: strerror(status)))
    }
    self.pid = pid
    return .updateWaiterStarted
  }

  func stop() {
    guard let pid else { return }
    self.pid = nil
    kill(pid, SIGTERM)
    waitpid(pid, nil, 0)
  }
}
