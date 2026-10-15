import Darwin
import Foundation

/// A failed POSIX call on a path.
nonisolated struct FileIOError: LocalizedError, Equatable {
    var operation: String
    var path: String
    var code: Int32

    var errorDescription: String? {
        String(localized: "\(operation) failed: \(path): \(String(cString: strerror(code)))")
    }
}

/// Small POSIX file helpers for the app's own files (state.json, logs, imported configs).
nonisolated enum FileIO {
    /// `mkdir -p` that creates missing directories with `mode` and tightens the last one to `mode`.
    static func makeDirectory(_ url: URL, mode: mode_t = 0o700) throws(FileIOError) {
        do {
            try FileManager.default.createDirectory(
                at: url, withIntermediateDirectories: true, attributes: [.posixPermissions: NSNumber(value: mode)]
            )
        } catch {
            throw FileIOError(operation: "mkdir", path: url.path, code: (error as NSError).posixCode ?? EIO)
        }
        guard chmod(url.path, mode) == 0 else { throw FileIOError(operation: "chmod", path: url.path, code: errno) }
    }

    /// Atomically replaces `url` with `data`: `mkstemp` in the same directory (0600) → write → fsync → rename.
    static func writeAtomically(_ data: Data, to url: URL, mode: mode_t = 0o600) throws(FileIOError) {
        let directory = url.deletingLastPathComponent().path
        var template = Array("\(directory)/.\(url.lastPathComponent).XXXXXX".utf8CString)
        let fd = template.withUnsafeMutableBufferPointer { buffer in
            buffer.baseAddress.map { mkstemp($0) } ?? -1
        }
        guard fd >= 0 else { throw FileIOError(operation: "mkstemp", path: directory, code: errno) }
        let tempPath = String(decoding: template.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
        var renamed = false
        defer {
            close(fd)
            if !renamed { unlink(tempPath) }
        }
        guard fchmod(fd, mode) == 0 else { throw FileIOError(operation: "fchmod", path: tempPath, code: errno) }
        try writeAll(data, to: fd, path: tempPath)
        guard fsync(fd) == 0 else { throw FileIOError(operation: "fsync", path: tempPath, code: errno) }
        guard rename(tempPath, url.path) == 0 else { throw FileIOError(operation: "rename", path: url.path, code: errno) }
        renamed = true
    }

    static func writeAll(_ data: Data, to fd: Int32, path: String) throws(FileIOError) {
        var offset = 0
        while offset < data.count {
            let count = data.withUnsafeBytes { buffer in
                buffer.baseAddress.map { write(fd, $0 + offset, buffer.count - offset) } ?? 0
            }
            if count < 0 {
                if errno == EINTR { continue }
                throw FileIOError(operation: "write", path: path, code: errno)
            }
            offset += count
        }
    }

    /// Why `readRegularFile` refused a file.
    enum ReadError: Error, Equatable {
        case notFound
        case notRegularFile
        case tooLarge(limit: Int)
        case io(FileIOError)
    }

    /// Reads a regular file of at most `limit` bytes without following FIFOs or devices.
    static func readRegularFile(_ url: URL, limit: Int) throws(ReadError) -> Data {
        let path = url.path
        let fd = open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC)
        guard fd >= 0 else {
            let code = errno
            if code == ENOENT || code == ENOTDIR { throw .notFound }
            throw .io(FileIOError(operation: "open", path: path, code: code))
        }
        defer { close(fd) }
        var info = stat()
        guard fstat(fd, &info) == 0 else { throw .io(FileIOError(operation: "fstat", path: path, code: errno)) }
        guard (info.st_mode & S_IFMT) == S_IFREG else { throw .notRegularFile }
        guard info.st_size <= off_t(limit) else { throw .tooLarge(limit: limit) }

        var data = Data(count: limit + 1)
        var total = 0
        while total < data.count {
            let count = data.withUnsafeMutableBytes { buffer in
                buffer.baseAddress.map { read(fd, $0 + total, buffer.count - total) } ?? 0
            }
            if count < 0 {
                if errno == EINTR { continue }
                throw .io(FileIOError(operation: "read", path: path, code: errno))
            }
            if count == 0 { break }
            total += count
        }
        guard total <= limit else { throw .tooLarge(limit: limit) }
        data.count = total
        return data
    }
}

private extension NSError {
    /// The POSIX errno behind a Foundation file error, if any.
    nonisolated var posixCode: Int32? {
        if domain == NSPOSIXErrorDomain { return Int32(code) }
        return (userInfo[NSUnderlyingErrorKey] as? NSError)?.posixCode
    }
}
