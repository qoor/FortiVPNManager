/// Classifies a tagged `NO` to AUTHENTICATE (or LOGIN).
public enum IMAPLoginRejection {
    /// Whether a tagged `NO` to AUTHENTICATE means the account cannot log in this way, so retrying
    /// with the same credentials cannot help: `[AUTHENTICATIONFAILED]` (wrong or revoked app
    /// password), an `[ALERT]` or Gmail's `[WEBALERT url]` demanding a web sign-in, IMAP disabled for
    /// the account or domain, or any other refusal.
    ///
    /// Only refusals that clearly mean "later" are transient: `[UNAVAILABLE]`, `[LIMIT]`, `[INUSE]`,
    /// `[SERVERBUG]`, or an `[ALERT]` about simultaneous connections, bandwidth limits or a
    /// temporary problem.
    public static func isCredentialRejection(code: IMAPResponseCode?, text: String) -> Bool {
        switch code {
        case .unavailable?, .limit?, .other(name: "INUSE", _)?, .other(name: "SERVERBUG", _)?:
            return false
        case .alert?:
            let lowered = text.lowercased()
            return !transientAlertKeywords.contains { lowered.contains($0) }
        default:
            return true
        }
    }

    private static let transientAlertKeywords = ["simultaneous", "bandwidth", "limit", "temporar", "try again"]
}
