import Foundation

extension AppInternal.AccountValueProblem {
    var message: String {
        switch self {
        case .invalidHost:
            String(localized: "Enter only a host name or IPv4 address as the server address (no https://, :port, or /). Enter the port in the Port field.")
        case let .tooLong(field, maxBytes):
            String(localized: "\(field.name) is too long (at most \(maxBytes) bytes).")
        case let .leadingOrTrailingSpace(field):
            String(localized: "\(field.name) can't start or end with a space.")
        case let .disallowedCharacters(field):
            String(localized: "\(field.name) contains characters that aren't allowed.")
        }
    }
}

extension AppInternal.AccountField {
    /// The field's label; the realm, which has no label of its own in these sentences, by its key.
    fileprivate var name: String {
        switch self {
        case .host: String(localized: "Server address")
        case .username: String(localized: "Username")
        case .realm: "realm"
        }
    }
}

extension AppInternal.AccountSaveError {
    var message: String {
        switch self {
        case .invalidPort:
            String(localized: "The port must be a number from 1 to 65535.")
        case .passwordNeedsAccount:
            String(localized: "Enter the server address and username to save the VPN password.")
        case .appPasswordNeedsAddress:
            String(localized: "Enter the Gmail address to save the app password.")
        case let .accountValue(problem):
            problem.message
        case let .invalidVpnPassword(maxBytes):
            String(localized: "The VPN password can't contain line breaks and must be at most \(maxBytes) bytes.")
        case let .config(error):
            error.message
        case let .secretStore(failure):
            String(localized: "Couldn't save the password to Keychain: \(failure.description)")
        }
    }
}

extension AppInternal.ImportError {
    var message: String {
        switch self {
        case let .sourceNotFound(path):
            String(localized: "openfortivpn config file not found: \(path)")
        case let .unreadable(path, problem):
            String(localized: "Could not read the openfortivpn config file (\(path)): \(problem.importReason)")
        case let .missingHost(path):
            String(localized: "The openfortivpn config file has no host: \(path)")
        case let .config(error):
            error.message
        }
    }
}

extension AppInternal.ImportReport {
    /// "Server: vpn.example.com:443 · Username: alice · Server certificates: 1".
    var summary: String {
        var parts = [String(localized: "Server: \(host):\(port)")]
        if !username.isEmpty { parts.append(String(localized: "Username: \(username)")) }
        if !realm.isEmpty { parts.append(String(localized: "Realm: \(realm)")) }
        parts.append(String(localized: "Server certificates: \(trustedCerts)"))
        return parts.joined(separator: " · ")
    }

    var unknownKeysMessage: String? {
        guard !unknownKeys.isEmpty else { return nil }
        return String(localized: """
            Ignored unknown keys: \(unknownKeys.joined(separator: ", ")). \
            openfortivpn doesn't read the settings after such keys.
            """)
    }

    var ignoredKeysMessage: String? {
        guard !ignoredKeys.isEmpty else { return nil }
        return String(localized: "Options not imported: \(ignoredKeys.joined(separator: ", "))")
    }

    var unreadableLinesMessage: String? {
        guard !unreadableLines.isEmpty else { return nil }
        return String(localized: "Unreadable lines: \(unreadableLines.map(String.init).joined(separator: ", "))")
    }

    /// What became of the password, and how it is to be shown.
    var passwordStatus: (text: String, style: NoticeStyle) {
        switch password {
        case .saved: (String(localized: "Saved the password to Keychain."), .success)
        case .notSavedWithoutUsername:
            (String(localized: "The password was not saved to the Keychain because there is no username."), .warning)
        case let .notSaved(failure): (failure.description, .warning)
        case nil: (String(localized: "The file has no password. Enter the VPN password above."), .info)
        }
    }
}
