import Foundation
import os
import FortiCore
import FortiShared

/// `OTPSessionService` over the Gmail REST API: one polling loop per connection attempt. Its snapshot
/// is the `users.getProfile` history ID.
final class GmailAPIOTPSession: OTPSessionService {
    var onEvent: (@MainActor (OTPSessionEvent) -> Void)?

    private let tokens: GoogleOAuthService
    private var current: GmailAPIOTPAttempt?

    init(tokens: GoogleOAuthService) {
        self.tokens = tokens
    }

    func start(attemptID: AttemptID, account: GmailAccount) {
        guard current?.attemptID != attemptID else { return }
        current?.end(.close(checkJunk: false))
        let attempt = GmailAPIOTPAttempt(attemptID: attemptID, address: account.address, tokens: tokens) {
            [weak self] event in
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

    /// Why the attempt reported `.failed` (plain English, no secrets), for `OTPSessionRouter`'s notes.
    func failureReason(attemptID: AttemptID) -> String? {
        attempt(attemptID)?.failureReason
    }

    private func attempt(_ attemptID: AttemptID) -> GmailAPIOTPAttempt? {
        current?.attemptID == attemptID ? current : nil
    }
}

/// One attempt's polling loop.
private final class GmailAPIOTPAttempt {
    enum Ending {
        case finish
        case close(checkJunk: Bool)
    }

    private enum SessionError: Error {
        case token(GoogleAccessTokenError)
        case api(GmailAPIError)
        case accountMismatch

        /// Authorization problems end the attempt as `.authentication` (no retry can help; the router
        /// may fall back to IMAP); transport and server trouble as `.network`.
        var failure: OTPSessionFailure {
            switch self {
            case let .token(error): error.isAuthenticationFailure ? .authentication : .network
            case .api(.unauthorized), .api(.forbidden), .accountMismatch: .authentication
            case .api: .network
            }
        }

        /// Worth another poll before giving up.
        var isTransient: Bool {
            switch self {
            case .token(.network), .token(.server), .api(.network), .api(.server), .api(.rateLimited): true
            default: false
            }
        }

        var logDescription: String {
            switch self {
            case let .token(error): error.logDescription
            case .api(.unauthorized): "Gmail API rejected the access token"
            case let .api(.forbidden(reason)): "Gmail API access forbidden (\(reason))"
            case .api(.notFound): "Gmail API: not found"
            case .api(.rateLimited): "Gmail API rate limit"
            case let .api(.network(detail)): "network error: \(detail)"
            case let .api(.server(status)): "Gmail API error HTTP \(status)"
            case .accountMismatch: "the Google authorization belongs to another account"
            }
        }
    }

    static let pollInterval: Duration = .milliseconds(500)
    /// Wait between wake-ups while nothing is to be done (before the prompt, after a match).
    static let idleWait: Duration = .seconds(300)
    /// Consecutive failed polls tolerated before the attempt fails as a network problem.
    static let maxConsecutivePollFailures = 3
    static let rateLimitBackoff: Duration = .seconds(2)
    /// History pages read per poll; more would mean a very busy mailbox.
    static let maxHistoryPages = 10
    /// Search used when the history snapshot is unusable (`history.list` 404).
    static let fallbackQuery = "subject:AuthCode newer_than:1d"
    static let fallbackLimit = 10
    static let junkQuery = "in:spam subject:AuthCode newer_than:1h"
    static let junkLimit = 5

    let attemptID: AttemptID
    private(set) var failureReason: String?
    private let address: String
    private let tokens: GoogleOAuthService
    private let client = GmailAPIClient()
    private let emit: (OTPSessionEvent) -> Void
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "otp")
    private let wake = WakeSignal()
    private var promptedAt: Date?
    private var ending: Ending?
    private var selection: GmailOTPSelection?
    /// History ID to list from; `nil` once the session searches instead.
    private var cursor: String?
    /// When the profile snapshot was taken; bounds the search fallback's candidates.
    private var startedAt = Date()
    /// IDs whose metadata was fetched (or that no longer exist).
    private var seen: Set<String> = []
    /// Reported by history or search but not fetched yet.
    private var pending: [GmailMessageRef] = []
    private var messages: [GmailMessageMetadata] = []
    private var polls = 0

    init(attemptID: AttemptID, address: String, tokens: GoogleOAuthService, emit: @escaping (OTPSessionEvent) -> Void) {
        self.attemptID = attemptID
        self.address = address
        self.tokens = tokens
        self.emit = emit
    }

    func start() {
        Task { await run() }
    }

    func beginSearch(promptedAt: Date) {
        guard self.promptedAt == nil, ending == nil else { return }
        self.promptedAt = promptedAt
        wake.signal()
    }

    func end(_ ending: Ending) {
        guard self.ending == nil else { return }
        self.ending = ending
        wake.signal()
    }

    // MARK: - Flow

    private func run() async {
        do throws(SessionError) {
            let profile = try await call { [client] token async throws(GmailAPIError) in
                try await client.profile(accessToken: token)
            }
            guard profile.emailAddress.caseInsensitiveCompare(address) == .orderedSame else {
                throw SessionError.accountMismatch
            }
            startedAt = Date()
            if GmailAPI.isValidHistoryID(profile.historyId) {
                cursor = profile.historyId
            } else {
                logger.notice("Gmail API profile has no usable history ID; searching instead")
            }
            if ending == nil {
                logger.info("Gmail API session ready")
                emit(.ready(attemptID))
            }
            try await watch()
        } catch {
            guard ending != nil || selection != nil else {
                failureReason = error.logDescription
                logger.error("Gmail API session failed (\(error.failure.rawValue, privacy: .public)): \(error.logDescription, privacy: .public)")
                emit(.failed(attemptID, error.failure))
                return
            }
            logger.notice("Gmail API session error after it was settled: \(error.logDescription, privacy: .public)")
        }
        await wrapUp()
    }

    /// Waits for the prompt, polls until a code matches (a poll starts every `pollInterval`, or right
    /// after the previous one when that took longer), then waits for `end`.
    private func watch() async throws(SessionError) {
        var failures = 0
        while ending == nil {
            guard let promptedAt, selection == nil else {
                await wake.wait(upTo: Self.idleWait)
                continue
            }
            let pollStarted = ContinuousClock.now
            do {
                polls += 1
                try await poll(promptedAt: promptedAt)
                failures = 0
            } catch where error.isTransient {
                failures += 1
                logger.notice("Gmail API poll failed (\(failures)/\(Self.maxConsecutivePollFailures)): \(error.logDescription, privacy: .public)")
                if failures >= Self.maxConsecutivePollFailures { throw error }
                if case .api(.rateLimited) = error {
                    await wake.wait(upTo: Self.rateLimitBackoff)
                    continue
                }
            }
            guard selection == nil, ending == nil else { continue }
            let remaining = Self.pollInterval - (ContinuousClock.now - pollStarted)
            if remaining > .zero {
                await wake.wait(upTo: remaining)
            } else {
                await Task.yield()
            }
        }
    }

    private func poll(promptedAt: Date) async throws(SessionError) {
        let refs: [GmailMessageRef]
        if cursor != nil {
            do {
                refs = try await historyMessages()
            } catch .api(.notFound) {
                logger.notice("Gmail history ID is no longer valid; searching instead")
                cursor = nil
                refs = try await searchedMessages()
            }
        } else {
            refs = try await searchedMessages()
        }
        for ref in refs where !seen.contains(ref.id) && !pending.contains(where: { $0.id == ref.id }) {
            if !OTPMatcher.excludedGmailLabels.isDisjoint(with: ref.labelIds) {
                seen.insert(ref.id)
            } else {
                pending.append(ref)
            }
        }
        let fetchError = await fetchPending()
        let notBefore = cursor == nil ? startedAt.addingTimeInterval(-OTPMatcher.arrivalTolerance) : nil
        if let match = OTPMatcher.select(otpPromptedAt: promptedAt, messages: messages, candidatesNotBefore: notBefore) {
            matched(match, promptedAt: promptedAt)
            return
        }
        if let fetchError { throw fetchError }
    }

    private func historyMessages() async throws(SessionError) -> [GmailMessageRef] {
        guard let start = cursor else { return [] }
        var refs: [GmailMessageRef] = []
        var pageToken: String?
        var latest: String?
        var pages = 0
        repeat {
            let page = try await call { [client] token async throws(GmailAPIError) in
                try await client.history(accessToken: token, startHistoryID: start, pageToken: pageToken)
            }
            refs += page.addedMessages
            latest = page.historyId ?? latest
            pageToken = page.nextPageToken
            pages += 1
        } while pageToken != nil && pages < Self.maxHistoryPages
        if pageToken == nil, let latest, GmailAPI.isValidHistoryID(latest) {
            cursor = latest
        }
        return refs
    }

    private func searchedMessages() async throws(SessionError) -> [GmailMessageRef] {
        try await call { [client] token async throws(GmailAPIError) in
            try await client.listMessages(accessToken: token, query: Self.fallbackQuery, maxResults: Self.fallbackLimit)
        }.messages
    }

    /// Fetches metadata for every pending ID in parallel. Fetched and vanished (404) IDs leave the
    /// queue; others stay for the next poll. Returns the first failure, if any.
    private func fetchPending() async -> SessionError? {
        guard !pending.isEmpty else { return nil }
        let refs = pending
        let results: [(String, Result<GmailMessageMetadata, GmailAPIError>)]
        do {
            results = try await call { [client] token async throws(GmailAPIError) in
                let results = await Self.fetchMetadata(client: client, ids: refs.map(\.id), accessToken: token)
                if results.contains(where: { if case .failure(.unauthorized) = $0.1 { true } else { false } }) {
                    throw .unauthorized
                }
                return results
            }
        } catch {
            return error
        }
        var firstError: SessionError?
        for (id, result) in results {
            switch result {
            case let .success(metadata):
                messages.append(metadata)
                settle(id)
            case .failure(.notFound):
                settle(id)
            case let .failure(error):
                firstError = firstError ?? .api(error)
            }
        }
        return firstError
    }

    private func settle(_ id: String) {
        seen.insert(id)
        pending.removeAll { $0.id == id }
    }

    private nonisolated static func fetchMetadata(
        client: GmailAPIClient, ids: [String], accessToken: String
    ) async -> [(String, Result<GmailMessageMetadata, GmailAPIError>)] {
        await withTaskGroup(of: (String, Result<GmailMessageMetadata, GmailAPIError>).self) { group in
            for id in ids {
                group.addTask { @Sendable in
                    do throws(GmailAPIError) {
                        let metadata = try await client.messageMetadata(accessToken: accessToken, id: id)
                        return (id, .success(metadata))
                    } catch {
                        return (id, .failure(error))
                    }
                }
            }
            var results: [(String, Result<GmailMessageMetadata, GmailAPIError>)] = []
            for await result in group { results.append(result) }
            return results
        }
    }

    private func matched(_ match: GmailOTPSelection, promptedAt: Date) {
        selection = match
        logger.info("OTP message matched via the Gmail API (\(match.allCandidateIDs.count) candidate(s), \(self.polls) poll(s))")
        guard ending == nil else { return }
        emit(.codeFound(attemptID, code: match.code))
        emit(.note(attemptID, Self.timingNote(
            promptedAt: promptedAt, arrivedAt: match.internalDate, foundAt: Date(), polls: polls
        )))
    }

    /// Both spans are clamped at zero (clocks differ slightly); `internalDate` has millisecond resolution.
    static func timingNote(promptedAt: Date, arrivedAt: Date, foundAt: Date, polls: Int) -> String {
        let delivery = seconds(arrivedAt.timeIntervalSince(promptedAt))
        let detection = seconds(foundAt.timeIntervalSince(arrivedAt))
        return "‹OTP mail (Gmail API): in Gmail \(delivery)s after the prompt · detected \(detection)s after arrival "
            + "(\(polls) \(polls == 1 ? "poll" : "polls"))›"
    }

    private static func seconds(_ interval: TimeInterval) -> String {
        String(format: "%.1f", max(0, interval))
    }

    // MARK: - Ending

    /// Runs the requested ending. Errors are only logged: the attempt has moved on.
    private func wrapUp() async {
        switch ending {
        case .finish?:
            await trash(selection?.allCandidateIDs ?? [])
        case .close(checkJunk: true)?:
            await checkJunk()
        case .close(checkJunk: false)?, nil:
            break
        }
    }

    private func trash(_ ids: [String]) async {
        guard !ids.isEmpty else { return }
        var trashed = 0
        for id in ids {
            do {
                try await call { [client] token async throws(GmailAPIError) in
                    try await client.trash(accessToken: token, id: id)
                }
                trashed += 1
            } catch {
                logger.warning("Could not trash an AuthCode message: \(error.logDescription, privacy: .public)")
            }
        }
        logger.info("Moved \(trashed) of \(ids.count) AuthCode message(s) to Trash")
    }

    private func checkJunk() async {
        guard let promptedAt else { return }
        do {
            let list = try await call { [client] token async throws(GmailAPIError) in
                try await client.listMessages(
                    accessToken: token, query: Self.junkQuery, includeSpamTrash: true, maxResults: Self.junkLimit
                )
            }
            let ids = list.messages.map(\.id)
            guard !ids.isEmpty else { return }
            let results = try await call { [client] token async throws(GmailAPIError) in
                await Self.fetchMetadata(client: client, ids: ids, accessToken: token)
            }
            let spam = results.compactMap { try? $0.1.get() }
            guard OTPMatcher.containsFreshAuthCode(otpPromptedAt: promptedAt, messages: spam) else { return }
            logger.notice("Fresh AuthCode message found in the spam folder")
            emit(.foundInJunk(attemptID))
        } catch {
            logger.error("Gmail API spam check failed: \(error.logDescription, privacy: .public)")
        }
    }

    // MARK: - Calls

    /// Runs `operation` with an access token; on 401 drops the cached token and tries once more.
    private func call<T: Sendable>(
        _ operation: (String) async throws(GmailAPIError) -> T
    ) async throws(SessionError) -> T {
        var retried = false
        while true {
            let token: String
            do {
                token = try await tokens.accessToken(for: address)
            } catch {
                throw .token(error)
            }
            do {
                return try await operation(token)
            } catch .unauthorized where !retried {
                tokens.invalidateAccessToken(for: address)
                retried = true
            } catch {
                throw .api(error)
            }
        }
    }
}

/// A one-slot wake-up for the attempt's loop: `signal()` ends the current `wait` (or makes the next
/// one return at once); otherwise `wait` returns after its timeout.
private final class WakeSignal {
    private var continuation: CheckedContinuation<Void, Never>?
    private var timer: Task<Void, Never>?
    private var signaled = false

    func wait(upTo timeout: Duration) async {
        if signaled {
            signaled = false
            return
        }
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            self.continuation = continuation
            timer = Task { [weak self] in
                try? await Task.sleep(for: timeout)
                guard !Task.isCancelled else { return }
                self?.resume()
            }
        }
    }

    func signal() {
        if continuation != nil {
            resume()
        } else {
            signaled = true
        }
    }

    private func resume() {
        timer?.cancel()
        timer = nil
        guard let continuation else { return }
        self.continuation = nil
        continuation.resume()
    }
}
