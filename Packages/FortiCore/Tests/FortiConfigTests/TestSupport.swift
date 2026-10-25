import Foundation
import Testing
@testable import FortiConfig

/// Decodes a YAML string, returning the error instead of throwing.
func loadError(_ yaml: String) -> ConfigLoadError? {
    loadError(Data(yaml.utf8))
}

func loadError(_ data: Data) -> ConfigLoadError? {
    do {
        _ = try ConfigCodec.decode(data)
        return nil
    } catch {
        return error
    }
}

func decodeYAML(_ yaml: String) throws -> ConfigLoadResult {
    try ConfigCodec.decode(Data(yaml.utf8))
}

/// String with one scalar; Swift literals cannot spell noncharacters such as U+FFFE.
func scalar(_ value: UInt32) -> String {
    String(Unicode.Scalar(value)!)
}

func loc(_ line: Int, _ column: Int) -> LineColumn {
    LineColumn(line: line, column: column)
}

let digestA = String(repeating: "a", count: 64)
let digestB = String(repeating: "0123456789abcdef", count: 4)
let digestC = String(repeating: "c", count: 63) + "d"
let digestD = String(repeating: "d", count: 64)
let digestE = String(repeating: "e", count: 64)

let googleClientID = "123456789012-abcdefghijklmnopqrstuvwxyz012345.apps.googleusercontent.com"

/// A config with every field changed from the defaults.
let nonDefaultConfig = AppConfig(
    vpn: AppConfig.VPN(
        host: "vpn.example.com", port: 8443, username: "bob", realm: "corp",
        trustedCerts: [digestA, digestB]),
    otp: AppConfig.OTP(gmailAddress: "otp@example.com", method: .gmailAPI, googleClientID: googleClientID),
    connection: AppConfig.Connection(
        autoConnectOnLaunch: false, sessionMaxMinutes: 175,
        proactiveReconnect: .init(enabled: false, marginMinutes: 7),
        reconnectAfterSleep: .init(enabled: true, minSleepMinutes: 45),
        schedule: .init(
            enabled: true, days: [.mon, .wed, .sat],
            start: TimeOfDay(hour: 8, minute: 30), end: TimeOfDay(hour: 19, minute: 5), awayMinutes: 90)))

/// Fresh directory under the system temp dir, resolved with realpath (`/var` → `/private/var`).
func makeTempDirectory() throws -> URL {
    let base = FileManager.default.temporaryDirectory
        .appendingPathComponent("FortiConfigTests-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
    return URL(fileURLWithPath: try realPath(base.path), isDirectory: true)
}

func removeTempDirectory(_ url: URL) {
    try? FileManager.default.removeItem(at: url)
}

struct POSIXFailure: Error {
    var function: String
    var code: Int32
}

func realPath(_ path: String) throws -> String {
    guard let resolved = realpath(path, nil) else { throw POSIXFailure(function: "realpath", code: errno) }
    defer { free(resolved) }
    return String(cString: resolved)
}

func lstatInfo(_ path: String) throws -> stat {
    var st = stat()
    guard lstat(path, &st) == 0 else { throw POSIXFailure(function: "lstat", code: errno) }
    return st
}

func statInfo(_ path: String) throws -> stat {
    var st = stat()
    guard stat(path, &st) == 0 else { throw POSIXFailure(function: "stat", code: errno) }
    return st
}

func permissions(_ path: String) throws -> UInt16 {
    UInt16(try statInfo(path).st_mode) & 0o7777
}

func isSymlink(_ path: String) throws -> Bool {
    (try lstatInfo(path).st_mode & S_IFMT) == S_IFLNK
}

/// Writes bytes in place (same inode), like an editor that does not replace the file.
func writeInPlace(_ text: String, to url: URL) throws {
    let handle = try FileHandle(forWritingTo: url)
    defer { try? handle.close() }
    try handle.truncate(atOffset: 0)
    try handle.write(contentsOf: Data(text.utf8))
}

func writeFile(_ text: String, to url: URL, mode: Int = 0o600) throws {
    try Data(text.utf8).write(to: url)
    guard chmod(url.path, mode_t(mode)) == 0 else { throw POSIXFailure(function: "chmod", code: errno) }
}

func readText(_ url: URL) throws -> String {
    String(decoding: try Data(contentsOf: url), as: UTF8.self)
}
