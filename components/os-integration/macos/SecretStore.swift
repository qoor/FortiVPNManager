import Foundation
import Security

/// The app's secrets as generic passwords in the login Keychain, under the service and account each
/// item names. The file-based login keychain: the data-protection keychain would need entitlements
/// and a provisioning profile.
nonisolated enum SecretStore {
    static func read(_ item: AppInternal.SecretItem) -> AppInternal.PortResponse {
        var query = query(for: item)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        switch status {
        case errSecSuccess:
            return .secret(value: (result as? Data).flatMap { String(data: $0, encoding: .utf8) })
        case errSecItemNotFound:
            return .secret(value: nil)
        default:
            return failed(status)
        }
    }

    /// Asks for the attributes only, never the secret, so the system asks the user for nothing.
    static func exists(_ item: AppInternal.SecretItem) -> AppInternal.PortResponse {
        var query = query(for: item)
        query[kSecReturnAttributes as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        let status = SecItemCopyMatching(query as CFDictionary, nil)
        switch status {
        case errSecSuccess: return .hasSecret(exists: true)
        case errSecItemNotFound: return .hasSecret(exists: false)
        default: return failed(status)
        }
    }

    static func write(_ value: String, to item: AppInternal.SecretItem) -> AppInternal.PortResponse {
        let data = Data(value.utf8)
        let query = query(for: item)
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        guard status == errSecItemNotFound else {
            return status == errSecSuccess ? .secretChanged : failed(status)
        }
        var newItem = query
        newItem[kSecValueData as String] = data
        let addStatus = SecItemAdd(newItem as CFDictionary, nil)
        return addStatus == errSecSuccess ? .secretChanged : failed(addStatus)
    }

    static func delete(_ item: AppInternal.SecretItem) -> AppInternal.PortResponse {
        let status = SecItemDelete(query(for: item) as CFDictionary)
        return status == errSecSuccess || status == errSecItemNotFound ? .secretChanged : failed(status)
    }

    /// The attributes that pick out `item`.
    static func query(for item: AppInternal.SecretItem) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: item.keychainService,
            kSecAttrAccount as String: item.keychainAccount,
        ]
    }

    private static func failed(_ status: OSStatus) -> AppInternal.PortResponse {
        let description = SecCopyErrorMessageString(status, nil) as String? ?? String(localized: "Keychain error \(status)")
        return .secretStoreFailed(failure: AppInternal.SecretStoreFailure(description: description))
    }
}
