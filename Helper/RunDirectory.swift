import Darwin
import Foundation

/// What a restarted helper must clean up if the previous one died during a run (the pidfile).
struct RunRecord: Codable, Sendable, Equatable {
    /// openfortivpn, plus its pppd child(ren) once the tunnel is up.
    var processes: [ProcessIdentity]
    /// For removing the gateway host route (`RouteRepair`).
    var gatewayIP: String?
}

/// `/var/run/com.qoor.FortiVPNManager.Helper` (root, 0700): rendered openfortivpn configs
/// (`config.XXXXXX`, 0600, deleted after each run) and the pidfile `run.json`.
enum RunDirectory {
    static let path = "/var/run/com.qoor.FortiVPNManager.Helper"
    static let recordPath = path + "/" + recordName

    private static let recordName = "run.json"
    private static let configPrefix = "config."

    struct Failure: Error, Sendable, CustomStringConvertible {
        let operation: String
        let code: Int32

        var description: String {
            "\(operation): \(String(cString: strerror(code))) (errno \(code))"
        }
    }

    /// Creates the directory if needed and makes sure it is a real directory owned by us with mode 0700.
    static func prepare() throws(Failure) {
        if mkdir(path, 0o700) != 0 {
            let code = errno
            guard code == EEXIST else { throw Failure(operation: "mkdir \(path)", code: code) }
        }
        var info = stat()
        guard lstat(path, &info) == 0 else { throw Failure(operation: "lstat \(path)", code: errno) }
        guard info.st_mode & S_IFMT == S_IFDIR, info.st_uid == geteuid() else {
            throw Failure(operation: "\(path) is not a directory owned by the helper", code: EPERM)
        }
        if info.st_mode & 0o777 != 0o700, chmod(path, 0o700) != 0 {
            throw Failure(operation: "chmod \(path)", code: errno)
        }
    }

    /// Writes `contents` to a new `config.XXXXXX` file (mode 0600) and returns its path.
    static func writeConfig(_ contents: String) throws(Failure) -> String {
        let (fd, filePath) = try makeTemporaryFile(prefix: configPrefix)
        let code = writeAll(fd, Array(contents.utf8))
        close(fd)
        if let code {
            unlink(filePath)
            throw Failure(operation: "write \(filePath)", code: code)
        }
        return filePath
    }

    /// Atomically replaces the pidfile.
    static func writeRecord(_ record: RunRecord) throws(Failure) {
        let data: Data
        do {
            data = try JSONEncoder().encode(record)
        } catch {
            throw Failure(operation: "encode \(recordName)", code: EINVAL)
        }
        let (fd, temporaryPath) = try makeTemporaryFile(prefix: recordName + ".")
        let code = writeAll(fd, Array(data))
        close(fd)
        if let code {
            unlink(temporaryPath)
            throw Failure(operation: "write \(temporaryPath)", code: code)
        }
        guard rename(temporaryPath, recordPath) == 0 else {
            let code = errno
            unlink(temporaryPath)
            throw Failure(operation: "rename \(temporaryPath)", code: code)
        }
    }

    /// The pidfile, `nil` if there is none. An unreadable pidfile yields an empty record, so the
    /// caller still repairs routes.
    static func readRecord() -> RunRecord? {
        guard let data = FileManager.default.contents(atPath: recordPath) else { return nil }
        do {
            return try JSONDecoder().decode(RunRecord.self, from: data)
        } catch {
            HelperLog.helper.error("ignoring unreadable pidfile \(recordPath, privacy: .public)")
            return RunRecord(processes: [], gatewayIP: nil)
        }
    }

    static func removeRecord() {
        remove(recordPath)
    }

    /// Deletes a file; a missing file is fine.
    static func remove(_ filePath: String) {
        guard unlink(filePath) != 0 else { return }
        let code = errno
        if code != ENOENT {
            HelperLog.helper.error("cannot delete \(filePath, privacy: .public): errno \(code)")
        }
    }

    /// Deletes every config file and pidfile (including half-written ones). Only call this while
    /// no run is active.
    static func removeLeftovers() {
        guard let names = try? FileManager.default.contentsOfDirectory(atPath: path) else { return }
        for name in names where name.hasPrefix(configPrefix) || name.hasPrefix(recordName) {
            remove(path + "/" + name)
        }
    }

    // MARK: - Private

    /// `mkstemp` in the run directory: a new file with mode 0600, opened for writing.
    private static func makeTemporaryFile(prefix: String) throws(Failure) -> (fd: Int32, path: String) {
        var template = Array("\(path)/\(prefix)XXXXXX".utf8CString)
        let fd = template.withUnsafeMutableBufferPointer { buffer in
            buffer.baseAddress.map { mkstemp($0) } ?? -1
        }
        guard fd >= 0 else { throw Failure(operation: "mkstemp in \(path)", code: errno) }
        let filePath = String(decoding: template.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
        return (fd, filePath)
    }

    /// Writes all of `bytes`; returns `errno` on failure.
    private static func writeAll(_ fd: Int32, _ bytes: [UInt8]) -> Int32? {
        var offset = 0
        while offset < bytes.count {
            let written = bytes[offset...].withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
            if written > 0 {
                offset += written
            } else if written < 0 && errno == EINTR {
                continue
            } else {
                return written < 0 ? errno : EIO
            }
        }
        return nil
    }
}
