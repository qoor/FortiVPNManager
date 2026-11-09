import Foundation

extension AppInternal.HelperState {
    var label: String {
        switch self {
        case .checking: String(localized: "Checking…")
        case .notRegistered: String(localized: "Not registered")
        case .requiresApproval: String(localized: "Needs approval")
        case let .running(build): String(localized: "Running (build \(build))")
        case let .versionMismatch(helperBuild, appBuild):
            String(localized: "Version mismatch (Helper \(helperBuild), app \(appBuild))")
        case let .unreachable(failure): String(localized: "No response: \(failure.message)")
        }
    }

    var style: NoticeStyle {
        switch self {
        case .running: .success
        case .checking: .info
        case .notRegistered, .requiresApproval, .versionMismatch, .unreachable: .warning
        }
    }
}

extension AppInternal.HelperChannelFailure {
    var message: String {
        switch self {
        case .appNotSigned: String(localized: "The app is not signed with a team certificate.")
        case let .connection(description): String(localized: "Helper connection failed: \(description)")
        case .connectionLost: String(localized: "The Helper connection was lost before the request was sent.")
        case .noReply: String(localized: "The Helper is not responding.")
        case let .unreadableReply(description): String(localized: "Could not read the Helper response: \(description)")
        }
    }
}

extension AppInternal.RegistrationError {
    /// Why registering the Helper failed.
    var helperMessage: String {
        switch self {
        case .notInstalledCopy:
            String(localized: "The Helper can only be registered from the app installed in /Applications (make install).")
        case let .failed(description):
            description
        }
    }

    /// Why changing "Open at login" failed.
    var loginItemMessage: String {
        switch self {
        case .notInstalledCopy:
            String(localized: "Launch at Login can only be set from the app installed in /Applications.")
        case let .failed(description):
            description
        }
    }
}

extension AppInternal.UntestedVersion {
    var message: String {
        String(localized: "This isn't a tested version (\(testedSeries).x). Its output may differ and connections may fail.")
    }
}

extension AppInternal.ClientProblem {
    var message: String {
        switch kind {
        case .notFound:
            String(localized: "openfortivpn not found at \(path); install it with: brew install openfortivpn")
        case .outsideAllowedLocation:
            String(localized: "openfortivpn resolves outside the Homebrew Cellar (/opt/homebrew/Cellar/openfortivpn/): \(path)")
        case .notRegularFile:
            String(localized: "openfortivpn is not a regular file: \(path)")
        case let .writableByOthers(mode):
            String(localized: "openfortivpn is writable by other users (mode \(String(mode, radix: 8))); refusing to run it: \(path)")
        case .notExecutable:
            String(localized: "openfortivpn is not executable: \(path)")
        }
    }
}
