import Foundation
import os
import FortiCore
import FortiShared

/// `OTPSessionService` over Gmail IMAP (DESIGN.md §5.3): one IMAP connection per connection attempt.
///
/// `start` hands any previous attempt a graceful close (it logs out in the background); every other
/// call is ignored unless it names the current attempt.
final class GmailOTPSession: OTPSessionService {
    var onEvent: (@MainActor (OTPSessionEvent) -> Void)?

    private let endpoint: IMAPEndpoint
    private var current: GmailOTPAttempt?

    init(endpoint: IMAPEndpoint = .gmail) {
        self.endpoint = endpoint
    }

    func start(attemptID: AttemptID, account: GmailAccount) {
        guard current?.attemptID != attemptID else { return }
        current?.end(.close(checkJunk: false))
        let attempt = GmailOTPAttempt(attemptID: attemptID, account: account, endpoint: endpoint) { [weak self] event in
            self?.onEvent?(event)
        }
        current = attempt
        attempt.start()
    }

    func beginSearch(attemptID: AttemptID, promptedAt: Date) {
        attempt(attemptID)?.beginSearch(promptedAt: promptedAt)
    }

    func finish(attemptID: AttemptID) {
        attempt(attemptID)?.end(.finish)
    }

    func close(attemptID: AttemptID, checkJunk: Bool) {
        attempt(attemptID)?.end(.close(checkJunk: checkJunk))
    }

    private func attempt(_ attemptID: AttemptID) -> GmailOTPAttempt? {
        current?.attemptID == attemptID ? current : nil
    }
}

/// One attempt's IMAP connection: log in → snapshot `\All` → trash stale codes → `.ready` → IDLE;
/// after `beginSearch`, search on every EXISTS and every 5 s until a code matches; then wait for
/// `end` to trash the used messages (finish) or to stop, optionally checking `\Junk` (close).
///
/// Events: `.ready`, `.codeFound` and `.failed` only before `end`; `.failed` never after `.codeFound`;
/// `.foundInJunk` only from the close-time junk check.
private final class GmailOTPAttempt {
    enum Ending {
        case finish
        case close(checkJunk: Bool)
    }

    private nonisolated enum SessionError: LocalizedError {
        case allMailMissing
        case snapshotMissing
        case uidValidityChanged

        var errorDescription: String? {
            switch self {
            case .allMailMissing: "LIST returned no \\All mailbox"
            case .snapshotMissing: "SELECT response has no UIDVALIDITY/UIDNEXT"
            case .uidValidityChanged: "UIDVALIDITY of the \\All mailbox changed"
            }
        }
    }

    /// Fallback search period while looking for the code, in case an IDLE notification is missed or
    /// late. Short on purpose: it only runs during the ≤ 50 s OTP window.
    private static let searchInterval: Duration = .seconds(1)
    /// IDLE refresh period while not searching (servers end IDLE after ~30 min of silence).
    private static let idleRefresh: Duration = .seconds(300)
    /// `UID SEARCH SUBJECT` term for the spam-folder check.
    private static let junkSearchTerm = "AuthCode"
    /// Newest spam messages the junk check inspects.
    private static let junkFetchLimit = 5

    let attemptID: AttemptID
    private let account: GmailAccount
    private let transport: IMAPTransport
    private let emit: (OTPSessionEvent) -> Void
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "otp")
    private var mailboxes: IMAPSpecialMailboxes?
    private var promptedAt: Date?
    private var selection: OTPSelection?
    private var ending: Ending?

    init(attemptID: AttemptID, account: GmailAccount, endpoint: IMAPEndpoint, emit: @escaping (OTPSessionEvent) -> Void) {
        self.attemptID = attemptID
        self.account = account
        self.transport = IMAPTransport(endpoint: endpoint)
        self.emit = emit
    }

    func start() {
        Task { await run() }
    }

    func beginSearch(promptedAt: Date) {
        guard self.promptedAt == nil, ending == nil else { return }
        self.promptedAt = promptedAt
        transport.interruptIdle()
    }

    func end(_ ending: Ending) {
        guard self.ending == nil else { return }
        self.ending = ending
        transport.interruptIdle()
    }

    private func run() async {
        do {
            let snapshot = try await prepare()
            if ending == nil {
                logger.info("Gmail session ready (UIDVALIDITY \(snapshot.uidValidity), UIDNEXT \(snapshot.uidNext))")
                emit(.ready(attemptID))
            }
            try await watch(snapshot)
        } catch {
            if ending == nil, selection == nil {
                let failure: OTPSessionFailure = error is GmailLoginRejected ? .authentication : .network
                logger.error("Gmail session failed (\(failure.rawValue, privacy: .public)): \(error.localizedDescription, privacy: .public)")
                emit(.failed(attemptID, failure))
            } else {
                logger.notice("Gmail session lost after it was settled: \(error.localizedDescription, privacy: .public)")
            }
            await transport.close()
            return
        }
        await wrapUp()
    }

    /// Logs in, finds the special mailboxes, SELECTs `\All`, snapshots it and trashes AuthCode
    /// messages that already arrived after the snapshot.
    private func prepare() async throws -> MailboxSnapshot {
        try await transport.logIn(account)
        let mailboxes = try await transport.specialMailboxes()
        self.mailboxes = mailboxes
        guard let all = mailboxes.all else { throw SessionError.allMailMissing }
        let selected = try await transport.execute(.select(mailbox: all.name))
        guard let snapshot = MailboxSnapshot(selectResponses: selected) else { throw SessionError.snapshotMissing }
        let recent = try await messages(since: snapshot)
        // An attempt that already ended (e.g. superseded) must not sweep: a newer attempt's code may be there.
        if ending == nil {
            try await moveToTrash(OTPMatcher.staleAuthCodeUIDs(
                snapshot: snapshot,
                currentUIDValidity: snapshot.uidValidity,
                messages: recent
            ))
        }
        return snapshot
    }

    /// Idles until `end`. Once `beginSearch` was called and no code matched yet, searches whenever
    /// IDLE reports new messages and at least every `searchInterval`.
    private func watch(_ snapshot: MailboxSnapshot) async throws {
        var trigger = SearchTrigger.prompt
        var stats = SearchStats()
        while ending == nil {
            if let promptedAt, selection == nil {
                stats.searches += 1
                try await search(snapshot, promptedAt: promptedAt, trigger: trigger, stats: stats)
            }
            let searching = promptedAt != nil && selection == nil
            let updates = try await transport.idle(for: searching ? Self.searchInterval : Self.idleRefresh)
            try verifyUIDValidity(updates, snapshot)
            let notified = updates.contains { if case .exists = $0 { true } else { false } }
            if searching, notified { stats.notifications += 1 }
            trigger = notified ? .idleNotification : .poll
        }
    }

    /// How the OTP search went, for the Log tab's timing line.
    private struct SearchStats {
        var searches = 0
        var notifications = 0
    }

    /// What made the session look for the code (reported in the Log tab's timing line).
    private enum SearchTrigger: String {
        case prompt = "search right after the prompt"
        case idleNotification = "IDLE notification"
        case poll = "periodic search"
    }

    private func search(_ snapshot: MailboxSnapshot, promptedAt: Date, trigger: SearchTrigger, stats: SearchStats) async throws {
        let messages = try await messages(since: snapshot)
        guard let selection = OTPMatcher.select(
            snapshot: snapshot,
            currentUIDValidity: snapshot.uidValidity,
            otpPromptedAt: promptedAt,
            messages: messages
        ) else { return }
        self.selection = selection
        logger.info("OTP message matched (UID \(selection.uid), \(selection.allCandidateUIDs.count) candidate(s))")
        if ending == nil {
            emit(.codeFound(attemptID, code: selection.code))
            if let arrival = messages.first(where: { $0.uid == selection.uid })?.internalDate {
                emit(.note(attemptID, Self.timingNote(promptedAt: promptedAt, arrivedAt: arrival, foundAt: Date(), trigger: trigger, stats: stats)))
            }
        }
    }

    /// e.g. "‹OTP mail: in Gmail 5s after the prompt · detected 1s after arrival (IDLE notification · 3 searches · 1 IDLE notification)›".
    /// INTERNALDATE has one-second resolution, so both spans are rounded and clamped at zero.
    private static func timingNote(
        promptedAt: Date, arrivedAt: Date, foundAt: Date, trigger: SearchTrigger, stats: SearchStats
    ) -> String {
        let delivery = max(0, Int(arrivedAt.timeIntervalSince(promptedAt).rounded()))
        let detection = max(0, Int(foundAt.timeIntervalSince(arrivedAt).rounded()))
        return "‹OTP mail: in Gmail \(delivery)s after the prompt · detected \(detection)s after arrival "
            + "(\(trigger.rawValue) · \(stats.searches) searches · \(stats.notifications) IDLE notifications)›"
    }

    /// Headers of every message at/after the snapshot (`UID SEARCH UID n:*` also returns the newest
    /// older message, so results are filtered here too). Fails if `\All`'s UIDVALIDITY changed.
    private func messages(since snapshot: MailboxSnapshot) async throws -> [FetchedHeaderMessage] {
        let search = try await transport.execute(.uidSearch(.from(snapshot.uidNext)))
        try verifyUIDValidity(search, snapshot)
        guard let uids = IMAPUIDSet(search.searchedUIDs.filter { $0 >= snapshot.uidNext }) else { return [] }
        let fetch = try await transport.execute(.uidFetchHeaders(uids))
        try verifyUIDValidity(fetch, snapshot)
        return fetch.headerMessages
    }

    /// UIDs are meaningless after a UIDVALIDITY change, so no code could ever be matched: fail fast
    /// (as a network problem) instead of waiting for the OTP deadline.
    private func verifyUIDValidity(_ responses: [IMAPResponse], _ snapshot: MailboxSnapshot) throws {
        guard responses.uidValidities.allSatisfy({ $0 == snapshot.uidValidity }) else {
            throw SessionError.uidValidityChanged
        }
    }

    /// `UID MOVE` to `\Trash`. A refused MOVE or a missing `\Trash` is logged, not fatal.
    private func moveToTrash(_ uids: [UInt32]) async throws {
        guard let set = IMAPUIDSet(uids) else { return }
        guard let trash = mailboxes?.trash else {
            logger.warning("No \\Trash mailbox; left \(uids.count) AuthCode message(s) in place")
            return
        }
        do {
            _ = try await transport.execute(.uidMove(set, mailbox: trash.name))
            logger.info("Moved \(uids.count) AuthCode message(s) to Trash")
        } catch IMAPTransportError.rejected {
            logger.warning("Could not move \(uids.count) AuthCode message(s) to Trash")
        }
    }

    /// Runs the requested ending, then logs out. Errors are only logged: the attempt has moved on.
    private func wrapUp() async {
        do {
            switch ending {
            case .finish?:
                try await moveToTrash(selection?.allCandidateUIDs ?? [])
            case .close(checkJunk: true)?:
                try await checkJunk()
            case .close(checkJunk: false)?, nil:
                break
            }
        } catch {
            logger.error("Gmail session wrap-up failed: \(error.localizedDescription, privacy: .public)")
        }
        await transport.logout()
    }

    /// After the OTP deadline: looks at the newest AuthCode messages in `\Junk` once and reports
    /// `.foundInJunk` when one arrived after the prompt.
    private func checkJunk() async throws {
        guard let promptedAt, let junk = mailboxes?.junk else { return }
        _ = try await transport.execute(.examine(mailbox: junk.name))
        let found = try await transport.execute(.uidSearchSubject(Self.junkSearchTerm)).searchedUIDs
        guard let newest = IMAPUIDSet(found.sorted().suffix(Self.junkFetchLimit)) else { return }
        let messages = try await transport.execute(.uidFetchHeaders(newest)).headerMessages
        guard OTPMatcher.containsFreshAuthCode(otpPromptedAt: promptedAt, messages: messages) else { return }
        logger.notice("Fresh AuthCode message found in the spam folder")
        emit(.foundInJunk(attemptID))
    }
}
