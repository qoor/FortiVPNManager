import Darwin

/// The openfortivpn executable the helper may run as root.
struct OpenFortiVPNBinary: Sendable, Equatable {
    /// Resolved path inside the Homebrew Cellar (no symlinks).
    let path: String
    /// Version from the Cellar directory name without the Homebrew revision (`1.24.1_1` → `1.24.1`).
    let version: String?

    /// The output parser was written against openfortivpn 1.24.x.
    var isTestedVersion: Bool {
        guard let version else { return false }
        return version == "1.24" || version.hasPrefix("1.24.")
    }
}

/// Finds Homebrew's openfortivpn and applies the basic checks of DESIGN §5.2 "BinaryLocator":
/// `/opt/homebrew/bin/openfortivpn` must resolve to a regular, executable file under
/// `/opt/homebrew/Cellar/openfortivpn/` that neither its group nor other users can modify.
enum BinaryLocator {
    static let homebrewPath = "/opt/homebrew/bin/openfortivpn"
    static let cellarPrefix = "/opt/homebrew/Cellar/openfortivpn/"

    /// Why the binary cannot be used.
    struct Problem: Error, Sendable, Equatable {
        /// The resolved path, if the Homebrew link resolved at all.
        let path: String?
        /// English text shown as-is in the app's Connection tab, logged, and sent with the
        /// `binaryUnavailable` error.
        let message: String
    }

    static func locate() -> Result<OpenFortiVPNBinary, Problem> {
        guard let resolved = resolvedPath(homebrewPath) else {
            return .failure(Problem(
                path: nil,
                message: "openfortivpn not found at \(homebrewPath); install it with: brew install openfortivpn"
            ))
        }
        guard resolved.hasPrefix(cellarPrefix) else {
            return .failure(Problem(
                path: resolved,
                message: "openfortivpn resolves outside the Homebrew Cellar (\(cellarPrefix)): \(resolved)"
            ))
        }
        var info = stat()
        guard lstat(resolved, &info) == 0, info.st_mode & S_IFMT == S_IFREG else {
            return .failure(Problem(
                path: resolved,
                message: "openfortivpn is not a regular file: \(resolved)"
            ))
        }
        guard info.st_mode & (S_IWGRP | S_IWOTH) == 0 else {
            let mode = String(info.st_mode & 0o7777, radix: 8)
            return .failure(Problem(
                path: resolved,
                message: "openfortivpn is writable by other users (mode \(mode)); refusing to run it: \(resolved)"
            ))
        }
        guard info.st_mode & (S_IXUSR | S_IXGRP | S_IXOTH) != 0 else {
            return .failure(Problem(
                path: resolved,
                message: "openfortivpn is not executable: \(resolved)"
            ))
        }
        return .success(OpenFortiVPNBinary(path: resolved, version: version(fromCellarPath: resolved)))
    }

    /// `/opt/homebrew/Cellar/openfortivpn/1.24.1_1/bin/openfortivpn` → `1.24.1`.
    static func version(fromCellarPath path: String) -> String? {
        guard path.hasPrefix(cellarPrefix) else { return nil }
        let directory = path.dropFirst(cellarPrefix.count).prefix { $0 != "/" }
        guard !directory.isEmpty else { return nil }
        if let underscore = directory.lastIndex(of: "_") {
            let revision = directory[directory.index(after: underscore)...]
            if !revision.isEmpty, revision.allSatisfy({ $0.isASCII && $0.isNumber }) {
                return String(directory[..<underscore])
            }
        }
        return String(directory)
    }

    private static func resolvedPath(_ path: String) -> String? {
        guard let resolved = realpath(path, nil) else { return nil }
        defer { free(resolved) }
        return String(cString: resolved)
    }
}
