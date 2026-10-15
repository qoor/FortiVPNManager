import Darwin
import Foundation
import FortiShared
import os

/// Appends raw helper output to `~/Library/Logs/FortiVPNManager/openfortivpn.log` (DESIGN §5.6):
/// directory 0700, files 0600, rotated at 2 MB keeping five files (`openfortivpn.log`, `.1` … `.4`).
/// The text is unmasked, like the Log tab; it holds no secrets at openfortivpn's default verbosity.
actor LogFileWriter {
    nonisolated static let maxFileBytes = 2 * 1024 * 1024
    nonisolated static let keptFiles = 5

    nonisolated let directory: URL
    nonisolated let fileURL: URL

    private var fd: Int32 = -1
    private var size = 0
    private var reportedFailure = false
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "logfile")

    nonisolated static var defaultDirectory: URL {
        URL.libraryDirectory.appending(path: "Logs/FortiVPNManager", directoryHint: .isDirectory)
    }

    init(directory: URL = LogFileWriter.defaultDirectory) {
        self.directory = directory
        fileURL = directory.appending(path: "openfortivpn.log", directoryHint: .notDirectory)
    }

    deinit {
        if fd >= 0 { close(fd) }
    }

    func write(_ lines: [OutputLine]) {
        guard !lines.isEmpty else { return }
        let data = Data(lines.map { LogLineFormat.fileLine($0) + "\n" }.joined().utf8)
        do {
            if fd < 0 { try open() }
            if size > 0, size + data.count > Self.maxFileBytes {
                try rotate()
            }
            try FileIO.writeAll(data, to: fd, path: fileURL.path)
            size += data.count
            reportedFailure = false
        } catch {
            closeFile()
            if !reportedFailure {
                reportedFailure = true
                logger.error("log file write failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    private func open() throws(FileIOError) {
        try FileIO.makeDirectory(directory)
        let path = fileURL.path
        let descriptor = Darwin.open(path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw FileIOError(operation: "open", path: path, code: errno) }
        var info = stat()
        guard fchmod(descriptor, 0o600) == 0, fstat(descriptor, &info) == 0 else {
            let code = errno
            close(descriptor)
            throw FileIOError(operation: "fchmod", path: path, code: code)
        }
        fd = descriptor
        size = Int(info.st_size)
    }

    /// `openfortivpn.log` → `.1` → … → `.4`; the oldest is dropped.
    private func rotate() throws(FileIOError) {
        closeFile()
        let path = fileURL.path
        unlink("\(path).\(Self.keptFiles - 1)")
        for index in stride(from: Self.keptFiles - 2, through: 1, by: -1) {
            rename("\(path).\(index)", "\(path).\(index + 1)")
        }
        guard rename(path, "\(path).1") == 0 || errno == ENOENT else {
            throw FileIOError(operation: "rename", path: path, code: errno)
        }
        try open()
    }

    private func closeFile() {
        if fd >= 0 { close(fd) }
        fd = -1
        size = 0
    }
}
