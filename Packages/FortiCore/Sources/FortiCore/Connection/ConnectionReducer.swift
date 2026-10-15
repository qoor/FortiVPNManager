import Foundation
import FortiShared

/// The connection state machine (§5.4, §6). Pure: time comes in as `now`, attempt IDs come from the
/// model's seeded generator, and every side effect is returned as a `ConnectionEffect`.
///
/// Timers, App Nap activity and ledger persistence are derived from the resulting state after each
/// event, so no transition can forget to cancel a timer or save the ledger.
public enum ConnectionReducer {
    public static func reduce(state model: inout ConnectionModel, event: ConnectionEvent, now: Date) -> [ConnectionEffect] {
        guard !model.isTerminated else { return [] }
        var now = now
        if case let .timerFired(timer, generation) = event, let slot = model.timers[timer],
           slot.generation == generation, !slot.fired {
            // A current timer is never early as far as the reducer is concerned. The app may sleep on a
            // monotonic clock and stamp a slewed (or stepped-back) wall clock; evaluated a few ms early, a
            // retry gated by the authentication cooldown would re-enter the very same wait whose timer has
            // already fired, and nothing would ever move it on.
            now = max(now, slot.date)
        }
        var transition = ConnectionTransition(model: model, now: now)
        transition.handle(event)
        let effects = transition.finish()
        model = transition.model
        return effects
    }
}

/// One `reduce` call: the model being edited plus the effects collected so far.
private struct ConnectionTransition {
    var model: ConnectionModel
    let now: Date
    private var ledgerBefore: FailureLedger
    private var effects: [ConnectionEffect] = []
    private var terminates = false

    init(model: ConnectionModel, now: Date) {
        self.model = model
        self.now = now
        self.ledgerBefore = model.ledger
    }

    private mutating func emit(_ effect: ConnectionEffect) {
        effects.append(effect)
    }

    // MARK: - Dispatch

    mutating func handle(_ event: ConnectionEvent) {
        switch event {
        case let .appLaunched(autoConnect, restoredLedger): appLaunched(autoConnect: autoConnect, restoredLedger: restoredLedger)
        case let .readinessChanged(readiness): readinessChanged(readiness)
        case let .profileChanged(profile): profileChanged(profile)
        case let .settingsChanged(settings): settingsChanged(settings)
        case .userConnect: userConnect()
        case .userDisconnect: userDisconnect()
        case .userReconnectNow: userReconnectNow()
        case .quitRequested: quitRequested()
        case let .otpSessionReady(attemptID): otpSessionReady(attemptID)
        case let .otpSessionFailed(attemptID, failure): otpSessionFailed(attemptID, failure)
        case let .otpCodeFound(attemptID, code): otpCodeFound(attemptID, code: code)
        case let .otpFoundInJunk(attemptID): otpFoundInJunk(attemptID)
        case let .helperEvent(event): helperEvent(event)
        case let .helperConnectRejected(attemptID, error): helperConnectRejected(attemptID, error)
        case .helperConnectionLost: helperConnectionLost()
        case let .timerFired(timer, generation): timerFired(timer, generation: generation)
        case .willSleep: willSleep()
        case let .didWake(sleptFor): didWake(sleptFor: sleptFor)
        case let .wakeSettled(observation): wakeSettled(observation)
        case let .pathChanged(snapshot): pathChanged(snapshot)
        }
    }

    /// Derives timers, activity and ledger persistence from the final state. `terminateApp` stays last.
    mutating func finish() -> [ConnectionEffect] {
        reconcileTimers()
        let wantsActivity = !model.isTerminated && model.state.holdsActivity
        if wantsActivity != model.activityHeld {
            model.activityHeld = wantsActivity
            emit(.setActivity(wantsActivity))
        }
        if model.ledger != ledgerBefore {
            emit(.persistLedger(model.ledger))
        }
        if terminates {
            emit(.terminateApp)
        }
        return effects
    }

    private mutating func reconcileTimers() {
        let wanted = model.isTerminated ? [:] : model.wantedTimers()
        for timer in ConnectionTimer.allCases {
            let slot = model.timers[timer]
            switch (slot, wanted[timer]) {
            case (nil, nil):
                break
            case let (slot?, nil):
                model.timers[timer] = nil
                if !slot.fired { emit(.cancelTimer(timer)) }
            case let (slot, date?):
                // A fired timer whose date is still wanted stays fired (no re-fire loop).
                guard slot?.date != date else { continue }
                model.nextTimerGeneration += 1
                let generation = model.nextTimerGeneration
                model.timers[timer] = TimerSlot(date: date, generation: generation)
                emit(.scheduleTimer(timer, at: date, generation: generation))
            }
        }
    }

    // MARK: - Lifecycle and configuration

    private mutating func appLaunched(autoConnect: Bool, restoredLedger: FailureLedger?) {
        guard !model.hasLaunched else { return }
        model.hasLaunched = true
        if let restoredLedger, restoredLedger.profileKey == model.profile.profileKey {
            model.ledger = restoredLedger
            ledgerBefore = restoredLedger
        }
        model.ledger.prune(asOf: now)

        guard autoConnect else {
            model.desired = .disconnected
            emit(.checkReadiness)
            return
        }
        model.desired = .connected
        if model.ledger.consecutiveAuthFailures >= RetryPolicy.maxConsecutiveAuthFailures {
            // The policy had already given up before the relaunch; only a manual action clears it.
            fail(model.ledger.lastAuthFailureReason ?? .authenticationRejected)
            return
        }
        beginAttempt(.launch)
    }

    private mutating func readinessChanged(_ readiness: Readiness) {
        model.readiness = readiness
        switch model.state {
        case var .connecting(context) where context.stage == .checkingReadiness:
            switch readiness {
            case .ready:
                context.stage = .preparingOTP
                context.otpSessionOpen = true
                model.state = .connecting(context)
                emit(.startOTPSession(context.attemptID))
            case let .needsSetup(items):
                model.state = .needsSetup(items)
            case .unknown:
                break
            }
        case .idle, .needsSetup, .waitingRetry, .waitingNetwork:
            switch readiness {
            case let .needsSetup(items):
                model.state = .needsSetup(items)
            case .ready:
                guard case .needsSetup = model.state else { break }
                if model.desired == .connected {
                    beginAttempt(.setupCompleted, readinessVerified: true)
                } else {
                    model.state = .idle
                }
            case .unknown:
                break
            }
        case .connecting, .connected, .disconnecting, .failed:
            // Applies to the next attempt; `failed` is only cleared by the user.
            break
        }
    }

    private mutating func profileChanged(_ incoming: VPNProfile) {
        let old = model.profile
        var profile = incoming
        if profile.isDifferentGateway(from: old), !profile.trustedCerts.isEmpty, profile.trustedCerts == old.trustedCerts {
            // The pins belong to the old gateway; the next connect learns the new one (§5.4).
            profile.trustedCerts = []
            emit(.persistTrustedCerts([]))
        }
        model.profile = profile
        if profile.profileKey != old.profileKey {
            model.ledger = FailureLedger(profileKey: profile.profileKey)
        }
        if profile.isDifferentAccount(from: old) {
            model.readiness = .unknown
            emit(.checkReadiness)
        }
    }

    private mutating func settingsChanged(_ settings: ConnectionSettings) {
        let old = model.settings
        model.settings = settings
        guard settings.affectsPlannedReconnect(comparedTo: old), case var .connected(context) = model.state else { return }
        context.plannedReconnectAt = RetryPolicy.plannedReconnect(connectedAt: context.since, settings: settings, now: now)
        model.state = .connected(context)
    }

    // MARK: - User actions

    private mutating func userConnect() {
        switch model.state {
        case .idle, .needsSetup, .failed:
            model.desired = .connected
            resetForManualAction()
            beginAttempt(.user)
        case var .disconnecting(context):
            model.desired = .connected
            if context.after == .idle {
                resetForManualAction()
                context.after = .reconnect(.user)
                model.state = .disconnecting(context)
            }
        case .connecting, .connected, .waitingRetry, .waitingNetwork:
            break
        }
    }

    private mutating func userDisconnect() {
        model.desired = .disconnected
        switch model.state {
        case .idle, .needsSetup:
            break
        case .failed, .waitingRetry, .waitingNetwork:
            model.state = .idle
        case let .connecting(context):
            abortAttempt(context, stop: .userDisconnect, then: .idle)
        case let .connected(context):
            stopSession(context, reason: .userDisconnect, then: .idle)
        case var .disconnecting(context):
            if context.after != .terminate {
                context.after = .idle
                model.state = .disconnecting(context)
            }
        }
    }

    private mutating func userReconnectNow() {
        switch model.state {
        case let .connected(context):
            resetForManualAction()
            stopSession(context, reason: .reconnect, then: .reconnect(.manualReconnect))
        case .waitingRetry, .waitingNetwork, .failed:
            model.desired = .connected
            resetForManualAction()
            beginAttempt(.manualReconnect)
        case .idle, .needsSetup, .connecting, .disconnecting:
            // Restarting an attempt in flight could burn an OTP or a login; ignore.
            break
        }
    }

    private mutating func quitRequested() {
        model.desired = .disconnected
        if model.quitRequestedAt == nil {
            model.quitRequestedAt = now
        }
        switch model.state {
        case let .connecting(context):
            abortAttempt(context, stop: .appQuit, then: .terminate)
        case let .connected(context):
            stopSession(context, reason: .appQuit, then: .terminate)
        case var .disconnecting(context):
            context.after = .terminate
            model.state = .disconnecting(context)
        case .idle, .needsSetup, .waitingRetry, .waitingNetwork, .failed:
            terminate()
        }
    }

    private mutating func resetForManualAction() {
        model.ledger.resetForManualAction()
        model.networkFailureStreak = 0
    }

    // MARK: - Gmail OTP session

    private mutating func otpSessionReady(_ attemptID: AttemptID) {
        guard case var .connecting(context) = model.state, context.attemptID == attemptID,
              context.stage == .preparingOTP else { return }
        context.stage = .tunnel
        model.state = .connecting(context)
        // The account this attempt checked readiness for; a profile change applies from the next attempt (§5.7).
        let profile = context.profile ?? model.profile
        model.runProfile = profile
        emit(.startTunnel(attemptID, profile))
    }

    private mutating func otpSessionFailed(_ attemptID: AttemptID, _ failure: OTPSessionFailure) {
        guard case var .connecting(context) = model.state, context.attemptID == attemptID,
              context.otpSessionOpen else { return }
        context.otpSessionOpen = false
        switch context.stage {
        case .checkingReadiness:
            break
        case .preparingOTP:
            // openfortivpn was never started (§6.2 IMAP rows).
            if failure == .authentication {
                fail(.gmailAppPassword)
            } else {
                networkFailure()
            }
        case .tunnel:
            // No OTP can arrive any more: stop now instead of waiting for the deadline; classify on exit.
            context.otpSessionFailure = failure
            if context.stopRequested == nil {
                context.stopRequested = .otpUnavailable
                emit(.stopTunnel(attemptID, .otpUnavailable))
            }
            model.state = .connecting(context)
        }
    }

    private mutating func otpCodeFound(_ attemptID: AttemptID, code: String) {
        guard case var .connecting(context) = model.state, context.attemptID == attemptID,
              context.stage == .tunnel, context.otpPromptedAt != nil, !context.otpSubmitted,
              context.stopRequested == nil, Self.isPlausibleOTP(code) else { return }
        context.otpSubmitted = true
        // The session trashes the used messages and logs out by itself after submission.
        context.otpSessionOpen = false
        model.state = .connecting(context)
        emit(.submitOTP(attemptID, code: code))
    }

    private mutating func otpFoundInJunk(_ attemptID: AttemptID) {
        if case var .connecting(context) = model.state, context.attemptID == attemptID {
            context.otpFoundInJunk = true
            model.state = .connecting(context)
            return
        }
        guard model.lastFailureAttemptID == attemptID else { return }
        let plain = ConnectionFailureReason.otpNotReceived(inJunk: false)
        let junk = ConnectionFailureReason.otpNotReceived(inJunk: true)
        switch model.state {
        case .failed(plain):
            model.state = .failed(junk)
        case var .waitingRetry(wait) where wait.reason == .authFailure(plain):
            wait.reason = .authFailure(junk)
            model.state = .waitingRetry(wait)
        case var .waitingNetwork(wait) where wait.reason == .authFailure(plain):
            wait.reason = .authFailure(junk)
            model.state = .waitingNetwork(wait)
        default:
            break
        }
        if model.ledger.lastAuthFailureReason == plain {
            model.ledger.lastAuthFailureReason = junk
        }
    }

    private static func isPlausibleOTP(_ code: String) -> Bool {
        (6...8).contains(code.utf8.count) && code.utf8.allSatisfy { (0x30...0x39).contains($0) }
    }

    // MARK: - Helper events

    private mutating func helperEvent(_ event: HelperEvent) {
        switch event.kind {
        case let .milestone(milestone, tunnelIP):
            milestoneReached(milestone, tunnelIP: tunnelIP, attemptID: event.attemptID)
        case let .exited(facts):
            exited(facts, attemptID: event.attemptID)
        }
    }

    private mutating func milestoneReached(_ milestone: Milestone, tunnelIP: String?, attemptID: AttemptID) {
        guard case var .connecting(context) = model.state, context.stage == .tunnel,
              context.attemptID == attemptID, context.stopRequested == nil else { return }
        let previous = context.milestone
        if let previous, milestone <= previous { return }
        context.milestone = milestone

        if (previous ?? .spawned) < .connectedToGateway, milestone >= .connectedToGateway {
            // TLS (and the pin check) passed: credentials go out now. Counts toward the 60-minute cap.
            model.ledger.recordAuthAttempt(at: now)
        }
        switch milestone {
        case .tunnelUp:
            enterConnected(context, tunnelIP: tunnelIP)
        case .otpPrompted:
            context.otpPromptedAt = now
            model.state = .connecting(context)
            emit(.beginOTPSearch(attemptID, promptedAt: now))
        case .spawned, .connectedToGateway, .otpSubmitted, .authenticated:
            model.state = .connecting(context)
        }
    }

    private mutating func enterConnected(_ attempt: AttemptContext, tunnelIP: String?) {
        var attempt = attempt
        // Only still open when the gateway never asked for an OTP.
        closeOTPSession(&attempt)
        model.networkFailureStreak = 0
        model.state = .connected(ConnectedContext(
            attemptID: attempt.attemptID,
            since: now,
            tunnelIP: tunnelIP,
            plannedReconnectAt: RetryPolicy.plannedReconnect(connectedAt: now, settings: model.settings, now: now),
            underlay: model.path?.underlay
        ))
    }

    private mutating func exited(_ facts: ExitFacts, attemptID: AttemptID) {
        switch model.state {
        case var .connecting(context) where context.stage == .tunnel && context.attemptID == attemptID:
            learnCertificates(from: facts)
            closeOTPSession(&context)
            model.lastFailureAttemptID = attemptID
            let input = ExitClassifier.Input(
                facts: facts,
                knownMilestone: context.milestone,
                certRetryUsed: context.certRetryUsed,
                appStop: context.stopRequested,
                otpSessionFailure: context.otpSessionFailure,
                otpFoundInJunk: context.otpFoundInJunk,
                pathChangedDuringAttempt: context.pathChanged,
                withinWakeGrace: startedWithinWakeGrace(context)
            )
            apply(ExitClassifier.classify(input), after: context)

        case let .connected(context) where context.attemptID == attemptID:
            learnCertificates(from: facts)
            let upDuration = facts.upDuration ?? now.timeIntervalSince(context.since)
            if upDuration >= RetryPolicy.stableSessionDuration {
                model.ledger.resetConsecutiveFailures()
            }
            model.lastFailureAttemptID = attemptID
            let input = ExitClassifier.Input(
                facts: facts,
                knownMilestone: .tunnelUp,
                certRetryUsed: true,
                observedUpDuration: upDuration
            )
            apply(ExitClassifier.classify(input), after: nil)

        case let .disconnecting(context) where context.attemptID == attemptID:
            learnCertificates(from: facts)
            finishStop(context, upDuration: facts.upDuration)

        default:
            // Stale: an attempt we already finished with.
            break
        }
    }

    private mutating func helperConnectRejected(_ attemptID: AttemptID, _ error: HelperError) {
        switch model.state {
        case var .connecting(context) where context.stage == .tunnel && context.attemptID == attemptID:
            closeOTPSession(&context)
            if let failure = context.otpSessionFailure {
                failure == .authentication ? fail(.gmailAppPassword) : networkFailure()
                return
            }
            switch error.code {
            case .binaryUnavailable, .spawnFailed, .invalidRequest, .notAuthorized:
                fail(.helper(error.code))
            case .busy, .wrongPhase, .unknownAttempt, .internalError:
                networkFailure()
            }
        case let .disconnecting(context) where context.attemptID == attemptID:
            finishStop(context, upDuration: nil)
        default:
            break
        }
    }

    private mutating func helperConnectionLost() {
        switch model.state {
        case var .connecting(context) where context.stage == .tunnel:
            closeOTPSession(&context)
            context.otpSessionFailure == .authentication ? fail(.gmailAppPassword) : networkFailure()
        case let .connected(context):
            if now.timeIntervalSince(context.since) >= RetryPolicy.stableSessionDuration {
                model.ledger.resetConsecutiveFailures()
            }
            networkFailure()
        case let .disconnecting(context):
            finishStop(context, upDuration: nil)
        default:
            break
        }
    }

    // MARK: - Timers

    private mutating func timerFired(_ timer: ConnectionTimer, generation: UInt64) {
        guard let slot = model.timers[timer], slot.generation == generation, !slot.fired else { return }
        model.timers[timer]?.fired = true
        switch timer {
        case .attemptDeadline: attemptDeadlinePassed()
        case .otpDeadline: otpDeadlinePassed()
        case .retry: retryDue()
        case .proactiveReconnect: proactiveReconnectDue()
        case .quitDeadline: terminate()
        }
    }

    private mutating func attemptDeadlinePassed() {
        guard case var .connecting(context) = model.state, context.stopRequested == nil else { return }
        guard context.stage == .tunnel else {
            // Readiness check or Gmail login hung; nothing is running yet.
            closeOTPSession(&context)
            networkFailure()
            return
        }
        context.stopRequested = .attemptTimeout
        emit(.stopTunnel(context.attemptID, .attemptTimeout))
        closeOTPSession(&context)
        model.state = .connecting(context)
    }

    private mutating func otpDeadlinePassed() {
        guard case var .connecting(context) = model.state, context.stage == .tunnel,
              context.otpPromptedAt != nil, !context.otpSubmitted, context.stopRequested == nil else { return }
        context.stopRequested = .otpTimeout
        emit(.stopTunnel(context.attemptID, .otpTimeout))
        closeOTPSession(&context, checkJunk: true)
        model.state = .connecting(context)
    }

    private mutating func retryDue() {
        guard case let .waitingRetry(wait) = model.state else { return }
        beginAttempt(.retry(wait.reason))
    }

    private mutating func proactiveReconnectDue() {
        guard case let .connected(context) = model.state else { return }
        stopSession(context, reason: .proactiveReconnect, then: .reconnect(.proactive))
    }

    // MARK: - Sleep, wake and network path

    private mutating func willSleep() {
        model.isSleeping = true
        switch model.state {
        case let .connecting(context):
            abortAttempt(context, stop: .sleep, then: .resumeAfterSleep)
        case .waitingRetry, .waitingNetwork:
            waitForNetwork(.sleep, delay: 0)
        case var .disconnecting(context):
            if case .reconnect = context.after {
                context.after = .resumeAfterSleep
                model.state = .disconnecting(context)
            }
        case .connected, .idle, .needsSetup, .failed:
            // A connected tunnel is kept across sleep (§2); its proactive timer is suspended.
            break
        }
    }

    private mutating func didWake(sleptFor: TimeInterval) {
        model.isSleeping = false
        model.lastWakeAt = now
        model.pendingWake = PendingWake(sleptFor: sleptFor)
    }

    private mutating func wakeSettled(_ observation: WakeObservation) {
        let pending = model.pendingWake
        model.pendingWake = nil
        recordPath(PathSnapshot(isSatisfied: observation.pathSatisfied, underlay: observation.underlay))
        switch model.state {
        case let .connected(context):
            // §6.3 judges a tunnel that was up across the sleep. A session that came up after the wake (the
            // path returned before the wake settled) is left alone: the observation may predate it, and it
            // was never asleep. Clearing `pendingWake` above re-arms its proactive timer either way.
            guard model.lastWakeAt.map({ context.since < $0 }) ?? true else { break }
            evaluateWake(context, observation, sleptFor: pending?.sleptFor ?? 0)
        case let .waitingNetwork(wait) where observation.pathSatisfied && !model.isSleeping:
            resumeFromNetworkWait(wait)
        default:
            break
        }
    }

    /// §6.3 rules for a tunnel that survived sleep, evaluated in order.
    private mutating func evaluateWake(_ context: ConnectedContext, _ observation: WakeObservation, sleptFor: TimeInterval) {
        var context = context
        guard observation.pathSatisfied else {
            // The network did not settle within 30 s: keep the tunnel, decide later.
            context.plannedReconnectAt = context.plannedReconnectAt.map {
                max($0, now.addingTimeInterval(RetryPolicy.minimumPlannedLeadTime))
            }
            model.state = .connected(context)
            rearmProactiveTimer()
            return
        }
        if let planned = context.plannedReconnectAt, planned <= now {
            restartSession(context, reason: .proactive)
        } else if !observation.processAlive || !observation.pppUp {
            restartSession(context, reason: .tunnelLost)
        } else if let old = context.underlay, let new = observation.underlay, old != new {
            restartSession(context, reason: .networkChanged)
        } else if model.settings.reconnectAfterSleepEnabled,
                  sleptFor >= TimeInterval(model.settings.minSleepMinutes) * 60 {
            restartSession(context, reason: .afterSleep)
        } else {
            if context.underlay == nil {
                context.underlay = observation.underlay
            }
            model.state = .connected(context)
            rearmProactiveTimer()
        }
    }

    private mutating func pathChanged(_ snapshot: PathSnapshot) {
        recordPath(snapshot)
        guard !model.isSleeping else { return }
        switch model.state {
        case let .waitingNetwork(wait) where snapshot.isSatisfied:
            resumeFromNetworkWait(wait)
        case var .connected(context) where model.pendingWake == nil:
            // While a wake is being evaluated, `wakeSettled` decides instead.
            guard snapshot.isSatisfied, let underlay = snapshot.underlay else { return }
            if let old = context.underlay {
                if old != underlay {
                    restartSession(context, reason: .networkChanged)
                }
            } else {
                context.underlay = underlay
                model.state = .connected(context)
            }
        default:
            // connecting / disconnecting ignore path events (§5.5); other states act on the next attempt.
            break
        }
    }

    /// Stores the snapshot and flags a running attempt whose network changed underneath it.
    private mutating func recordPath(_ snapshot: PathSnapshot) {
        if let previous = model.path, previous != snapshot, case var .connecting(context) = model.state {
            context.pathChanged = true
            model.state = .connecting(context)
        }
        model.path = snapshot
    }

    private mutating func resumeFromNetworkWait(_ wait: NetworkWait) {
        if wait.delayAfterPath > 0 {
            model.state = .waitingRetry(RetryWait(until: now.addingTimeInterval(wait.delayAfterPath), reason: wait.reason))
        } else {
            beginAttempt(wait.reason == .sleep ? .afterSleep : .retry(wait.reason))
        }
    }

    /// Forces a fresh generation for the proactive timer (timers may not survive sleep reliably).
    private mutating func rearmProactiveTimer() {
        model.timers[.proactiveReconnect]?.date = .distantPast
    }

    // MARK: - Attempts

    /// Starts a new attempt unless a gate applies: desire, sleep, the rolling cap, the 90 s gap after an
    /// authentication failure, or an unsatisfied network path.
    private mutating func beginAttempt(
        _ reason: AttemptReason,
        continuing previous: AttemptContext? = nil,
        readinessVerified: Bool = false
    ) {
        guard model.desired == .connected else {
            model.state = .idle
            return
        }
        if model.isSleeping {
            waitForNetwork(.sleep, delay: 0)
            return
        }
        model.ledger.prune(asOf: now)
        if !reason.isManual, model.ledger.recentAuthAttempts.count >= RetryPolicy.maxAuthAttemptsPerWindow {
            fail(.tooManyReconnects)
            return
        }
        if let cooldownEnd = model.ledger.cooldownEnd, now < cooldownEnd {
            let waitReason: RetryReason = if case let .retry(.authFailure(failure)) = reason {
                .authFailure(failure)
            } else {
                .authCooldown
            }
            model.state = .waitingRetry(RetryWait(until: cooldownEnd, reason: waitReason))
            return
        }
        if model.path?.isSatisfied == false {
            let waitReason: RetryReason = switch reason {
            case let .retry(retryReason): retryReason
            case .afterSleep: .sleep
            default: .network
            }
            waitForNetwork(waitReason, delay: 0)
            return
        }

        let attemptID = model.attemptIDs.next()
        var context = AttemptContext(
            attemptID: attemptID,
            reason: reason,
            chainID: previous?.chainID ?? attemptID,
            certRetryUsed: previous != nil,
            startedAt: now,
            stage: .checkingReadiness
        )
        context.profile = model.profile
        if readinessVerified {
            context.stage = .preparingOTP
            context.otpSessionOpen = true
            emit(.startOTPSession(attemptID))
        } else {
            emit(.checkReadiness)
        }
        model.state = .connecting(context)
    }

    /// Ends an attempt the app no longer wants (user, sleep, quit).
    private mutating func abortAttempt(_ context: AttemptContext, stop: StopReason, then after: AfterStop) {
        var context = context
        guard context.stage == .tunnel else {
            closeOTPSession(&context)
            settle(after)
            return
        }
        if context.stopRequested == nil {
            emit(.stopTunnel(context.attemptID, stop))
        }
        closeOTPSession(&context)
        model.state = .disconnecting(DisconnectContext(
            attemptID: context.attemptID,
            reason: context.stopRequested ?? stop,
            after: after,
            connectedSince: nil
        ))
    }

    /// Break-before-make: stop the tunnel; `after` runs once `.exited` arrives.
    private mutating func stopSession(_ context: ConnectedContext, reason: StopReason, then after: AfterStop) {
        emit(.stopTunnel(context.attemptID, reason))
        model.state = .disconnecting(DisconnectContext(
            attemptID: context.attemptID,
            reason: reason,
            after: after,
            connectedSince: context.since
        ))
    }

    private mutating func restartSession(_ context: ConnectedContext, reason: AttemptReason) {
        stopSession(context, reason: reason == .proactive ? .proactiveReconnect : .reconnect, then: .reconnect(reason))
    }

    private mutating func finishStop(_ context: DisconnectContext, upDuration: TimeInterval?) {
        let up = upDuration ?? context.connectedSince.map { now.timeIntervalSince($0) }
        if let up, up >= RetryPolicy.stableSessionDuration {
            model.ledger.resetConsecutiveFailures()
        }
        settle(context.after)
    }

    private mutating func settle(_ after: AfterStop) {
        switch after {
        case .idle:
            model.state = .idle
        case let .reconnect(reason):
            beginAttempt(reason)
        case .resumeAfterSleep:
            beginAttempt(.afterSleep)
        case .terminate:
            model.state = .idle
            terminate()
        }
    }

    private mutating func terminate() {
        model.isTerminated = true
        terminates = true
    }

    private mutating func closeOTPSession(_ context: inout AttemptContext, checkJunk: Bool = false) {
        guard context.otpSessionOpen else { return }
        context.otpSessionOpen = false
        emit(.closeOTPSession(context.attemptID, checkJunk: checkJunk))
    }

    private func startedWithinWakeGrace(_ context: AttemptContext) -> Bool {
        guard let wake = model.lastWakeAt else { return false }
        return context.startedAt <= wake.addingTimeInterval(RetryPolicy.wakeGracePeriod)
    }

    /// `true` when the run that just exited talked to the gateway `model.profile` points at now.
    private var runIsForCurrentGateway: Bool {
        !(model.runProfile?.isDifferentGateway(from: model.profile) ?? false)
    }

    private mutating func learnCertificates(from facts: ExitFacts) {
        let learned = TrustedCertPins.normalized(facts.certRejectedDigests)
        // A run started before a host/port change presented the old gateway's certificate (§5.4).
        guard !learned.isEmpty, runIsForCurrentGateway else { return }
        let merged = TrustedCertPins.merging(learned, into: model.profile.trustedCerts)
        guard merged != model.profile.trustedCerts else { return }
        model.profile.trustedCerts = merged
        emit(.persistTrustedCerts(merged))
    }

    // MARK: - Outcomes

    private mutating func apply(_ outcome: ExitClassifier.Outcome, after attempt: AttemptContext?) {
        switch outcome {
        case .certificateRetry:
            if let attempt {
                // The chain's one immediate retry belongs to a gateway; a new one starts its own chain.
                beginAttempt(.certificateRetry, continuing: runIsForCurrentGateway ? attempt : nil)
            } else {
                networkFailure()
            }
        case .serverSessionEnded:
            model.ledger.resetConsecutiveFailures()
            scheduleRetry(.serverSessionEnded, after: RetryPolicy.serverSessionEndDelay)
        case .networkError:
            networkFailure()
        case let .authFailure(reason):
            authFailure(reason)
        case let .fatal(reason):
            fail(reason)
        }
    }

    private mutating func networkFailure() {
        model.networkFailureStreak += 1
        scheduleRetry(.network, after: RetryPolicy.networkBackoff(afterFailures: model.networkFailureStreak))
    }

    private mutating func authFailure(_ reason: ConnectionFailureReason) {
        model.ledger.recordAuthFailure(reason, at: now)
        if model.ledger.consecutiveAuthFailures >= RetryPolicy.maxConsecutiveAuthFailures {
            fail(reason)
        } else {
            scheduleRetry(.authFailure(reason), after: RetryPolicy.authFailureRetryDelay)
        }
    }

    private mutating func scheduleRetry(_ reason: RetryReason, after delay: TimeInterval) {
        guard model.desired == .connected else {
            model.state = .idle
            return
        }
        if model.isSleeping {
            waitForNetwork(.sleep, delay: 0)
        } else if model.path?.isSatisfied == false {
            // Authentication retries are gated by the ledger's cooldown instead of a fresh delay.
            let isAuth = if case .authFailure = reason { true } else { false }
            waitForNetwork(reason, delay: isAuth ? 0 : delay)
        } else {
            model.state = .waitingRetry(RetryWait(until: now.addingTimeInterval(delay), reason: reason))
        }
    }

    private mutating func waitForNetwork(_ reason: RetryReason, delay: TimeInterval) {
        model.state = .waitingNetwork(NetworkWait(reason: reason, delayAfterPath: delay))
    }

    private mutating func fail(_ reason: ConnectionFailureReason) {
        model.state = .failed(reason)
    }
}
