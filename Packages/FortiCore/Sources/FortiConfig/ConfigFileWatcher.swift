import Foundation

/// What a poll found.
public enum ConfigFileChange: Sendable, Equatable {
    /// Same bytes as last time (including the app's own writes and touch-only changes).
    case unchanged
    /// The file is gone (reported once per disappearance).
    case missing
    /// The bytes differ from the last ones seen; decode `contents.data` with ``ConfigCodec``.
    case modified(ConfigFileContents)
}

/// Polling state for `config.yaml` (DESIGN §5.7, file watching): a cheap ``FileFingerprint`` stat every
/// poll, and a byte comparison before reporting a change, so writes made by the app itself
/// (recorded with ``noteOwnWrite(_:)``) and metadata-only changes are ignored. Hand edits are always
/// reported, including one that ``ConfigFileStore/update(at:_:)`` merged into the app's own write.
///
/// Value type; the app keeps one in its MainActor polling task and calls ``poll()`` every 2 s and
/// when it becomes active.
public struct ConfigFileWatcher: Sendable {
    public let url: URL
    private let store: ConfigFileStore
    private var lastFingerprint: FileFingerprint?
    private var lastData: Data?
    private var hasPolled = false

    public init(url: URL, store: ConfigFileStore = ConfigFileStore()) {
        self.url = url
        self.store = store
    }

    /// Records bytes the app just wrote so the next ``poll()`` does not report them as a change.
    ///
    /// Only a write that started from the bytes this watcher last saw is recorded. If the file had been
    /// edited by hand since the last poll (``ConfigSaveResult/baseContents`` differs), nothing is recorded
    /// and the next ``poll()`` reports the file as `.modified`, so the app still runs its change handling
    /// for that edit. That also covers an `update` that found nothing to change (`didWrite == false`).
    public mutating func noteOwnWrite(_ result: ConfigSaveResult) {
        guard result.baseContents?.data == lastData else { return }
        lastFingerprint = result.contents.fingerprint
        lastData = result.contents.data
        hasPolled = true
    }

    /// Checks the file. The first poll reports `.modified` (or `.missing`). If reading fails, the
    /// fingerprint is still remembered so the same broken file is not re-read every poll; the
    /// error is thrown once and the next real change is reported again.
    public mutating func poll() throws(ConfigStoreError) -> ConfigFileChange {
        let firstPoll = !hasPolled
        hasPolled = true
        guard let fingerprint = try store.fingerprint(of: url) else {
            let wasPresent = lastFingerprint != nil || lastData != nil
            lastFingerprint = nil
            lastData = nil
            return wasPresent || firstPoll ? .missing : .unchanged
        }
        if fingerprint == lastFingerprint {
            return .unchanged
        }
        let contents: ConfigFileContents?
        do {
            contents = try store.read(at: url)
        } catch {
            lastFingerprint = fingerprint
            lastData = nil
            throw error
        }
        guard let contents else {
            // Deleted between stat and read.
            let wasPresent = lastFingerprint != nil || lastData != nil
            lastFingerprint = nil
            lastData = nil
            return wasPresent || firstPoll ? .missing : .unchanged
        }
        lastFingerprint = contents.fingerprint
        if contents.data == lastData {
            return .unchanged
        }
        lastData = contents.data
        return .modified(contents)
    }
}
