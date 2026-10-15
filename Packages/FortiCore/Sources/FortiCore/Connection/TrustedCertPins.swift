import Foundation

/// Rules for `vpn.trusted_certs` (§2, §5.4): lowercase 64-hex digests, newest first, at most four.
public enum TrustedCertPins {
    public static let limit = 4

    /// `true` for a lowercase hex SHA-256 digest (`^[0-9a-f]{64}$`).
    public static func isValidDigest(_ digest: String) -> Bool {
        digest.utf8.count == 64 && digest.utf8.allSatisfy { (0x30...0x39).contains($0) || (0x61...0x66).contains($0) }
    }

    /// Lowercases, drops invalid entries and duplicates, keeping first-seen order.
    public static func normalized(_ digests: [String]) -> [String] {
        var seen = Set<String>()
        var result: [String] = []
        for raw in digests {
            let digest = raw.lowercased()
            guard isValidDigest(digest), seen.insert(digest).inserted else { continue }
            result.append(digest)
        }
        return result
    }

    /// Puts `newDigests` in front of `existing` (moving duplicates forward) and keeps the newest `limit`.
    public static func merging(_ newDigests: [String], into existing: [String]) -> [String] {
        Array(normalized(newDigests + existing).prefix(limit))
    }
}
