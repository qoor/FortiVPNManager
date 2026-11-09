import Foundation

extension AppInternal.ScheduleOutlook {
    /// One line for the menu ("Schedule: …") and the Connection tab.
    func summary(time: TimeStyle) -> String {
        switch self {
        case let .connects(at):
            String(localized: "Connects \(time.dayTime(at))")
        case .noDays:
            String(localized: "No days selected")
        case let .withinHours(end):
            String(localized: "After \(time.clockTime(end)), reconnects wait while you're away")
        case .outsideHoursPresent:
            String(localized: "Reconnects wait while you're away")
        case let .holdingWhileAway(until):
            String(localized: "Reconnects stop if you're still away at \(time.clockTime(until))")
        case .suspended:
            String(localized: "Won't reconnect after this connection ends")
        }
    }
}

extension AppInternal.ScheduleHoursError {
    var message: String {
        switch self {
        case .endNotAfterStart: String(localized: "The end time must be later than the start time.")
        case let .notSaved(error): error.message
        }
    }
}

extension AppInternal.Weekday {
    /// Short weekday name for the day buttons.
    var shortName: String {
        switch self {
        case .mon: String(localized: "Mon")
        case .tue: String(localized: "Tue")
        case .wed: String(localized: "Wed")
        case .thu: String(localized: "Thu")
        case .fri: String(localized: "Fri")
        case .sat: String(localized: "Sat")
        case .sun: String(localized: "Sun")
        }
    }
}
