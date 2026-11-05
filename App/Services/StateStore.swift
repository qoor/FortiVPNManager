import Foundation
import FortiCore
import FortiShared
import os

/// Per-device transient state in `state.json`, kept out of the dotfiles-synced `config.yaml`.
final class StateStore {
    struct State: Codable, Equatable {
        /// Never read, but builds before 2026-11-05 require the key and would otherwise drop the whole
        /// file, including the failure ledger.
        var schemaVersion = 1
        /// Authentication failure bookkeeping for one `username@host`.
        var ledger: FailureLedger?
        /// The login item was registered on the first launch from /Applications.
        var loginItemRegistered = false
        /// The additional routes the exception rules last wrote, by SCPreferences service ID.
        /// Plain strings without validation, so no change to the address rules can make the file unreadable.
        var appliedExceptionRoutes: [String: [AdditionalRoute]]?
    }

    let fileURL = URL.applicationSupportDirectory
        .appending(path: "FortiVPNManager", directoryHint: .isDirectory)
        .appending(path: "state.json", directoryHint: .notDirectory)
    private(set) var state: State
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "state")

    /// Loads the file; a missing or unreadable file starts from an empty state (overwritten on the next save).
    init() {
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

    /// The routes the exception rules last wrote to the service `serviceID` (empty if never).
    func appliedExceptionRoutes(serviceID: String) -> [AdditionalRoute] {
        state.appliedExceptionRoutes?[serviceID] ?? []
    }

    func setAppliedExceptionRoutes(_ routes: [AdditionalRoute], serviceID: String) {
        update { state in
            var applied = state.appliedExceptionRoutes ?? [:]
            applied[serviceID] = routes.isEmpty ? nil : routes
            state.appliedExceptionRoutes = applied.isEmpty ? nil : applied
        }
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
