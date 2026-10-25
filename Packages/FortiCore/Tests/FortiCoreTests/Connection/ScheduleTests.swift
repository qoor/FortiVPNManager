import Foundation
import Testing
import FortiShared
@testable import FortiCore

/// Monday to Friday, 09:00-18:00, reconnects stop after 120 minutes away.
private let weekdayHours = ConnectionSchedule(weekdays: [2, 3, 4, 5, 6], startMinute: 9 * 60, endMinute: 18 * 60, awayMinutes: 120)

/// March 2030 in Seoul: the 18th is a Monday, the 22nd a Friday, the 23rd a Saturday.
private func at(_ day: Int, _ hour: Int, _ minute: Int = 0, _ second: Int = 0) -> Date {
    ReducerFixture.date(2030, 3, day, hour, minute).addingTimeInterval(TimeInterval(second))
}

private func harness(at date: Date, schedule: ConnectionSchedule? = weekdayHours) -> ReducerHarness {
    var h = ReducerHarness(settings: ConnectionSettings(schedule: schedule))
    h.now = date
    return h
}

private func observe(alive: Bool = true, ppp: Bool = true) -> WakeObservation {
    WakeObservation(processAlive: alive, pppUp: ppp, underlay: ReducerFixture.wifi, pathSatisfied: true)
}

@Suite("Connection schedule (§5.4 schedule)")
struct ScheduleTests {
    // MARK: Start, end and launch

    @Test func launchInsideTheHoursConnects() {
        var h = harness(at: at(18, 10))
        let effects = h.launch()
        #expect(h.attempt?.reason == .launch)
        #expect(effects.scheduled(.schedule) == at(18, 18))
    }

    @Test func launchOutsideTheHoursWaitsForTheStart() throws {
        var h = harness(at: at(18, 7))
        let effects = h.launch()
        #expect(h.model.state == .idle)
        #expect(h.model.desired == .disconnected)
        #expect(effects.scheduled(.schedule) == at(18, 9))

        try h.fire(.schedule)
        #expect(h.model.desired == .connected)
        #expect(h.attempt?.reason == .scheduled)
        try h.runToConnected()
        #expect(h.model.timers[.schedule]?.date == at(18, 18))
    }

    @Test func launchWithoutAutoConnectStaysOffEvenInsideTheHours() {
        var h = harness(at: at(18, 10))
        h.launch(autoConnect: false)
        #expect(h.model.state == .idle)
    }

    @Test func startClearsAFailureLikeConnect() throws {
        var h = harness(at: at(18, 7))
        h.launch()
        h.send(.userConnect)
        let id = try h.startTunnel()
        h.send(.helperConnectRejected(id, HelperError(.binaryUnavailable, "missing")))
        #expect(h.model.state == .failed(.helper(.binaryUnavailable)))

        try h.fire(.schedule)
        #expect(h.attempt?.reason == .scheduled)
    }

    @Test func endKeepsTheTunnelAndReconnectsWhileTheUserIsPresent() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        let effects = try h.fire(.schedule)
        #expect(h.now == at(18, 18))
        #expect(effects.stopReasons.isEmpty)
        #expect(h.connected != nil)
        #expect(h.model.timers[.proactiveReconnect] != nil)

        h.advance(3600)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 7200))
        try h.fire(.retry)
        #expect(h.attempt?.reason == .retry(.serverSessionEnded))
    }

    @Test func manualConnectOutsideTheHoursKeepsReconnecting() throws {
        var h = harness(at: at(23, 10))
        h.launch()
        #expect(h.model.state == .idle)
        h.send(.userConnect)
        try h.runToConnected()
        let id = try #require(h.connected?.attemptID)

        let effects = try h.fire(.proactiveReconnect)
        #expect(effects.stopReasons == [.proactiveReconnect])
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.proactiveReconnect), up: 175 * 60), id: id)
        #expect(h.attempt?.reason == .proactive)
    }

    // MARK: Away outside the hours

    @Test func awayOutsideTheHoursHoldsTheReconnectUntilTheUserReturns() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.advance(600)
        let away = h.send(.presenceChanged(away: true))
        #expect(away.cancels(.proactiveReconnect))
        #expect(away.scheduled(.awayLimit) == at(18, 20, 10))

        h.advance(1800)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 5400))
        try h.fire(.retry)
        #expect(h.retryWait == RetryWait(until: at(18, 20, 10), reason: .userAway))
        #expect(h.model.icon == .disconnected)
        #expect(h.model.scheduleOutlook(now: h.now) == .holdingWhileAway(until: at(18, 20, 10)))
        #expect(!h.effects.startsOTPSession && !h.effects.contains(.checkReadiness))

        h.advance(1200)
        h.send(.presenceChanged(away: false))
        #expect(h.attempt?.reason == .retry(.userAway))
    }

    @Test func insideTheHoursAwayChangesNothing() throws {
        var h = harness(at: at(18, 10))
        try h.launchConnected()
        h.advance(3600)
        h.send(.presenceChanged(away: true))
        #expect(h.model.timers[.proactiveReconnect] != nil)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 3600))
        try h.fire(.retry)
        #expect(h.attempt?.reason == .retry(.serverSessionEnded))
    }

    @Test func awayLimitStopsReconnectsButKeepsTheTunnel() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.advance(600)
        h.send(.presenceChanged(away: true))

        let limit = try h.fire(.awayLimit)
        #expect(h.now == at(18, 20, 10))
        #expect(limit.stopReasons.isEmpty)
        #expect(h.connected != nil)
        #expect(h.model.reconnectSuspended)
        #expect(h.model.scheduleOutlook(now: h.now) == .suspended)

        // Back before the server ends the session: it is kept, but the next one is not replaced.
        h.advance(600)
        h.send(.presenceChanged(away: false))
        #expect(h.model.timers[.proactiveReconnect] == nil)
        h.advance(600)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 3 * 3600))
        try h.fire(.retry)
        #expect(h.model.state == .idle)
        #expect(h.model.desired == .disconnected)
        #expect(h.model.scheduleOutlook(now: h.now) == .connects(at: at(19, 9)))
    }

    @Test func aHeldReconnectIsGivenUpAtTheLimit() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.advance(600)
        h.send(.presenceChanged(away: true))
        h.advance(1800)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 5400))
        try h.fire(.retry)
        #expect(h.retryWait?.reason == .userAway)

        try h.fire(.retry)
        #expect(h.model.state == .idle)
        #expect(h.model.desired == .disconnected)
        #expect(h.model.reconnectSuspended)
        h.send(.presenceChanged(away: false))
        #expect(h.model.state == .idle)
    }

    @Test func awayTimeBeforeTheEndCountsAtTheEnd() throws {
        var h = harness(at: at(18, 15))
        try h.launchConnected()
        h.now = at(18, 15, 30)
        h.send(.presenceChanged(away: true))
        try h.fire(.awayLimit)
        #expect(!h.model.reconnectSuspended)

        try h.fire(.schedule)
        #expect(h.now == at(18, 18))
        #expect(h.model.reconnectSuspended)
        #expect(h.connected != nil)
    }

    @Test func theNextStartClearsTheSuspension() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.send(.presenceChanged(away: true))
        try h.fire(.awayLimit)
        h.exit(ReducerFixture.facts(.tunnelUp, up: 3 * 3600))
        try h.fire(.retry)
        #expect(h.model.state == .idle)

        try h.fire(.schedule)
        #expect(h.now == at(19, 9))
        #expect(!h.model.reconnectSuspended)
        #expect(h.attempt?.reason == .scheduled)
    }

    @Test func reconnectNowClearsTheSuspension() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.send(.presenceChanged(away: true))
        try h.fire(.awayLimit)
        h.send(.presenceChanged(away: false))

        let effects = h.send(.userReconnectNow)
        #expect(effects.stopReasons == [.reconnect])
        #expect(!h.model.reconnectSuspended)
    }

    // MARK: Sleep

    @Test func aLongSleepOutsideTheHoursStopsReconnectsBeforeTheUserUnlocks() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)
        h.now = at(18, 17, 30)
        h.send(.presenceChanged(away: true))
        h.send(.willSleep)

        h.now = at(18, 21)
        let wake = h.send(.didWake(sleptFor: 3.5 * 3600))
        #expect(wake.stopReasons.isEmpty)
        #expect(h.model.reconnectSuspended)
        try h.fire(.schedule)
        h.send(.presenceChanged(away: false))

        h.advance(5)
        let settled = h.send(.wakeSettled(observe(alive: false, ppp: false)))
        #expect(settled.stopReasons == [.reconnect])
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.reconnect), up: 3600), id: id)
        #expect(h.model.state == .idle)
        #expect(h.model.desired == .disconnected)
    }

    @Test func aWorkingTunnelIsKeptOnWakeWhileReconnectsWait() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        let planned = try #require(h.connected?.plannedReconnectAt)
        try h.fire(.schedule)
        h.now = at(18, 19)
        h.send(.presenceChanged(away: true))
        h.send(.willSleep)
        h.now = at(18, 20)
        h.send(.didWake(sleptFor: 3600))
        #expect(!h.model.reconnectSuspended)

        let settled = h.send(.wakeSettled(observe()))
        #expect(settled.stopReasons.isEmpty)
        #expect(h.connected != nil)
        #expect(h.model.timers[.proactiveReconnect] == nil)

        let back = h.send(.presenceChanged(away: false))
        #expect(back.scheduled(.proactiveReconnect) == planned)
        let effects = try h.fire(.proactiveReconnect)
        #expect(effects.stopReasons == [.proactiveReconnect])
    }

    @Test func aMissedWeekendAppliesOnlyTheLatestBoundary() throws {
        var h = harness(at: at(22, 17))
        try h.launchConnected()
        let id = try #require(h.connected?.attemptID)
        h.send(.userDisconnect)
        h.exit(ReducerFixture.facts(.tunnelUp, initiator: .helper(.userDisconnect), up: 600), id: id)
        #expect(h.model.state == .idle)
        h.send(.presenceChanged(away: true))
        h.send(.willSleep)

        h.now = at(25, 9, 30)
        h.send(.didWake(sleptFor: 64 * 3600))
        try h.fire(.schedule)
        #expect(h.now == at(25, 9, 30))
        #expect(h.attempt?.reason == .scheduled)
        #expect(h.model.timers[.schedule]?.date == at(25, 18))
    }

    @Test func sleepKeepsAHeldReconnectWaitingForTheUser() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.send(.presenceChanged(away: true))
        h.exit(ReducerFixture.facts(.tunnelUp, up: 3600))
        try h.fire(.retry)
        #expect(h.retryWait?.reason == .userAway)

        h.send(.willSleep)
        #expect(h.retryWait?.reason == .userAway)
    }

    // MARK: Settings

    @Test func editingTheScheduleActsFromTheNextBoundary() {
        var h = harness(at: at(18, 20))
        h.launch()
        #expect(h.model.state == .idle)
        let later = ConnectionSchedule(weekdays: weekdayHours.weekdays, startMinute: 9 * 60, endMinute: 21 * 60, awayMinutes: 120)
        let effects = h.send(.settingsChanged(ConnectionSettings(schedule: later)))
        #expect(h.model.state == .idle)
        #expect(effects.scheduled(.schedule) == at(18, 21))
    }

    @Test func removingTheScheduleResumesAHeldReconnect() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.send(.presenceChanged(away: true))
        h.exit(ReducerFixture.facts(.tunnelUp, up: 3600))
        try h.fire(.retry)
        #expect(h.retryWait?.reason == .userAway)

        let effects = h.send(.settingsChanged(ConnectionSettings()))
        #expect(h.attempt?.reason == .retry(.userAway))
        #expect(effects.cancels(.schedule))
        #expect(effects.cancels(.awayLimit))
    }

    @Test func aShorterAwayLimitAppliesAtOnce() throws {
        var h = harness(at: at(18, 17))
        try h.launchConnected()
        try h.fire(.schedule)
        h.send(.presenceChanged(away: true))
        h.advance(3600)
        let shorter = ConnectionSchedule(weekdays: weekdayHours.weekdays, startMinute: 9 * 60, endMinute: 18 * 60, awayMinutes: 30)
        h.send(.settingsChanged(ConnectionSettings(schedule: shorter)))
        #expect(h.model.reconnectSuspended)
        #expect(h.connected != nil)
    }

    @Test func aTimeZoneChangeRereadsTheTimes() throws {
        var h = harness(at: at(18, 8))
        h.launch()
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = try #require(TimeZone(identifier: "UTC"))
        let effects = h.send(.calendarChanged(utc))
        // Monday 08:00 in Seoul is Sunday 23:00 UTC; the next start is Monday 09:00 UTC.
        #expect(effects.scheduled(.schedule) == ReducerFixture.date(2030, 3, 18, 9, calendar: utc))
    }

    @Test func outlookFollowsTheDay() throws {
        var h = harness(at: at(18, 7))
        h.launch()
        #expect(h.model.scheduleOutlook(now: h.now) == .connects(at: at(18, 9)))
        try h.fire(.schedule)
        try h.runToConnected()
        #expect(h.model.scheduleOutlook(now: h.now) == .withinHours(end: at(18, 18)))
        try h.fire(.schedule)
        #expect(h.model.scheduleOutlook(now: h.now) == .outsideHoursPresent)

        var none = harness(at: at(18, 7), schedule: nil)
        none.launch()
        #expect(none.model.scheduleOutlook(now: none.now) == nil)
        let empty = ConnectionSchedule(weekdays: [], startMinute: 540, endMinute: 1080, awayMinutes: 120)
        var noDays = harness(at: at(18, 7), schedule: empty)
        noDays.launch()
        #expect(noDays.model.state == .idle)
        #expect(noDays.model.scheduleOutlook(now: noDays.now) == .noStart)
        #expect(noDays.model.timers[.schedule] == nil)
    }
}
