import Foundation
import os
import FortiCore
import FortiShared

/// The `OTPSessionService` the `ConnectionController` talks to: per attempt it picks the Gmail API
/// session when `otp.method` is `gmail_api` and its authorization is stored, otherwise the IMAP
/// session (which needs the app password).
///
/// When the API session fails with `.authentication` before `.ready` (revoked authorization, refused
/// client, Gmail API disabled) and an app password exists, the same attempt continues over IMAP
/// transparently; the Log tab gets a note. Events from a backend that no longer serves the current
/// attempt are dropped.
final class OTPSessionRouter: OTPSessionService {
    enum Backend: String {
        case imap = "IMAP"
        case gmailAPI = "Gmail API"
    }

    var onEvent: (@MainActor (OTPSessionEvent) -> Void)?

    private struct Active {
        var attemptID: AttemptID
        var account: GmailAccount
        /// `nil` when no backend could be started (the attempt already failed).
        var backend: Backend?
        var ready = false
    }

    private let imap: any OTPSessionService
    private let api: GmailAPIOTPSession
    private let availability: (String) -> GmailAPIAvailability
    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "otp")
    private var active: Active?

    /// `availability` answers for a Gmail address whether the API session can be used (method and
    /// stored authorization), e.g. `GoogleOAuthService.availability(address:)`.
    init(imap: any OTPSessionService, api: GmailAPIOTPSession, availability: @escaping (String) -> GmailAPIAvailability) {
        self.imap = imap
        self.api = api
        self.availability = availability
        imap.onEvent = { [weak self] event in
            self?.handle(event, from: .imap)
        }
        api.onEvent = { [weak self] event in
            self?.handle(event, from: .gmailAPI)
        }
    }

    /// The backend serving the current attempt, if any (diagnostics).
    var currentBackend: Backend? { active?.backend }

    func start(attemptID: AttemptID, account: GmailAccount) {
        guard active?.attemptID != attemptID else { return }
        if let previous = active, let backend = previous.backend {
            session(backend).close(attemptID: previous.attemptID, checkJunk: false)
        }
        let hasAppPassword = !account.appPassword.isEmpty
        switch availability(account.address) {
        case .available:
            active = Active(attemptID: attemptID, account: account, backend: .gmailAPI)
            api.start(attemptID: attemptID, account: account)
        case .notConfigured where hasAppPassword:
            active = Active(attemptID: attemptID, account: account, backend: .imap)
            emit(.note(attemptID, "‹Gmail API selected but not signed in with Google; using IMAP›"))
            imap.start(attemptID: attemptID, account: account)
        case .notSelected where hasAppPassword:
            active = Active(attemptID: attemptID, account: account, backend: .imap)
            imap.start(attemptID: attemptID, account: account)
        case .notConfigured:
            active = Active(attemptID: attemptID, account: account, backend: nil)
            emit(.note(attemptID, "‹Gmail API selected but not signed in with Google, and no IMAP app password is stored›"))
            emit(.failed(attemptID, .authentication))
        case .notSelected:
            active = Active(attemptID: attemptID, account: account, backend: nil)
            emit(.failed(attemptID, .authentication))
        }
    }

    func beginSearch(attemptID: AttemptID, promptedAt: Date) {
        backend(for: attemptID).map { session($0).beginSearch(attemptID: attemptID, promptedAt: promptedAt) }
    }

    func finish(attemptID: AttemptID) {
        backend(for: attemptID).map { session($0).finish(attemptID: attemptID) }
    }

    func close(attemptID: AttemptID, checkJunk: Bool) {
        backend(for: attemptID).map { session($0).close(attemptID: attemptID, checkJunk: checkJunk) }
    }

    // MARK: - Events

    private func handle(_ event: OTPSessionEvent, from backend: Backend) {
        guard let current = active, current.backend == backend, event.attemptID == current.attemptID else { return }
        switch event {
        case .ready:
            active?.ready = true
        case let .failed(attemptID, failure) where backend == .gmailAPI:
            let reason = api.failureReason(attemptID: attemptID) ?? "unknown error"
            if failure == .authentication, !current.ready, !current.account.appPassword.isEmpty {
                logger.notice("Gmail API unavailable (\(reason, privacy: .public)); falling back to IMAP")
                emit(.note(attemptID, "‹Gmail API unavailable (\(reason)); falling back to IMAP›"))
                active?.backend = .imap
                api.close(attemptID: attemptID, checkJunk: false)
                imap.start(attemptID: attemptID, account: current.account)
                return
            }
            emit(.note(attemptID, "‹Gmail API session failed (\(reason))›"))
        default:
            break
        }
        emit(event)
    }

    private func emit(_ event: OTPSessionEvent) {
        onEvent?(event)
    }

    private func backend(for attemptID: AttemptID) -> Backend? {
        active?.attemptID == attemptID ? active?.backend : nil
    }

    private func session(_ backend: Backend) -> any OTPSessionService {
        switch backend {
        case .imap: imap
        case .gmailAPI: api
        }
    }
}
