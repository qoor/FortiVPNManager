import Foundation

/// How the UI writes times: relative to `now`, in the locale and calendar (with its time zone) of
/// the view's environment. Times arrive as absolute dates; every countdown and relative time is
/// drawn here.
struct TimeStyle {
    var now: Date
    var locale: Locale
    var calendar: Calendar

    /// A time value that came with a kind, such as a time limit: "50 seconds", "2 minutes".
    func spelledOut(seconds: Int) -> String {
        Duration.seconds(seconds).formatted(.units(width: .wide).locale(locale))
    }

    /// Whole minutes: "1 hr 23 min", "45 min", "2 hr", or "less than a minute".
    func duration(_ interval: TimeInterval) -> String {
        let minutes = Int((interval / 60).rounded(.down))
        guard minutes >= 1 else { return String(localized: "less than a minute") }
        return Duration.seconds(minutes * 60)
            .formatted(.units(allowed: [.hours, .minutes], width: .condensedAbbreviated).locale(locale))
    }

    /// "Reconnects in about 1 hr 32 min", or "Reconnects soon" within the last minute.
    func plannedReconnect(_ date: Date) -> String {
        let remaining = date.timeIntervalSince(now)
        return remaining < 60
            ? String(localized: "Reconnects soon")
            : String(localized: "Reconnects in about \(duration(remaining))")
    }

    /// "1:12" (minutes:seconds, rounded up).
    func countdown(_ interval: TimeInterval) -> String {
        let seconds = max(0, Int(interval.rounded(.up)))
        let (minutes, rest) = seconds.quotientAndRemainder(dividingBy: 60)
        return rest < 10 ? "\(minutes):0\(rest)" : "\(minutes):\(rest)"
    }

    /// "2:03 PM" today, otherwise with the weekday ("Mon 9:00 AM"); for times within a week.
    func dayTime(_ date: Date) -> String {
        if calendar.isDate(date, inSameDayAs: now) {
            return date.formatted(style(date: .omitted, time: .shortened))
        }
        return date.formatted(style(date: nil, time: nil).weekday(.abbreviated).hour().minute())
    }

    /// "2:03 PM", with the date when it is not today ("Oct 14, 2026 at 11:58 PM").
    func clockTime(_ date: Date) -> String {
        let dateStyle: Date.FormatStyle.DateStyle = calendar.isDate(date, inSameDayAs: now) ? .omitted : .abbreviated
        return date.formatted(style(date: dateStyle, time: .shortened))
    }

    private func style(date: Date.FormatStyle.DateStyle?, time: Date.FormatStyle.TimeStyle?) -> Date.FormatStyle {
        Date.FormatStyle(date: date, time: time, locale: locale, calendar: calendar, timeZone: calendar.timeZone)
    }
}
