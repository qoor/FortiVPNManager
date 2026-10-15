import Foundation

/// The OTP to submit plus every AuthCode message to trash afterwards (Gmail API variant of `OTPSelection`).
public struct GmailOTPSelection: Sendable, Hashable {
    public var code: String
    /// ID of the message the code came from.
    public var messageID: String
    /// When Gmail received that message.
    public var internalDate: Date
    /// Every considered message whose subject carries a code (including stale ones outside the date
    /// window), oldest first.
    public var allCandidateIDs: [String]

    public init(code: String, messageID: String, internalDate: Date, allCandidateIDs: [String]) {
        self.code = code
        self.messageID = messageID
        self.internalDate = internalDate
        self.allCandidateIDs = allCandidateIDs
    }
}

/// OTP matching over Gmail API message metadata (DESIGN.md §5.3 rules, same subject pattern and
/// date window as the IMAP path).
extension OTPMatcher {
    /// Messages with these labels never count: IMAP's `\All` excludes Spam and Trash as well, and a
    /// draft is not a delivered OTP. Spam is only looked at by the junk check.
    public static let excludedGmailLabels: Set<String> = ["SPAM", "TRASH", "DRAFT"]

    /// Chooses the OTP once `otpPrompted` arrived. A message qualifies when:
    /// - none of its labels is in ``excludedGmailLabels``,
    /// - `internalDate >= otpPromptedAt - arrivalTolerance`,
    /// - the subject carries a code.
    ///
    /// The newest qualifying message wins (by `internalDate`, then by ID). Candidates for trashing are
    /// every non-excluded message whose subject carries a code and, when `candidatesNotBefore` is set,
    /// whose `internalDate` is not earlier than it (used when the session could not rely on a history
    /// snapshot and searched instead). Returns `nil` when nothing qualifies.
    public static func select(
        otpPromptedAt: Date,
        messages: [GmailMessageMetadata],
        candidatesNotBefore: Date? = nil
    ) -> GmailOTPSelection? {
        let earliest = otpPromptedAt.addingTimeInterval(-arrivalTolerance)
        var candidates: [String: Date] = [:]
        var best: (message: GmailMessageMetadata, date: Date, code: String)?
        for message in messages where excludedGmailLabels.isDisjoint(with: message.labelIds) {
            guard let subject = message.subject, let code = code(inSubject: subject) else { continue }
            if let candidatesNotBefore {
                guard let date = message.internalDate, date >= candidatesNotBefore else { continue }
            }
            candidates[message.id] = message.internalDate ?? .distantPast
            guard let date = message.internalDate, date >= earliest else { continue }
            if let current = best, !isNewer(message.id, date, than: current.message.id, current.date) { continue }
            best = (message, date, code)
        }
        guard let best else { return nil }
        let ordered = candidates.sorted { lhs, rhs in
            lhs.value != rhs.value ? lhs.value < rhs.value : isOrderedID(lhs.key, before: rhs.key)
        }
        return GmailOTPSelection(
            code: best.code, messageID: best.message.id, internalDate: best.date, allCandidateIDs: ordered.map(\.key)
        )
    }

    /// Whether any message carries a code and arrived inside the window, regardless of labels — used
    /// for the one-off spam check after the 50 s deadline.
    public static func containsFreshAuthCode(otpPromptedAt: Date, messages: [GmailMessageMetadata]) -> Bool {
        let earliest = otpPromptedAt.addingTimeInterval(-arrivalTolerance)
        return messages.contains { message in
            guard let date = message.internalDate, date >= earliest else { return false }
            return message.subject.flatMap(code(inSubject:)) != nil
        }
    }

    private static func isNewer(_ id: String, _ date: Date, than otherID: String, _ otherDate: Date) -> Bool {
        date != otherDate ? date > otherDate : isOrderedID(otherID, before: id)
    }

    /// Gmail message IDs are hexadecimal and grow over time: shorter first, then lexicographic.
    private static func isOrderedID(_ lhs: String, before rhs: String) -> Bool {
        lhs.count != rhs.count ? lhs.count < rhs.count : lhs < rhs
    }
}
