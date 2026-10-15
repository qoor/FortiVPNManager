import Foundation
import Testing
@testable import FortiConfig

@Suite struct ConfigFileWatcherTests {
    let store = ConfigFileStore()

    @Test func ignoresOwnWritesAndMetadataOnlyChanges() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        var watcher = ConfigFileWatcher(url: url)

        // Missing at launch: reported once.
        #expect(try watcher.poll() == .missing)
        #expect(try watcher.poll() == .unchanged)

        // The app writes the file and records it.
        let saved = try store.save(AppConfig(), to: url)
        watcher.noteOwnWrite(saved)
        #expect(try watcher.poll() == .unchanged)

        // Hand edit (same inode): reported with the new bytes.
        try writeInPlace("vpn:\n  host: gw\n", to: url)
        guard case let .modified(contents) = try watcher.poll() else {
            Issue.record("expected .modified")
            return
        }
        #expect(contents.data == Data("vpn:\n  host: gw\n".utf8))
        #expect(try ConfigCodec.decode(contents.data).config.vpn.host == "gw")
        #expect(try watcher.poll() == .unchanged)

        // Touch only: fingerprint changes, bytes do not.
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 1_000_000)],
                                              ofItemAtPath: url.path)
        #expect(try watcher.poll() == .unchanged)

        // Atomic replacement with identical bytes (e.g. an editor's save-as-new-file).
        let replacement = dir.appendingPathComponent("tmp.yaml")
        try writeFile("vpn:\n  host: gw\n", to: replacement)
        #expect(rename(replacement.path, url.path) == 0)
        #expect(try watcher.poll() == .unchanged)

        // Deleted, then recreated.
        try FileManager.default.removeItem(at: url)
        #expect(try watcher.poll() == .missing)
        #expect(try watcher.poll() == .unchanged)
        try writeFile("vpn:\n  host: again\n", to: url)
        guard case .modified = try watcher.poll() else {
            Issue.record("expected .modified after recreation")
            return
        }
    }

    @Test func firstPollReportsAnExistingFile() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        let saved = try store.save(nonDefaultConfig, to: url)
        var watcher = ConfigFileWatcher(url: url)
        #expect(try watcher.poll() == .modified(saved.contents))
        #expect(try watcher.poll() == .unchanged)
    }

    @Test func ownWriteWithoutNoteIsSeenAsAChange() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        var watcher = ConfigFileWatcher(url: url)
        _ = try store.save(AppConfig(), to: url)
        _ = try watcher.poll()
        _ = try store.update(at: url) { $0.vpn.host = "gw" }
        guard case .modified = try watcher.poll() else {
            Issue.record("expected .modified")
            return
        }
    }

    @Test func ownUpdateIsIgnored() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        var watcher = ConfigFileWatcher(url: url)
        #expect(try watcher.poll() == .missing)

        // Created by `update` (nothing was there).
        let created = try store.update(at: url) { $0 = nonDefaultConfig }
        #expect(created.didWrite)
        #expect(created.baseContents == nil)
        watcher.noteOwnWrite(created)
        #expect(try watcher.poll() == .unchanged)

        // Changed by `update` from the bytes the watcher already knows.
        let pinned = try store.update(at: url) { $0.vpn.pinTrustedCert(digestC) }
        #expect(pinned.didWrite)
        #expect(pinned.baseContents?.data == created.contents.data)
        watcher.noteOwnWrite(pinned)
        #expect(try watcher.poll() == .unchanged)

        // A no-op `update` is harmless to record.
        let same = try store.update(at: url) { $0.vpn.pinTrustedCert(digestC) }
        #expect(!same.didWrite)
        watcher.noteOwnWrite(same)
        #expect(try watcher.poll() == .unchanged)
    }

    /// A hand edit made after the last poll is merged by `update`; recording the write must not hide it.
    @Test(arguments: [false, true])
    func handEditMergedByUpdateIsStillReported(changesSomething: Bool) throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        var watcher = ConfigFileWatcher(url: url)
        watcher.noteOwnWrite(try store.save(nonDefaultConfig, to: url))
        #expect(try watcher.poll() == .unchanged)

        let edited = try readText(url).replacingOccurrences(of: "username: \"bob\"", with: "username: alice")
        try writeInPlace(edited, to: url)

        // digestA is already the newest pin, so pinning it again changes nothing.
        let result = try store.update(at: url) { $0.vpn.pinTrustedCert(changesSomething ? digestC : digestA) }
        #expect(result.didWrite == changesSomething)
        #expect(result.config.vpn.username == "alice")
        #expect(result.baseContents?.data == Data(edited.utf8))
        watcher.noteOwnWrite(result)

        guard case let .modified(contents) = try watcher.poll() else {
            Issue.record("the hand edit was swallowed")
            return
        }
        #expect(contents.data == result.contents.data)
        #expect(try ConfigCodec.decode(contents.data).config == result.config)
        #expect(try watcher.poll() == .unchanged)
    }

    /// `save` replaces the file; a hand edit since the last poll is reported (with the saved bytes) too.
    @Test func handEditOverwrittenBySaveIsReportedOnce() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        var watcher = ConfigFileWatcher(url: url)
        watcher.noteOwnWrite(try store.save(AppConfig(), to: url))
        #expect(try watcher.poll() == .unchanged)
        try writeInPlace("vpn:\n  host: gw\n", to: url)

        let saved = try store.save(nonDefaultConfig, to: url)
        #expect(saved.baseContents?.data == Data("vpn:\n  host: gw\n".utf8))
        watcher.noteOwnWrite(saved)
        #expect(try watcher.poll() == .modified(saved.contents))
        #expect(try watcher.poll() == .unchanged)
    }

    @Test func unreadableFileThrowsOnceUntilItChanges() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let url = dir.appendingPathComponent("config.yaml")
        try writeFile(String(repeating: "#", count: 70_000), to: url)
        var watcher = ConfigFileWatcher(url: url)
        #expect(throws: ConfigStoreError.self) { try watcher.poll() }
        #expect(try watcher.poll() == .unchanged)
        try writeFile("vpn: {}\n", to: url)
        #expect(try watcher.poll() != .unchanged)
    }

    @Test func symlinkRetargetingIsDetected() throws {
        let dir = try makeTempDirectory()
        defer { removeTempDirectory(dir) }
        let a = dir.appendingPathComponent("a.yaml")
        let b = dir.appendingPathComponent("b.yaml")
        let link = dir.appendingPathComponent("config.yaml")
        try writeFile("vpn:\n  host: a\n", to: a)
        try writeFile("vpn:\n  host: b\n", to: b)
        try FileManager.default.createSymbolicLink(atPath: link.path, withDestinationPath: "a.yaml")
        var watcher = ConfigFileWatcher(url: link)
        _ = try watcher.poll()
        try FileManager.default.removeItem(at: link)
        try FileManager.default.createSymbolicLink(atPath: link.path, withDestinationPath: "b.yaml")
        guard case let .modified(contents) = try watcher.poll() else {
            Issue.record("expected .modified")
            return
        }
        #expect(contents.fingerprint.realpath == b.path)
    }
}
