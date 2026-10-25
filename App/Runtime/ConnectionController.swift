import AppKit
import FortiConfig
import FortiCore
import FortiShared
import Observation
import os

/// Raw inputs for the menu's one-line status text (DESIGN §5.6 "status text, refreshed every 30 s").
struct ConnectionStatusInputs: Equatable {
    var state: ConnectionState
    var desired: DesiredState
    var readiness: Readiness
    var now: Date
    var connectedSince: Date?
    var tunnelIP: String?
    var plannedReconnectAt: Date?
    var nextRetryAt: Date?
    var lastFailure: ConnectionFailureReason?
    var scheduleOutlook: ScheduleOutlook?
}

/// Owns the `ConnectionModel`, feeds every input through `ConnectionReducer` and executes the
/// resulting effects in order (DESIGN §5.4, §6). Events raised while effects run are queued, so each
/// event's effects finish before the next event is reduced.
///
/// Inputs: helper events/output/connection loss (one ordered stream), OTP session events,
/// `SystemMonitor`, `ConfigStore` changes, readiness answers, timers and the user's menu actions.
@Observable
final class ConnectionController {
    private(set) var model: ConnectionModel
    /// Wall-clock "now" for relative times in the UI: ticks every 30 s and after every event.
    private(set) var now = Date()

    @ObservationIgnored private let configStore: ConfigStore
    @ObservationIgnored private let stateStore: StateStore
    @ObservationIgnored private let credentials: CredentialStore
    @ObservationIgnored private let helper: HelperClient
    @ObservationIgnored private let readinessChecker: ReadinessChecker
    @ObservationIgnored private let otpSession: any OTPSessionService
    @ObservationIgnored private let logStore: LogStore
    @ObservationIgnored private let timers = ConnectionTimers()
    @ObservationIgnored private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "connection")

    @ObservationIgnored private var launched = false
    @ObservationIgnored private var pendingEvents: [ConnectionEvent] = []
    @ObservationIgnored private var isDispatching = false
    @ObservationIgnored private var helperRequests: Task<Void, Never>?
    @ObservationIgnored private var readinessTask: Task<Void, Never>?
    @ObservationIgnored private var readinessRequested = false
    @ObservationIgnored private var systemSettingsWatch: Task<Void, Never>?
    @ObservationIgnored private var inboundTask: Task<Void, Never>?
    @ObservationIgnored private var tickerTask: Task<Void, Never>?
    @ObservationIgnored private var activity: NSObjectProtocol?
    @ObservationIgnored private var clockObservers: [NSObjectProtocol] = []
    @ObservationIgnored private var terminationPending = false
    @ObservationIgnored private var terminationApproved = false

    static let tickInterval: Duration = .seconds(30)
    /// Readiness polling "every 2 s for 2 minutes after opening System Settings".
    static let systemSettingsWatchDuration: TimeInterval = 120
    static let systemSettingsWatchInterval: Duration = .seconds(2)

    init(
        configStore: ConfigStore,
        stateStore: StateStore,
        credentials: CredentialStore,
        helper: HelperClient,
        readinessChecker: ReadinessChecker,
        otpSession: any OTPSessionService,
        logStore: LogStore
    ) {
        self.configStore = configStore
        self.stateStore = stateStore
        self.credentials = credentials
        self.helper = helper
        self.readinessChecker = readinessChecker
        self.otpSession = otpSession
        self.logStore = logStore
        model = Self.freshModel(configStore.config)

        timers.onFire = { [weak self] timer, generation in
            self?.send(.timerFired(timer, generation: generation))
        }
        otpSession.onEvent = { [weak self] event in
            self?.handleOTPEvent(event)
        }
        readinessChecker.canRestartHelper = { [weak self] in
            self?.model.activeAttemptID == nil
        }
    }

    // MARK: - UI state

    var icon: ConnectionIcon { model.icon }
    var menuActions: ConnectionMenuActions { model.menuActions }
    /// One-line reason plus "Open Settings" when the icon shows an error.
    var attention: ConnectionAttention? { model.attention }
    /// Next proactive reconnect, unless the schedule holds or stopped automatic reconnects.
    var plannedReconnectAt: Date? {
        model.automaticReconnectAllowed(at: now) ? model.nextPlannedReconnect : nil
    }
    var nextRetryAt: Date? { model.nextRetryAt }
    /// What the schedule does next, or `nil` without a schedule.
    var scheduleOutlook: ScheduleOutlook? { model.scheduleOutlook(now: now) }

    var connectedSince: Date? {
        guard case let .connected(context) = model.state else { return nil }
        return context.since
    }

    var tunnelIP: String? {
        guard case let .connected(context) = model.state else { return nil }
        return context.tunnelIP
    }

    /// The failure behind the current state (`failed`, or the wait after a counted or network failure).
    var lastFailure: ConnectionFailureReason? {
        if let failure = model.lastFailure { return failure }
        switch model.state {
        case let .failed(reason):
            return reason
        case let .waitingRetry(wait):
            return failure(for: wait.reason)
        case let .waitingNetwork(wait):
            return failure(for: wait.reason)
        default:
            return nil
        }
    }

    var statusInputs: ConnectionStatusInputs {
        ConnectionStatusInputs(
            state: model.state,
            desired: model.desired,
            readiness: model.readiness,
            now: now,
            connectedSince: connectedSince,
            tunnelIP: tunnelIP,
            plannedReconnectAt: plannedReconnectAt,
            nextRetryAt: nextRetryAt,
            lastFailure: lastFailure,
            scheduleOutlook: scheduleOutlook
        )
    }

    private func failure(for reason: RetryReason) -> ConnectionFailureReason? {
        switch reason {
        case .network: .network
        case let .authFailure(failure): failure
        case .authCooldown: model.ledger.lastAuthFailureReason
        case .serverSessionEnded, .sleep, .userAway: nil
        }
    }

    // MARK: - User intents

    /// "Connect" (also clears the failure counter).
    func connect() { send(.userConnect) }
    /// "Disconnect".
    func disconnect() { send(.userDisconnect) }
    /// "Reconnect Now" (waits until 90 s after the last authentication failure).
    func reconnectNow() { send(.userReconnectNow) }
    /// "Quit": goes through `applicationShouldTerminate` for a graceful disconnect.
    func quit() { NSApp.terminate(nil) }

    /// Re-check readiness now (app activation, settings window opened).
    func recheckReadiness() {
        requestReadinessCheck()
    }

    /// After System Settings was opened for the helper: re-check every 2 s for 2 minutes.
    func recheckReadinessWhileSystemSettingsOpen() {
        systemSettingsWatch?.cancel()
        systemSettingsWatch = Task { [weak self] in
            let end = Date().addingTimeInterval(Self.systemSettingsWatchDuration)
            while Date() < end {
                do { try await Task.sleep(for: Self.systemSettingsWatchInterval) } catch { return }
                self?.requestReadinessCheck()
            }
        }
    }

    /// `NSApplicationDelegate.applicationShouldTerminate`: stop the tunnel first, then reply (at most 12 s).
    func applicationShouldTerminate() -> NSApplication.TerminateReply {
        if terminationApproved { return .terminateNow }
        if !terminationPending {
            terminationPending = true
            send(.quitRequested)
        }
        return .terminateLater
    }

    // MARK: - Wiring

    /// Starts consuming helper messages and the 30 s ticker, then sends `appLaunched`.
    /// `ConfigStore` must have loaded the file already.
    func launch(autoConnect: Bool, restoredLedger: FailureLedger?) {
        guard !launched else { return }
        launched = true
        model = Self.freshModel(configStore.config)

        inboundTask = Task { [weak self, helper] in
            for await message in helper.inbound {
                self?.handleHelperMessage(message)
            }
        }
        tickerTask = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(for: Self.tickInterval) } catch { return }
                self?.now = Date()
            }
        }
        let center = NotificationCenter.default
        clockObservers = [
            // Timers sleep for a computed delay; a clock step makes that delay wrong.
            center.addObserver(forName: .NSSystemClockDidChange, object: nil, queue: .main) { @Sendable [weak self] _ in
                MainActor.assumeIsolated { self?.timers.rearmAll() }
            },
            center.addObserver(forName: .NSSystemTimeZoneDidChange, object: nil, queue: .main) { @Sendable [weak self] _ in
                MainActor.assumeIsolated { self?.systemTimeZoneDidChange() }
            },
        ]
        send(.appLaunched(autoConnect: autoConnect, restoredLedger: restoredLedger))
    }

    /// Schedule times are local wall-clock times: re-read them in the new time zone.
    private func systemTimeZoneDidChange() {
        NSTimeZone.resetSystemTimeZone()
        logger.notice("time zone changed to \(TimeZone.current.identifier, privacy: .public)")
        send(.calendarChanged(Calendar.current))
        timers.rearmAll()
    }

    /// `ConfigStore.onChange`: account and options apply to the next attempt; session limits apply now.
    func configDidChange(_ change: ConfigChange) {
        if change.new.vpn != change.old.vpn {
            send(.profileChanged(VPNProfile(change.new.vpn)))
        }
        let settings = ConnectionSettings(change.new.connection)
        if settings != model.settings {
            send(.settingsChanged(settings))
        }
        // `otp` covers the Gmail address, the OTP method and the Google client ID.
        let accountChanged = change.new.vpn.host != change.old.vpn.host
            || change.new.vpn.username != change.old.vpn.username
            || change.new.otp != change.old.otp
        if change.validityChanged || accountChanged {
            requestReadinessCheck()
        }
    }

    /// `SystemMonitor.onEvent`.
    func handleSystemEvent(_ event: ConnectionEvent) {
        send(event)
        if case .didWake = event {
            // Task sleeps may be skewed by system sleep; recompute every deadline from the wall clock.
            timers.rearmAll()
        }
    }

    /// Feeds one event through the reducer and runs its effects. Re-entrant calls are queued.
    func send(_ event: ConnectionEvent) {
        pendingEvents.append(event)
        guard !isDispatching else { return }
        isDispatching = true
        defer { isDispatching = false }
        while !pendingEvents.isEmpty {
            let next = pendingEvents.removeFirst()
            var updated = model
            let effects = ConnectionReducer.reduce(state: &updated, event: next, now: Date())
            if updated != model {
                if updated.state != model.state {
                    logger.info("state: \(String(describing: updated.state), privacy: .public)")
                }
                noteScheduleDecisions(from: model, to: updated)
                model = updated
            }
            effects.forEach(perform)
        }
        now = Date()
    }

    /// Leaves a `▸` line in the Log tab when the schedule connects, holds or stops reconnects, so
    /// a quiet evening can be told apart from a failure.
    private func noteScheduleDecisions(from old: ConnectionModel, to new: ConnectionModel) {
        guard let schedule = new.settings.schedule else { return }
        var notes: [String] = []
        if case let .connecting(context) = new.state, context.reason == .scheduled,
           !Self.isConnecting(old.state, attemptID: context.attemptID) {
            notes.append("‹schedule: scheduled hours started, connecting›")
        }
        if Self.isHolding(new.state), !Self.isHolding(old.state) {
            notes.append("‹schedule: outside the scheduled hours and away, the reconnect waits until you return›")
        }
        if new.reconnectSuspended, !old.reconnectSuspended {
            notes.append("‹schedule: away for \(schedule.awayMinutes) min outside the scheduled hours, automatic reconnects stopped›")
        }
        guard !notes.isEmpty else { return }
        let date = Date()
        logStore.append(notes.map { OutputLine(seq: 0, date: date, attemptID: nil, source: .helper, text: $0) })
    }

    private static func isHolding(_ state: ConnectionState) -> Bool {
        if case let .waitingRetry(wait) = state { wait.reason == .userAway } else { false }
    }

    private static func isConnecting(_ state: ConnectionState, attemptID: AttemptID) -> Bool {
        if case let .connecting(context) = state { context.attemptID == attemptID } else { false }
    }

    private static func freshModel(_ config: AppConfig) -> ConnectionModel {
        ConnectionModel(
            profile: VPNProfile(config.vpn),
            settings: ConnectionSettings(config.connection),
            attemptIDSeed: UUID()
        )
    }

    // MARK: - Inbound

    private func handleHelperMessage(_ message: HelperInbound) {
        switch message {
        case let .event(event): send(.helperEvent(event))
        case let .output(lines): logStore.append(lines)
        case .connectionLost: send(.helperConnectionLost)
        }
    }

    private func handleOTPEvent(_ event: OTPSessionEvent) {
        switch event {
        case let .ready(attemptID): send(.otpSessionReady(attemptID))
        case let .failed(attemptID, failure): send(.otpSessionFailed(attemptID, failure))
        case let .codeFound(attemptID, code): send(.otpCodeFound(attemptID, code: code))
        case let .foundInJunk(attemptID): send(.otpFoundInJunk(attemptID))
        case let .note(attemptID, text):
            logStore.append([OutputLine(seq: 0, date: Date(), attemptID: attemptID, source: .helper, text: text)])
        }
    }

    // MARK: - Effects

    private func perform(_ effect: ConnectionEffect) {
        switch effect {
        case .checkReadiness:
            requestReadinessCheck()
        case let .startOTPSession(attemptID):
            startOTPSession(attemptID)
        case let .beginOTPSearch(attemptID, promptedAt):
            otpSession.beginSearch(attemptID: attemptID, promptedAt: promptedAt)
        case let .closeOTPSession(attemptID, checkJunk):
            otpSession.close(attemptID: attemptID, checkJunk: checkJunk)
        case let .startTunnel(attemptID, profile):
            startTunnel(attemptID, profile: profile)
        case let .submitOTP(attemptID, code):
            submitOTP(attemptID, code: code)
        case let .stopTunnel(attemptID, reason):
            stopTunnel(attemptID, reason: reason)
        case let .persistTrustedCerts(digests):
            configStore.pinTrustedCerts(digests)
        case let .persistLedger(ledger):
            stateStore.saveLedger(ledger)
        case let .scheduleTimer(timer, date, generation):
            timers.schedule(timer, at: date, generation: generation)
        case let .cancelTimer(timer):
            timers.cancel(timer)
        case let .setActivity(hold):
            setActivity(hold)
        case .terminateApp:
            terminateApp()
        }
    }

    /// Coalesces checks: at most one runs; a request while it runs triggers exactly one more.
    private func requestReadinessCheck() {
        readinessRequested = true
        guard readinessTask == nil else { return }
        readinessTask = Task { [weak self] in
            while let self, self.readinessRequested {
                self.readinessRequested = false
                let readiness = await self.readinessChecker.check()
                self.send(.readinessChanged(readiness))
            }
            self?.readinessTask = nil
        }
    }

    /// The session (`OTPSessionRouter`) picks IMAP or the Gmail API; the app password is passed
    /// along when one is stored (empty otherwise) for IMAP and the API session's IMAP fallback.
    private func startOTPSession(_ attemptID: AttemptID) {
        let address = configStore.config.otp.gmailAddress
        guard !address.isEmpty else {
            send(.otpSessionFailed(attemptID, .authentication))
            return
        }
        var appPassword = ""
        do {
            appPassword = try credentials.gmailAccount(address: address)?.appPassword ?? ""
        } catch {
            logger.error("reading the Gmail app password failed: \(error.localizedDescription, privacy: .public)")
        }
        otpSession.start(attemptID: attemptID, account: GmailAccount(address: address, appPassword: appPassword))
    }

    private func startTunnel(_ attemptID: AttemptID, profile: VPNProfile) {
        let password: String
        do {
            guard let stored = try credentials.vpnPassword(for: profile), !stored.isEmpty else {
                connectRejected(attemptID, HelperError(.invalidRequest, "no VPN password in the Keychain"))
                return
            }
            password = stored
        } catch {
            logger.error("reading the VPN password failed: \(error.localizedDescription, privacy: .public)")
            connectRejected(attemptID, HelperError(
                .invalidRequest, "could not read the VPN password from the Keychain: \(error.localizedDescription)"
            ))
            return
        }
        let request = profile.connectRequest(attemptID: attemptID, password: password)
        logger.notice("connect \(attemptID, privacy: .public) (\(profile.trustedCerts.count) pinned digest(s))")
        enqueueHelperRequest { [weak self] in
            guard let self else { return }
            do throws(HelperError) {
                try await helper.connect(request)
            } catch {
                logger.error("connect rejected: \(error.code.rawValue, privacy: .public) \(error.message, privacy: .public)")
                connectRejected(attemptID, error)
            }
        }
    }

    /// The run never started, so the helper wrote no output: keep the reason in the Log tab as a
    /// `▸` meta line (the failure itself only carries `error.code`).
    private func connectRejected(_ attemptID: AttemptID, _ error: HelperError) {
        logStore.append([OutputLine(
            seq: 0, date: Date(), attemptID: attemptID, source: .helper, text: "‹connect rejected: \(error.message)›"
        )])
        send(.helperConnectRejected(attemptID, error))
    }

    private func submitOTP(_ attemptID: AttemptID, code: String) {
        enqueueHelperRequest { [weak self] in
            guard let self else { return }
            do throws(HelperError) {
                try await helper.submitOTP(attemptID: attemptID, code: code)
            } catch {
                // The helper's watchdog ends the run; `.exited` classifies it.
                logger.error("submitOTP rejected: \(error.code.rawValue, privacy: .public)")
            }
            otpSession.finish(attemptID: attemptID)
        }
    }

    private func stopTunnel(_ attemptID: AttemptID, reason: StopReason) {
        logger.notice("stop \(attemptID, privacy: .public): \(reason.rawValue, privacy: .public)")
        enqueueHelperRequest { [weak self] in
            guard let self else { return }
            do throws(HelperError) {
                try await helper.disconnect(attemptID: attemptID, reason: reason)
            } catch {
                logger.error("disconnect rejected: \(error.code.rawValue, privacy: .public) \(error.message, privacy: .public)")
            }
        }
    }

    /// Sends helper requests one after another in effect order (e.g. `disconnect` never overtakes the
    /// `connect` of the same attempt while that one is still verifying the connection).
    private func enqueueHelperRequest(_ request: @escaping @MainActor () async -> Void) {
        let previous = helperRequests
        helperRequests = Task {
            await previous?.value
            await request()
        }
    }

    private func setActivity(_ hold: Bool) {
        if hold, activity == nil {
            activity = ProcessInfo.processInfo.beginActivity(
                options: .userInitiatedAllowingIdleSystemSleep,
                reason: "Keeping the VPN connection alive"
            )
        } else if !hold, let activity {
            ProcessInfo.processInfo.endActivity(activity)
            self.activity = nil
        }
    }

    /// Always answered asynchronously, after `applicationShouldTerminate` returned `.terminateLater`
    /// and the log file caught up.
    private func terminateApp() {
        Task { [weak self] in
            guard let self else { return }
            await logStore.flushToFile()
            terminationApproved = true
            if terminationPending {
                NSApp.reply(toApplicationShouldTerminate: true)
            } else {
                NSApp.terminate(nil)
            }
        }
    }
}
