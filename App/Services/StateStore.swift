import Foundation
import FortiCore
import FortiShared
import os

/// Per-device transient state in `~/Library/Application Support/FortiVPNManager/state.json`
/// (DESIGN §5.7): the failure ledger and whether the login item was registered once.
/// Written atomically with mode 0600; kept out of the dotfiles-synced `config.yaml`.
final class StateStore {
    struct State: Codable, Equatable {
        var schemaVersion = 1
        /// Authentication failure bookkeeping for one `username@host`.
        var ledger: FailureLedger?
        /// The login item was registered on the first launch from /Applications.
        var loginItemRegistered = false
    }

    let fileURL: URL
    private(set) var state: State
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "state")

    static var defaultFileURL: URL {
        URL.applicationSupportDirectory
            .appending(path: "FortiVPNManager", directoryHint: .isDirectory)
            .appending(path: "state.json", directoryHint: .notDirectory)
    }

    /// Loads the file; a missing or unreadable file starts from an empty state (overwritten on the next save).
    init(fileURL: URL = StateStore.defaultFileURL) {
        self.fileURL = fileURL
        state = State()
        do {
            let data = try FileIO.readRegularFile(fileURL, limit: 1 << 20)
            state = try Self.decoder().decode(State.self, from: data)
        } catch FileIO.ReadError.notFound {
            // First launch.
        } catch {
            logger.error("ignoring unreadable state.json: \(String(describing: error), privacy: .public)")
        }
    }

    var ledger: FailureLedger? { state.ledger }
    var loginItemRegistered: Bool { state.loginItemRegistered }

    /// Effect `.persistLedger`.
    func saveLedger(_ ledger: FailureLedger) {
        update { $0.ledger = ledger }
    }

    func setLoginItemRegistered(_ registered: Bool) {
        update { $0.loginItemRegistered = registered }
    }

    private func update(_ mutate: (inout State) -> Void) {
        var next = state
        mutate(&next)
        guard next != state else { return }
        state = next
        do {
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            let data = try encoder.encode(next)
            try FileIO.makeDirectory(fileURL.deletingLastPathComponent())
            try FileIO.writeAtomically(data, to: fileURL)
        } catch {
            logger.error("saving state.json failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    private static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }
}
