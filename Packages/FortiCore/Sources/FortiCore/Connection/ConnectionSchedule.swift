import Foundation

/// `connection.schedule` as the reducer sees it (§5.4 schedule): one daily window on some weekdays.
///
/// The window's start connects (like "Connect"); its end never disconnects. Outside the window,
/// automatic reconnects wait while the user is away and stop for good once the user has been away
/// for `awayMinutes`.
public struct ConnectionSchedule: Sendable, Equatable, Codable {
    /// `Calendar` weekday numbers: 1 = Sunday … 7 = Saturday.
    public var weekdays: Set<Int>
    /// Minutes after local midnight, `0..<endMinute`.
    public var startMinute: Int
    /// Minutes after local midnight, `startMinute + 1 ... 1439`.
    public var endMinute: Int
    public var awayMinutes: Int

    public init(weekdays: Set<Int>, startMinute: Int, endMinute: Int, awayMinutes: Int) {
        self.weekdays = weekdays
        self.startMinute = startMinute
        self.endMinute = endMinute
        self.awayMinutes = awayMinutes
    }

    /// Away time after which automatic reconnects stop.
    public var awayLimit: TimeInterval { TimeInterval(awayMinutes) * 60 }

    /// A window start or end on a given day.
    public struct Boundary: Sendable, Equatable {
        public enum Edge: Sendable, Equatable {
            case start
            case end
        }

        public var date: Date
        public var edge: Edge

        public init(date: Date, edge: Edge) {
            self.date = date
            self.edge = edge
        }
    }

    /// `true` from a window's start (inclusive) to its end (exclusive).
    public func isActive(at date: Date, calendar: Calendar) -> Bool {
        latestBoundary(atOrBefore: date, calendar: calendar)?.edge == .start
    }

    /// The most recent start or end at or before `date`, or `nil` when no weekday is selected.
    public func latestBoundary(atOrBefore date: Date, calendar: Calendar) -> Boundary? {
        boundaries(around: date, calendar: calendar).last { $0.date <= date }
    }

    /// The first start or end strictly after `date`, or `nil` when no weekday is selected.
    public func nextBoundary(after date: Date, calendar: Calendar) -> Boundary? {
        boundaries(around: date, calendar: calendar).first { $0.date > date }
    }

    /// The first start strictly after `date`.
    public func nextStart(after date: Date, calendar: Calendar) -> Date? {
        boundaries(around: date, calendar: calendar).first { $0.date > date && $0.edge == .start }?.date
    }

    /// Every boundary from 8 days before to 8 days after `date`, in order. A week plus a day on
    /// either side always contains the neighbours of `date` when at least one weekday is selected.
    private func boundaries(around date: Date, calendar: Calendar) -> [Boundary] {
        guard !weekdays.isEmpty else { return [] }
        let today = calendar.startOfDay(for: date)
        var result: [Boundary] = []
        for offset in -8...8 {
            guard let day = calendar.date(byAdding: .day, value: offset, to: today),
                  weekdays.contains(calendar.component(.weekday, from: day)),
                  let start = Self.time(startMinute, on: day, calendar: calendar),
                  let end = Self.time(endMinute, on: day, calendar: calendar),
                  start < end
            else { continue }
            result.append(Boundary(date: start, edge: .start))
            result.append(Boundary(date: end, edge: .end))
        }
        return result
    }

    /// `minute` after midnight on `day`; a time skipped by a DST change moves to the next valid time.
    private static func time(_ minute: Int, on day: Date, calendar: Calendar) -> Date? {
        calendar.date(bySettingHour: minute / 60, minute: minute % 60, second: 0, of: day)
    }
}
