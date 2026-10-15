import Foundation
import Testing
@testable import FortiConfig

@Suite struct ConfigFileStoreTests {
    let store = ConfigFileStore()

    @Test func defaultLocation() {
        let url = ConfigFileStore.defaultFileURL(homeDirectory: URL(fileURLWithPath: "/Users/someone"))
        #expect(url.path == "/Users/someone/.config/FortiVPNManager/config.yaml")
    }

    @Test func missingFileReadsAsNil() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("absent/config.yaml")
        #expect(try store.read(at: url) == nil)
        #expect(try store.load(from: url) == nil)
        #expect(try store.fingerprint(of: url) == nil)
    }

    @Test func saveCreatesPrivateDirectoryAndFile() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let configDir = dir.appendingPathComponent(".config/FortiVPNManager", isDirectory: true)
        let url = configDir.appendingPathComponent("config.yaml")

        let result = try store.save(nonDefaultConfig, to: url)
        #expect(result.didWrite)
        #expect(result.config == nonDefaultConfig)
        #expect(result.contents.warnings.isEmpty)
        #expect(result.contents.data == ConfigCodec.encode(nonDefaultConfig))
        #expect(try Data(contentsOf: url) == result.contents.data)
        #expect(try permissions(configDir.path) == 0o700)
        #expect(try permissions(dir.appendingPathComponent(".config").path) == 0o700)
        #expect(try permissions(url.path) == 0o600)
        #expect(result.contents.fingerprint == (try store.fingerprint(of: url)))

        let loaded = try #require(try store.load(from: url))
        #expect(loaded.config == nonDefaultConfig)
        #expect(loaded.warnings.isEmpty)
        // No temporary files are left behind.
        #expect(try FileManager.default.contentsOfDirectory(atPath: configDir.path) == ["config.yaml"])
    }

    @Test func existingDirectoryPermissionsAreLeftAlone() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        #expect(chmod(dir.path, 0o755) == 0)
        _ = try store.save(AppConfig(), to: dir.appendingPathComponent("config.yaml"))
        #expect(try permissions(dir.path) == 0o755)
    }

    @Test func symlinkedConfigStaysASymlink() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let dotfiles = dir.appendingPathComponent("dotfiles", isDirectory: true)
        let configDir = dir.appendingPathComponent("config", isDirectory: true)
        try FileManager.default.createDirectory(at: dotfiles, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: configDir, withIntermediateDirectories: true)
        let real = dotfiles.appendingPathComponent("config.yaml")
        try writeFile("vpn:\n  host: old\n", to: real, mode: 0o644)
        // Two-level, relative symlink chain: config.yaml -> hop.yaml -> ../dotfiles/config.yaml
        let hop = configDir.appendingPathComponent("hop.yaml")
        let link = configDir.appendingPathComponent("config.yaml")
        try FileManager.default.createSymbolicLink(atPath: hop.path, withDestinationPath: "../dotfiles/config.yaml")
        try FileManager.default.createSymbolicLink(atPath: link.path, withDestinationPath: "hop.yaml")

        let loaded = try #require(try store.load(from: link))
        #expect(loaded.config.vpn.host == "old")
        #expect(loaded.warnings.contains(ConfigWarning(.insecurePermissions(mode: 0o644))))

        let result = try store.update(at: link) { $0.vpn.host = "new" }
        #expect(result.didWrite)
        #expect(try isSymlink(link.path))
        #expect(try isSymlink(hop.path))
        #expect(try !isSymlink(real.path))
        #expect(try permissions(real.path) == 0o600)
        #expect(try ConfigCodec.decode(Data(contentsOf: real)).config.vpn.host == "new")
        #expect(result.contents.fingerprint.realpath == real.path)
        #expect(result.contents.warnings.contains(ConfigWarning(.permissionsCorrected(previousMode: 0o644))))
        #expect(try FileManager.default.contentsOfDirectory(atPath: dotfiles.path) == ["config.yaml"])
    }

    @Test func danglingSymlinkIsNotFollowedOrReplaced() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let link = dir.appendingPathComponent("config.yaml")
        try FileManager.default.createSymbolicLink(atPath: link.path, withDestinationPath: "missing/config.yaml")
        #expect(try store.read(at: link) == nil)
        let error = #expect(throws: ConfigStoreError.self) { try store.save(AppConfig(), to: link) }
        #expect(error == .danglingSymlink(path: link.path))
        #expect(try isSymlink(link.path))
    }

    @Test func permissionsAreReportedOnLoadAndFixedOnSave() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        try writeFile("vpn:\n  host: gw\n", to: url, mode: 0o644)

        let loaded = try #require(try store.load(from: url))
        #expect(loaded.warnings == [ConfigWarning(.insecurePermissions(mode: 0o644))])
        #expect(loaded.warnings[0].message.contains("0644"))

        let saved = try store.save(loaded.config, to: url)
        #expect(saved.contents.warnings == [ConfigWarning(.permissionsCorrected(previousMode: 0o644))])
        #expect(try permissions(url.path) == 0o600)
        #expect(try store.load(from: url)?.warnings == [])
    }

    @Test func hardLinkIsWarnedAndDetachedBySave() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        let other = dir.appendingPathComponent("backup.yaml")
        try writeFile("vpn:\n  host: gw\n", to: url)
        #expect(link(url.path, other.path) == 0)

        let loaded = try #require(try store.load(from: url))
        #expect(loaded.warnings == [ConfigWarning(.multipleHardLinks(count: 2))])

        let saved = try store.update(at: url) { $0.vpn.port = 8443 }
        #expect(saved.contents.warnings == [ConfigWarning(.multipleHardLinks(count: 2))])
        #expect(try statInfo(url.path).st_nlink == 1)
        #expect(try readText(other) == "vpn:\n  host: gw\n")
        #expect(try store.load(from: url)?.config.vpn.port == 8443)
    }

    @Test func updateKeepsAConcurrentHandEditOfAnotherField() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        // The app saved this and still holds it as last-known-good...
        let lastKnownGood = try store.save(nonDefaultConfig, to: url).config
        // ...then the user edits the username by hand (in place, keeping a comment).
        let edited = try readText(url).replacingOccurrences(of: "username: \"bob\"", with: "username: alice # \u{C0C8} \u{ACC4}\u{C815}")
        try writeInPlace(edited, to: url)
        #expect(lastKnownGood.vpn.username == "bob")

        // The app stores a new certificate digest.
        let result = try store.update(at: url) { $0.vpn.pinTrustedCert(digestC) }
        #expect(result.didWrite)
        #expect(result.config.vpn.username == "alice")
        #expect(result.config.vpn.trustedCerts == [digestC, digestA, digestB])
        let onDisk = try #require(try store.load(from: url)).config
        #expect(onDisk == result.config)
        var expected = nonDefaultConfig
        expected.vpn.username = "alice"
        expected.vpn.trustedCerts = [digestC, digestA, digestB]
        #expect(onDisk == expected)
    }

    @Test func updateRetriesWhenTheFileChangesMidway() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        _ = try store.save(AppConfig(), to: url)

        var calls = 0
        let result = try store.update(at: url) { config in
            calls += 1
            if calls == 1 {
                // Simulates a hand edit landing between the read and the rename.
                try? writeInPlace("vpn:\n  username: bob\n", to: url)
            }
            config.connection.sessionMaxMinutes = 120
        }
        #expect(calls == 2)
        #expect(result.config.vpn.username == "bob")
        #expect(result.config.connection.sessionMaxMinutes == 120)
        #expect(try store.load(from: url)?.config == result.config)
    }

    @Test func updateGivesUpAfterRepeatedConcurrentChanges() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        _ = try store.save(AppConfig(), to: url)
        var calls = 0
        let error = #expect(throws: ConfigStoreError.self) {
            try store.update(at: url) { config in
                calls += 1
                try? writeInPlace("vpn:\n  username: u\(calls)\(String(repeating: " ", count: calls))\n", to: url)
                config.vpn.port = 1000
            }
        }
        #expect(error == .concurrentModification(path: url.path))
        #expect(calls == 3)
        #expect(try store.load(from: url)?.config.vpn.port == 443)
    }

    @Test func updateWithoutChangesLeavesTheFileUntouched() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        let handWritten = "# my notes\nvpn:\n  host: gw  # keep me\n"
        try writeFile(handWritten, to: url)
        let before = try store.fingerprint(of: url)

        let result = try store.update(at: url) { $0.vpn.host = "gw" }
        #expect(!result.didWrite)
        #expect(result.contents.data == Data(handWritten.utf8))
        #expect(try readText(url) == handWritten)
        #expect(try store.fingerprint(of: url) == before)
    }

    @Test func updateCreatesAMissingFileFromDefaults() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("new/config.yaml")
        let result = try store.update(at: url) { $0.vpn.host = "gw" }
        #expect(result.didWrite)
        var expected = AppConfig()
        expected.vpn.host = "gw"
        #expect(try store.load(from: url)?.config == expected)
        #expect(try permissions(url.path) == 0o600)
    }

    @Test func updateRefusesABrokenFileAndLeavesItAlone() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        let broken = "vpn:\n  port: 0443\n"
        try writeFile(broken, to: url)
        let error = #expect(throws: ConfigStoreError.self) { try store.update(at: url) { $0.vpn.host = "x" } }
        guard case let .invalidFile(path, loadError)? = error else {
            Issue.record("unexpected \(String(describing: error))")
            return
        }
        #expect(path == url.path)
        #expect(loadError.kind == .invalidInteger(path: "vpn.port", raw: "0443"))
        #expect(loadError.location == loc(2, 9))
        #expect(try readText(url) == broken)
        #expect(error?.message.contains("Line 2, column 9") == true)
    }

    @Test func newerSchemaFilesAreNeverOverwritten() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        let future = "schema_version: 2\nvpn:\n  host: gw\n  new_field: [1]\n"
        try writeFile(future, to: url)

        let updateError = #expect(throws: ConfigStoreError.self) { try store.update(at: url) { $0.vpn.host = "x" } }
        let saveError = #expect(throws: ConfigStoreError.self) { try store.save(AppConfig(), to: url) }
        for error in [updateError, saveError] {
            guard case let .invalidFile(_, loadError)? = error else {
                Issue.record("unexpected \(String(describing: error))")
                continue
            }
            #expect(loadError.kind == .newerSchemaVersion(2))
        }
        #expect(try readText(url) == future)
    }

    @Test func saveReplacesABrokenFile() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        try writeFile("vpn: [unclosed\n", to: url)
        _ = try store.save(nonDefaultConfig, to: url)
        #expect(try store.load(from: url)?.config == nonDefaultConfig)
    }

    @Test func saveReplacesAnOversizedFile() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        try writeFile(String(repeating: "#", count: 70_000), to: url)
        #expect(throws: ConfigStoreError.self) { try store.update(at: url) { $0.vpn.host = "x" } }
        _ = try store.save(AppConfig(), to: url)
        #expect(try store.load(from: url)?.config == AppConfig())
    }

    @Test func saveRejectsAnInvalidConfigWithoutWriting() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        var config = AppConfig()
        config.vpn.port = 0
        let error = #expect(throws: ConfigStoreError.self) { try store.save(config, to: url) }
        #expect(error == .invalidConfig(ConfigLoadError(.outOfRange(path: "vpn.port", raw: "0", range: 1...65535))))
        #expect(!FileManager.default.fileExists(atPath: url.path))
        #expect(try FileManager.default.contentsOfDirectory(atPath: dir.path).isEmpty)
    }

    @Test func loadReportsDecodeErrorsWithPath() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        try writeFile("vpn:\n  port: \"443\"\n", to: url)
        let error = #expect(throws: ConfigStoreError.self) { try store.load(from: url) }
        guard case let .invalidFile(path, loadError)? = error else {
            Issue.record("unexpected \(String(describing: error))")
            return
        }
        #expect(path == url.path)
        #expect(loadError.kind == .quotedNumber(path: "vpn.port", raw: "443"))
    }

    /// Whatever `save`/`update` accept must be readable by `load` and `update` afterwards.
    @Test func writesThatCouldNotBeReadBackAreRefused() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        let saved = try store.save(nonDefaultConfig, to: url)

        var huge = nonDefaultConfig
        huge.vpn.realm = String(repeating: "r", count: 70_000)
        let size = ConfigCodec.encode(huge).count
        let tooLarge = ConfigStoreError.invalidConfig(ConfigLoadError(.fileTooLarge(byteCount: size, limit: ConfigLimits.maxFileSize)))
        #expect(throws: tooLarge) { try store.save(huge, to: url) }
        #expect(throws: tooLarge) { try store.update(at: url) { $0.vpn.realm = huge.vpn.realm } }
        #expect(try store.read(at: url)?.data == saved.contents.data)

        // The largest config that fits round-trips through the file.
        var largest = nonDefaultConfig
        largest.vpn.realm = ""
        largest.vpn.realm = String(repeating: "r", count: ConfigLimits.maxFileSize - ConfigCodec.encode(largest).count)
        let written = try store.update(at: url) { $0.vpn.realm = largest.vpn.realm }
        #expect(written.contents.data.count == ConfigLimits.maxFileSize)
        #expect(try store.load(from: url)?.config == largest)
        #expect(try store.update(at: url) { $0.vpn.pinTrustedCert(digestA) }.didWrite == false)
    }

    @Test func oversizedFileIsRejectedBeforeReading() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        try writeFile("# " + String(repeating: "x", count: 70_000) + "\n", to: url)
        let error = #expect(throws: ConfigStoreError.self) { try store.read(at: url) }
        #expect(error == .invalidFile(path: url.path,
                                      ConfigLoadError(.fileTooLarge(byteCount: 70_003, limit: 65_536))))
    }

    @Test func nonRegularFilesAreRejected() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let fifo = dir.appendingPathComponent("fifo.yaml")
        #expect(mkfifo(fifo.path, 0o600) == 0)
        // Must not block waiting for a writer.
        #expect(throws: ConfigStoreError.notRegularFile(path: fifo.path)) { try store.read(at: fifo) }
        #expect(throws: ConfigStoreError.notRegularFile(path: fifo.path)) { try store.save(AppConfig(), to: fifo) }

        let directory = dir.appendingPathComponent("dir.yaml", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        #expect(throws: ConfigStoreError.notRegularFile(path: directory.path)) { try store.read(at: directory) }
    }

    @Test func fingerprintTracksReplacementAndEdits() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        let first = try store.save(AppConfig(), to: url).contents.fingerprint
        #expect(first.realpath == url.path)
        #expect(first.size == Int64(ConfigCodec.encode(AppConfig()).count))
        let second = try store.update(at: url) { $0.vpn.host = "gw" }.contents.fingerprint
        #expect(second != first)
        #expect(second.inode != first.inode)  // atomic rename → new inode
        #expect(try store.fingerprint(of: url) == second)
    }
}
