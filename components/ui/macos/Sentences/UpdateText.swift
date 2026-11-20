import Foundation

extension AppInternal.AutoUpdateSection {
    var failedInstallMessage: String? {
        failedInstallVersion.map { String(localized: "Couldn't install version \($0).") }
    }

    static func downloadLine(received: Int, total: Int, remaining: AppInternal.RemainingTime) -> String {
        String(localized: "\(size(received)) of \(size(total)) — \(remaining.text)")
    }

    static func size(_ bytes: Int) -> String {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        formatter.allowedUnits = [.useKB, .useMB, .useGB, .useTB, .usePB, .useEB, .useZB, .useYBOrHigher]
        formatter.allowsNonnumericFormatting = false
        return formatter.string(fromByteCount: Int64(bytes))
    }
}

extension AppInternal.UpdateStatus {
    func releaseNoteTitles(in language: String) -> [String] {
        guard case let .available(_, _, _, titles) = self else { return [] }
        return titles[language] ?? titles["en"] ?? []
    }
}

extension AppInternal.RemainingTime {
    var text: String {
        switch self {
        case .calculating: String(localized: "Calculating time remaining…")
        case let .minutes(minutes): String(localized: "About \(minutes) minutes remaining")
        case let .seconds(seconds): String(localized: "About \(seconds) seconds remaining")
        }
    }
}

extension AppInternal.UpdateCheckFailure {
    var message: String {
        switch self {
        case let .httpStatus(code): String(localized: "Couldn't check for updates (HTTP \(code)).")
        case let .network(detail): String(localized: "Couldn't check for updates: \(detail)")
        case .invalidReply: String(localized: "Couldn't check for updates (unexpected reply).")
        case .installerMissing: String(localized: "Couldn't check for updates (installer missing).")
        }
    }
}

extension AppInternal.InstallFailure {
    var message: String {
        switch self {
        case .downloadFailed:
            String(localized: "Couldn't download the update. Check the network connection.")
        case .waiterFailed:
            String(localized: "Couldn't prepare the install. Try installing again.")
        case .helperUnreachable, .helperBuildMismatch, .helperFailed:
            String(localized: "Couldn't reach the Helper, so the update wasn't installed.")
        case let .helperRefused(reason, _):
            switch reason {
            case .signatureInvalid, .checksumMismatch, .filesUnreadable:
                String(localized: "Couldn't verify the update file. Try installing again.")
            case .versionNotNewer:
                String(localized: "The same or a newer version is already installed.")
            case .alreadyInstalling:
                String(localized: "Another update is already being installed.")
            case .nothingPrepared:
                String(localized: "Couldn't prepare the install. Try installing again.")
            }
        }
    }
}
