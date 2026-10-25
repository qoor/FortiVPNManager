import Foundation
import Testing
@testable import FortiCore

@Suite("Schedule window math (§5.4 schedule)")
struct ConnectionScheduleTests {
    /// Monday to Friday, 09:00-18:00.
    static let weekdays = ConnectionSchedule(weekdays: [2, 3, 4, 5, 6], startMinute: 9 * 60, endMinute: 18 * 60, awayMinutes: 120)
    static let calendar = ReducerFixture.calendar

    static func at(_ day: Int, _ hour: Int, _ minute: Int = 0) -> Date {
        // March 2030: the 18th is a Monday, the 23rd a Saturday.
        ReducerFixture.date(2030, 3, day, hour, minute)
    }

    @Test("Active from the start (inclusive) to the end (exclusive) on selected days", arguments: [
        (18, 8, 59, false), (18, 9, 0, true), (18, 12, 0, true), (18, 17, 59, true), (18, 18, 0, false),
        (22, 9, 30, true), (23, 10, 0, false), (24, 10, 0, false), (25, 9, 0, true),
    ])
    func activeWindow(day: Int, hour: Int, minute: Int, expected: Bool) {
        #expect(Self.weekdays.isActive(at: Self.at(day, hour, minute), calendar: Self.calendar) == expected)
    }

    @Test func latestBoundaryLooksBackAcrossTheWeekend() {
        let saturday = Self.at(23, 10)
        #expect(Self.weekdays.latestBoundary(atOrBefore: saturday, calendar: Self.calendar)
            == .init(date: Self.at(22, 18), edge: .end))
        #expect(Self.weekdays.latestBoundary(atOrBefore: Self.at(18, 9), calendar: Self.calendar)
            == .init(date: Self.at(18, 9), edge: .start))
    }

    @Test func nextBoundaryIsStrictlyLater() {
        #expect(Self.weekdays.nextBoundary(after: Self.at(22, 18), calendar: Self.calendar)
            == .init(date: Self.at(25, 9), edge: .start))
        #expect(Self.weekdays.nextBoundary(after: Self.at(18, 9), calendar: Self.calendar)
            == .init(date: Self.at(18, 18), edge: .end))
        #expect(Self.weekdays.nextStart(after: Self.at(18, 9), calendar: Self.calendar) == Self.at(19, 9))
        #expect(Self.weekdays.nextStart(after: Self.at(22, 12), calendar: Self.calendar) == Self.at(25, 9))
    }

    @Test func singleDayRepeatsWeekly() {
        let sundays = ConnectionSchedule(weekdays: [1], startMinute: 0, endMinute: 1439, awayMinutes: 120)
        #expect(sundays.nextStart(after: Self.at(24, 0), calendar: Self.calendar) == Self.at(31, 0))
        #expect(sundays.latestBoundary(atOrBefore: Self.at(23, 12), calendar: Self.calendar)
            == .init(date: Self.at(17, 23, 59), edge: .end))
    }

    @Test func noWeekdaysNeverActive() {
        let none = ConnectionSchedule(weekdays: [], startMinute: 540, endMinute: 1080, awayMinutes: 120)
        #expect(!none.isActive(at: Self.at(18, 12), calendar: Self.calendar))
        #expect(none.latestBoundary(atOrBefore: Self.at(18, 12), calendar: Self.calendar) == nil)
        #expect(none.nextBoundary(after: Self.at(18, 12), calendar: Self.calendar) == nil)
        #expect(none.nextStart(after: Self.at(18, 12), calendar: Self.calendar) == nil)
    }

    @Test("Times stay on the local clock across DST changes", arguments: [(2026, 3, 8), (2026, 11, 1)])
    func daylightSavingDays(year: Int, month: Int, day: Int) throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try #require(TimeZone(identifier: "America/New_York"))
        let sundays = ConnectionSchedule(weekdays: [1], startMinute: 9 * 60, endMinute: 18 * 60, awayMinutes: 120)
        let midnight = ReducerFixture.date(year, month, day, 0, calendar: calendar)
        let start = try #require(sundays.nextBoundary(after: midnight, calendar: calendar))
        let end = try #require(sundays.nextBoundary(after: start.date, calendar: calendar))
        #expect(start.edge == .start && end.edge == .end)
        #expect(calendar.dateComponents([.hour, .minute], from: start.date) == DateComponents(hour: 9, minute: 0))
        #expect(calendar.dateComponents([.hour, .minute], from: end.date) == DateComponents(hour: 18, minute: 0))
        #expect(end.date.timeIntervalSince(start.date) == 9 * 3600)
    }
}
