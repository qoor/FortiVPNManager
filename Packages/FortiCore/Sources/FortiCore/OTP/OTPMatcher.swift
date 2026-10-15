import Foundation

/// `UIDVALIDITY`/`UIDNEXT` of the `\All` mailbox, taken right after SELECT and before connecting.
public struct MailboxSnapshot: Sendable, Hashable, Codable {
    public var uidValidity: UInt32
    public var uidNext: UInt32

    public init(uidValidity: UInt32, uidNext: UInt32) {
        self.uidValidity = uidValidity
        self.uidNext = uidNext
    }
}

/// A message fetched with `UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT DATE)]`.
public struct FetchedHeaderMessage: Sendable, Hashable {
    public var uid: UInt32
    /// Server receive time (`INTERNALDATE`).
    public var internalDate: Date
    /// Raw header block bytes.
    public var headerBlock: Data

    public init(uid: UInt32, internalDate: Date, headerBlock: Data) {
        self.uid = uid
        self.internalDate = internalDate
        self.headerBlock = headerBlock
    }

    /// Converts a FETCH reply; `nil` unless it carries UID, INTERNALDATE and a header block.
    public init?(_ fetch: IMAPFetchResponse) {
        guard let uid = fetch.uid, let date = fetch.internalDate, let header = fetch.headerBytes else { return nil }
        self.init(uid: uid, internalDate: date, headerBlock: header)
    }

    /// The Subject field, unfolded and RFC 2047-decoded; `nil` if absent.
    public var subject: String? {
        MailHeader.fieldValue("Subject", in: headerBlock)
    }
}

/// The OTP to submit plus every AuthCode mail to move to `\Trash` afterwards.
public struct OTPSelection: Sendable, Hashable {
    public var code: String
    /// UID of the message the code came from.
    public var uid: UInt32
    /// Ascending UIDs of every message at/after the snapshot whose subject carries a code
    /// (including stale ones outside the date window).
    public var allCandidateUIDs: [UInt32]

    public init(code: String, uid: UInt32, allCandidateUIDs: [UInt32]) {
        self.code = code
        self.uid = uid
        self.allCandidateUIDs = allCandidateUIDs
    }
}

/// Picks the FortiGate email OTP from fetched headers (DESIGN.md §5.3). Pure; time is injected.
public enum OTPMatcher {
    /// Subject marker. Equivalent to `AuthCode:\s*(\d{6,8})` with ASCII digits and no further digit
    /// after the code. To be confirmed against real mails from Phase 0b.
    public static let subjectMarker = "AuthCode:"
    public static let codeLengths = 6...8
    /// A message counts if `INTERNALDATE >= otpPromptedAt - arrivalTolerance`
    /// (INTERNALDATE has one-second resolution and server/client clocks differ slightly).
    public static let arrivalTolerance: TimeInterval = 5

    /// Extracts the code from a decoded subject, e.g. `"AuthCode: 123456"` → `"123456"`.
    public static func code(inSubject subject: String) -> String? {
        var searchStart = subject.startIndex
        while let marker = subject.range(of: subjectMarker, options: .literal, range: searchStart..<subject.endIndex) {
            var index = marker.upperBound
            while index < subject.endIndex, subject[index].isWhitespace { index = subject.index(after: index) }
            let digitsStart = index
            while index < subject.endIndex, let ascii = subject[index].asciiValue, (0x30...0x39).contains(ascii) {
                index = subject.index(after: index)
            }
            let digits = subject[digitsStart..<index]
            if codeLengths.contains(digits.count) { return String(digits) }
            searchStart = marker.upperBound
        }
        return nil
    }

    /// Chooses the OTP once `otpPrompted` arrives. A message qualifies when all of these hold:
    /// - `uid >= snapshot.uidNext` (checked here because `UID SEARCH UID n:*` also returns the
    ///   newest message even when its UID is below `n`),
    /// - `currentUIDValidity == snapshot.uidValidity` (otherwise UIDs are meaningless → `nil`),
    /// - `internalDate >= otpPromptedAt - arrivalTolerance`,
    /// - the subject carries a code.
    ///
    /// The highest qualifying UID wins. Returns `nil` when nothing qualifies.
    public static func select(
        snapshot: MailboxSnapshot,
        currentUIDValidity: UInt32,
        otpPromptedAt: Date,
        messages: [FetchedHeaderMessage]
    ) -> OTPSelection? {
        guard currentUIDValidity == snapshot.uidValidity else { return nil }
        let earliest = otpPromptedAt.addingTimeInterval(-arrivalTolerance)
        var candidates = Set<UInt32>()
        var best: (uid: UInt32, code: String)?
        for message in messages where message.uid >= snapshot.uidNext {
            guard let subject = message.subject, let code = code(inSubject: subject) else { continue }
            candidates.insert(message.uid)
            if message.internalDate >= earliest, message.uid > (best?.uid ?? 0) {
                best = (message.uid, code)
            }
        }
        guard let best else { return nil }
        return OTPSelection(code: best.code, uid: best.uid, allCandidateUIDs: candidates.sorted())
    }

    /// UIDs to move to `\Trash` before connecting: every message at/after the snapshot whose subject
    /// carries a code, regardless of date. Empty if `UIDVALIDITY` changed.
    public static func staleAuthCodeUIDs(
        snapshot: MailboxSnapshot,
        currentUIDValidity: UInt32,
        messages: [FetchedHeaderMessage]
    ) -> [UInt32] {
        guard currentUIDValidity == snapshot.uidValidity else { return [] }
        let uids = messages.lazy
            .filter { $0.uid >= snapshot.uidNext }
            .filter { $0.subject.flatMap(code(inSubject:)) != nil }
            .map(\.uid)
        return Set(uids).sorted()
    }

    /// Whether any message carries a code and arrived inside the window — used for the one-off
    /// `\Junk` check after the 50 s deadline, to explain the failure.
    public static func containsFreshAuthCode(otpPromptedAt: Date, messages: [FetchedHeaderMessage]) -> Bool {
        let earliest = otpPromptedAt.addingTimeInterval(-arrivalTolerance)
        return messages.contains { message in
            message.internalDate >= earliest && message.subject.flatMap(code(inSubject:)) != nil
        }
    }
}
