import Foundation

/// Identity of the file behind `config.yaml` for cheap change polling (DESIGN §5.7, file watching).
/// Any difference means "maybe changed"; compare bytes before reloading.
public struct FileFingerprint: Sendable, Hashable {
    /// Fully resolved path (symlinks followed).
    public var realpath: String
    public var inode: UInt64
    public var size: Int64
    public var mtimeSeconds: Int
    public var mtimeNanoseconds: Int

    public init(realpath: String, inode: UInt64, size: Int64, mtimeSeconds: Int, mtimeNanoseconds: Int) {
        self.realpath = realpath
        self.inode = inode
        self.size = size
        self.mtimeSeconds = mtimeSeconds
        self.mtimeNanoseconds = mtimeNanoseconds
    }

    init(realpath: String, stat st: stat) {
        self.init(realpath: realpath, inode: UInt64(st.st_ino), size: Int64(st.st_size),
                  mtimeSeconds: st.st_mtimespec.tv_sec, mtimeNanoseconds: st.st_mtimespec.tv_nsec)
    }
}

/// Raw bytes of `config.yaml` as read from disk.
public struct ConfigFileContents: Sendable, Equatable {
    public var data: Data
    public var fingerprint: FileFingerprint
    /// File-level findings (hard links, permissions). Codec warnings are separate.
    public var warnings: [ConfigWarning]

    public init(data: Data, fingerprint: FileFingerprint, warnings: [ConfigWarning] = []) {
        self.data = data
        self.fingerprint = fingerprint
        self.warnings = warnings
    }
}

/// Outcome of ``ConfigFileStore/save(_:to:)`` and ``ConfigFileStore/update(at:_:)``.
public struct ConfigSaveResult: Sendable, Equatable {
    /// The config now on disk.
    public var config: AppConfig
    /// The bytes and fingerprint now on disk (hand the whole result to ``ConfigFileWatcher/noteOwnWrite(_:)``).
    /// `contents.warnings` lists what was found or fixed while writing.
    public var contents: ConfigFileContents
    /// `false` when ``ConfigFileStore/update(at:_:)`` found nothing to change and left the file untouched.
    public var didWrite: Bool
    /// The file as read just before this save or update (`nil` if there was none, or it could not be
    /// read). When it differs from what the watcher last saw, the file was edited by hand in between —
    /// and `update` merged that edit into ``config`` — so the watcher still reports it.
    public var baseContents: ConfigFileContents?

    public init(config: AppConfig, contents: ConfigFileContents, didWrite: Bool, baseContents: ConfigFileContents? = nil) {
        self.config = config
        self.contents = contents
        self.didWrite = didWrite
        self.baseContents = baseContents
    }
}

/// File-system failure or refusal from ``ConfigFileStore``.
public enum ConfigStoreError: Error, Sendable, Equatable, LocalizedError, CustomStringConvertible {
    /// The in-memory config failed ``AppConfig/validate()``; nothing was written.
    case invalidConfig(ConfigLoadError)
    /// The file on disk failed to decode (or is from a newer schema and must not be overwritten).
    case invalidFile(path: String, ConfigLoadError)
    case notRegularFile(path: String)
    /// `path` is a symlink whose target does not exist.
    case danglingSymlink(path: String)
    /// The file kept changing while ``ConfigFileStore/update(at:_:)`` tried to write it.
    case concurrentModification(path: String)
    case io(operation: String, path: String, errno: Int32)

    /// Localized, user-facing explanation.
    public var message: String {
        switch self {
        case let .invalidConfig(error):
            return String(localized: "The settings to save are invalid. \(error.message)", bundle: .main)
        case let .invalidFile(path, error):
            return String(localized: "Can't read the config file (\(path)). \(error.displayMessage)", bundle: .main)
        case let .notRegularFile(path):
            return String(localized: "The config file path (\(path)) is not a regular file.", bundle: .main)
        case let .danglingSymlink(path):
            return String(localized: "The config file (\(path)) is a symbolic link to a target that doesn't exist.", bundle: .main)
        case let .concurrentModification(path):
            return String(localized: "The config file (\(path)) kept changing while it was being saved, so it wasn't saved. Try again.", bundle: .main)
        case let .io(operation, path, code):
            return String(localized: "Config file operation (\(operation)) failed: \(path): \(String(cString: strerror(code)))", bundle: .main)
        }
    }

    public var errorDescription: String? { message }
    public var description: String { message }
}

/// Reads and atomically writes `config.yaml` (DESIGN §5.7, writing).
///
/// - Directory is created 0700, the file is always written 0600.
/// - Writes resolve `realpath` first so a symlinked `config.yaml` stays a symlink, then
///   `mkstemp` (same directory) → write → `fsync` → `rename`.
/// - A file from a newer `schema_version` is never overwritten.
public struct ConfigFileStore: Sendable {
    public init() {}

    /// `~/.config/FortiVPNManager/config.yaml`.
    public static func defaultFileURL(homeDirectory: URL) -> URL {
        homeDirectory
            .appendingPathComponent(".config", isDirectory: true)
            .appendingPathComponent("FortiVPNManager", isDirectory: true)
            .appendingPathComponent("config.yaml", isDirectory: false)
    }

    // MARK: Reading

    /// Stat of the resolved file, or `nil` if it does not exist (including a dangling symlink).
    public func fingerprint(of url: URL) throws(ConfigStoreError) -> FileFingerprint? {
        guard let real = try Self.resolveExisting(url.path) else { return nil }
        var st = stat()
        guard stat(real, &st) == 0 else {
            let code = errno
            if code == ENOENT || code == ENOTDIR { return nil }
            throw .io(operation: "stat", path: real, errno: code)
        }
        return FileFingerprint(realpath: real, stat: st)
    }

    /// Reads the raw bytes (at most ``ConfigLimits/maxFileSize``), or `nil` if the file does not exist.
    public func read(at url: URL) throws(ConfigStoreError) -> ConfigFileContents? {
        guard let real = try Self.resolveExisting(url.path) else { return nil }
        // O_NONBLOCK: never hang on a FIFO planted at the path; it is rejected below.
        let fd = open(real, O_RDONLY | O_NONBLOCK | O_CLOEXEC)
        guard fd >= 0 else {
            let code = errno
            if code == ENOENT || code == ENOTDIR { return nil }
            throw .io(operation: "open", path: real, errno: code)
        }
        defer { close(fd) }
        var st = stat()
        guard fstat(fd, &st) == 0 else { throw .io(operation: "fstat", path: real, errno: errno) }
        guard (st.st_mode & S_IFMT) == S_IFREG else { throw .notRegularFile(path: real) }
        let limit = ConfigLimits.maxFileSize
        if st.st_size > off_t(limit) {
            throw .invalidFile(path: real, ConfigLoadError(.fileTooLarge(byteCount: Int(st.st_size), limit: limit)))
        }
        var data = Data(count: limit + 1)
        var total = 0
        while total < data.count {
            let count = data.withUnsafeMutableBytes { buffer in
                Foundation.read(fd, buffer.baseAddress! + total, buffer.count - total)
            }
            if count < 0 {
                if errno == EINTR { continue }
                throw .io(operation: "read", path: real, errno: errno)
            }
            if count == 0 { break }
            total += count
        }
        if total > limit {
            throw .invalidFile(path: real, ConfigLoadError(.fileTooLarge(byteCount: total, limit: limit)))
        }
        data.count = total
        return ConfigFileContents(data: data, fingerprint: FileFingerprint(realpath: real, stat: st),
                                  warnings: Self.fileWarnings(st))
    }

    /// Reads and decodes the file. `nil` if it does not exist. File-level warnings (hard links,
    /// permissions) are appended to the codec warnings.
    public func load(from url: URL) throws(ConfigStoreError) -> ConfigLoadResult? {
        guard let contents = try read(at: url) else { return nil }
        do {
            var result = try ConfigCodec.decode(contents.data)
            result.warnings += contents.warnings
            return result
        } catch {
            throw .invalidFile(path: contents.fingerprint.realpath, error)
        }
    }

    // MARK: Writing

    /// Validates `config` and writes it with ``ConfigTemplate``, replacing whatever is on disk
    /// (hand-written comments and unknown keys are lost). Refuses to overwrite a newer-schema file.
    /// Prefer ``update(at:_:)`` so concurrent hand edits of other fields survive.
    public func save(_ config: AppConfig, to url: URL) throws(ConfigStoreError) -> ConfigSaveResult {
        let existing: ConfigFileContents?
        do {
            existing = try read(at: url)
        } catch {
            // An unreadable (e.g. oversized) file is what this save replaces; anything else
            // (not a regular file, I/O failure) is reported again by `write`.
            guard case .invalidFile = error else { throw error }
            existing = nil
        }
        if let existing {
            do {
                _ = try ConfigCodec.decode(existing.data)
            } catch {
                if case .newerSchemaVersion = error.kind {
                    throw .invalidFile(path: existing.fingerprint.realpath, error)
                }
                // Any other problem is what this save is about to fix.
            }
        }
        return try write(config, to: url, expecting: .anything, base: existing)
    }

    /// Read-modify-write: re-reads the latest file (defaults if missing), applies `mutate`, and writes
    /// the result only if something changed. If the file changes between the read and the final
    /// rename, the whole cycle is retried (so `mutate` may run more than once and must only depend
    /// on its input). Throws ``ConfigStoreError/invalidFile(path:_:)`` if the file on disk does not
    /// decode, including a newer `schema_version` (read-only).
    public func update(
        at url: URL,
        _ mutate: (inout AppConfig) -> Void
    ) throws(ConfigStoreError) -> ConfigSaveResult {
        let maxAttempts = 3
        var attempt = 1
        while true {
            let current = try read(at: url)
            var base = AppConfig()
            if let current {
                do {
                    base = try ConfigCodec.decode(current.data).config
                } catch {
                    throw .invalidFile(path: current.fingerprint.realpath, error)
                }
            }
            var updated = base
            mutate(&updated)
            if let current, updated == base {
                return ConfigSaveResult(config: base, contents: current, didWrite: false, baseContents: current)
            }
            let expectation: WriteExpectation = current.map { .fingerprint($0.fingerprint) } ?? .absent
            do {
                return try write(updated, to: url, expecting: expectation, base: current)
            } catch {
                guard case .concurrentModification = error, attempt < maxAttempts else { throw error }
                attempt += 1
            }
        }
    }

    // MARK: - Internals

    enum WriteExpectation {
        case anything
        case absent
        case fingerprint(FileFingerprint)
    }

    private func write(
        _ config: AppConfig, to url: URL, expecting expectation: WriteExpectation, base: ConfigFileContents?
    ) throws(ConfigStoreError) -> ConfigSaveResult {
        do {
            try config.validate()
        } catch {
            throw .invalidConfig(error)
        }
        let data = ConfigCodec.encode(config)
        let target = try Self.resolveTarget(url)

        var warnings: [ConfigWarning] = []
        var existing = stat()
        if stat(target, &existing) == 0 {
            guard (existing.st_mode & S_IFMT) == S_IFREG else { throw .notRegularFile(path: target) }
            if existing.st_nlink > 1 {
                warnings.append(ConfigWarning(.multipleHardLinks(count: Int(existing.st_nlink))))
            }
            let mode = UInt16(existing.st_mode) & 0o7777
            if mode != 0o600 {
                warnings.append(ConfigWarning(.permissionsCorrected(previousMode: mode)))
            }
        }

        let directory = (target as NSString).deletingLastPathComponent
        let name = (target as NSString).lastPathComponent
        var template = Array("\(directory)/.\(name).XXXXXX".utf8CString)
        let fd = template.withUnsafeMutableBufferPointer { mkstemp($0.baseAddress!) }
        guard fd >= 0 else { throw .io(operation: "mkstemp", path: directory, errno: errno) }
        let tempPath = template.withUnsafeBufferPointer { String(cString: $0.baseAddress!) }
        var renamed = false
        var descriptor: Int32? = fd
        defer {
            if let descriptor { close(descriptor) }
            if !renamed { unlink(tempPath) }
        }

        guard fchmod(fd, 0o600) == 0 else { throw .io(operation: "fchmod", path: tempPath, errno: errno) }
        try Self.writeAll(data, to: fd, path: tempPath)
        guard fsync(fd) == 0 else { throw .io(operation: "fsync", path: tempPath, errno: errno) }
        var written = stat()
        guard fstat(fd, &written) == 0 else { throw .io(operation: "fstat", path: tempPath, errno: errno) }
        descriptor = nil
        guard close(fd) == 0 else { throw .io(operation: "close", path: tempPath, errno: errno) }

        try Self.check(expectation, target: target)
        guard rename(tempPath, target) == 0 else { throw .io(operation: "rename", path: target, errno: errno) }
        renamed = true
        Self.syncDirectory(directory)

        let fingerprint = FileFingerprint(realpath: target, stat: written)
        return ConfigSaveResult(config: config,
                                contents: ConfigFileContents(data: data, fingerprint: fingerprint, warnings: warnings),
                                didWrite: true,
                                baseContents: base)
    }

    private static func check(_ expectation: WriteExpectation, target: String) throws(ConfigStoreError) {
        var st = stat()
        let exists = stat(target, &st) == 0
        switch expectation {
        case .anything:
            return
        case .absent:
            if exists { throw .concurrentModification(path: target) }
        case let .fingerprint(expected):
            guard exists, FileFingerprint(realpath: target, stat: st) == expected else {
                throw .concurrentModification(path: target)
            }
        }
    }

    private static func writeAll(_ data: Data, to fd: Int32, path: String) throws(ConfigStoreError) {
        var offset = 0
        while offset < data.count {
            let count = data.withUnsafeBytes { buffer in
                Foundation.write(fd, buffer.baseAddress! + offset, buffer.count - offset)
            }
            if count < 0 {
                if errno == EINTR { continue }
                throw .io(operation: "write", path: path, errno: errno)
            }
            offset += count
        }
    }

    private static func syncDirectory(_ directory: String) {
        let fd = open(directory, O_RDONLY | O_CLOEXEC)
        guard fd >= 0 else { return }
        _ = fsync(fd)
        close(fd)
    }

    private static func fileWarnings(_ st: stat) -> [ConfigWarning] {
        var warnings: [ConfigWarning] = []
        if st.st_nlink > 1 {
            warnings.append(ConfigWarning(.multipleHardLinks(count: Int(st.st_nlink))))
        }
        let mode = UInt16(st.st_mode) & 0o7777
        if mode != 0o600 {
            warnings.append(ConfigWarning(.insecurePermissions(mode: mode)))
        }
        return warnings
    }

    /// `realpath(3)` of an existing path; `nil` if it (or a symlink's target) does not exist.
    static func resolveExisting(_ path: String) throws(ConfigStoreError) -> String? {
        switch realPath(path) {
        case let .resolved(real):
            return real
        case let .failed(code):
            if code == ENOENT || code == ENOTDIR { return nil }
            throw .io(operation: "realpath", path: path, errno: code)
        }
    }

    /// Path the atomic rename must replace: the symlink's final target when the file exists,
    /// otherwise `realpath(parent)/name` (creating the parent directory 0700 if needed).
    static func resolveTarget(_ url: URL) throws(ConfigStoreError) -> String {
        let path = url.path
        var linkStat = stat()
        if lstat(path, &linkStat) == 0 {
            switch realPath(path) {
            case let .resolved(real):
                return real
            case let .failed(code):
                if (linkStat.st_mode & S_IFMT) == S_IFLNK && (code == ENOENT || code == ENOTDIR) {
                    throw .danglingSymlink(path: path)
                }
                throw .io(operation: "realpath", path: path, errno: code)
            }
        }
        let lstatError = errno
        guard lstatError == ENOENT else { throw .io(operation: "lstat", path: path, errno: lstatError) }
        let directory = url.deletingLastPathComponent().path
        try makeDirectories(directory)
        switch realPath(directory) {
        case let .resolved(realDirectory):
            return (realDirectory as NSString).appendingPathComponent(url.lastPathComponent)
        case let .failed(code):
            throw .io(operation: "realpath", path: directory, errno: code)
        }
    }

    /// `mkdir -p` with mode 0700 for every directory it creates (existing ones are left alone).
    static func makeDirectories(_ path: String) throws(ConfigStoreError) {
        if mkdir(path, 0o700) == 0 {
            _ = chmod(path, 0o700)  // independent of umask
            return
        }
        let code = errno
        switch code {
        case EEXIST:
            var st = stat()
            guard stat(path, &st) == 0, (st.st_mode & S_IFMT) == S_IFDIR else {
                throw .io(operation: "mkdir", path: path, errno: ENOTDIR)
            }
        case ENOENT:
            let parent = (path as NSString).deletingLastPathComponent
            guard !parent.isEmpty, parent != path else { throw .io(operation: "mkdir", path: path, errno: code) }
            try makeDirectories(parent)
            if mkdir(path, 0o700) == 0 {
                _ = chmod(path, 0o700)
            } else if errno != EEXIST {
                throw .io(operation: "mkdir", path: path, errno: errno)
            }
        default:
            throw .io(operation: "mkdir", path: path, errno: code)
        }
    }

    private enum RealPathResult {
        case resolved(String)
        case failed(Int32)
    }

    private static func realPath(_ path: String) -> RealPathResult {
        guard let resolved = realpath(path, nil) else { return .failed(errno) }
        defer { free(resolved) }
        return .resolved(String(cString: resolved))
    }
}
