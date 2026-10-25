import Foundation
import Testing
@testable import FortiConfig

@Suite("connection.schedule in config.yaml")
struct ScheduleConfigTests {
    @Test func defaultsAreOffOnWeekdaysNineToSix() {
        let schedule = AppConfig().connection.schedule
        #expect(!schedule.enabled)
        #expect(schedule.days == [.mon, .tue, .wed, .thu, .fri])
        #expect(schedule.start.description == "09:00")
        #expect(schedule.end.description == "18:00")
        #expect(schedule.awayMinutes == 120)
    }

    @Test("Times are text in H:mm or HH:mm, quoted or plain", arguments: [
        ("\"09:00\"", 540), ("'9:00'", 540), ("9:05", 545), ("18:00", 1080), ("0:00", 0), ("\"23:58\"", 1438),
    ])
    func acceptedTimes(raw: String, minutes: Int) throws {
        let config = try decodeYAML("connection:\n  schedule: { start: \(raw), end: 23:59 }\n").config
        #expect(config.connection.schedule.start.minutes == minutes)
        #expect(config.connection.schedule.end.minutes == 1439)
    }

    @Test("Other spellings are errors at the value", arguments: [
        "\"24:00\"", "\"9:60\"", "\"09:0\"", "\"0900\"", "\"9am\"", "\"\"", "\" 9:00\"", "\"09:00:00\"", "\"-1:00\"", "\"\u{FF19}:00\"",
    ])
    func rejectedTimes(raw: String) throws {
        let error = try #require(loadError("connection:\n  schedule:\n    start: \(raw)\n"))
        guard case .invalidTime(let path, _) = error.kind else {
            Issue.record("expected invalidTime, got \(error.kind)")
            return
        }
        #expect(path == "connection.schedule.start")
        #expect(error.location == loc(3, 12))
        #expect(error.message.contains("HH:mm"))
    }

    @Test func numbersAreNotTimes() throws {
        let error = try #require(loadError("connection:\n  schedule: { end: 1080 }\n"))
        #expect(error.kind == .invalidTime(path: "connection.schedule.end", raw: "1080"))
    }

    @Test func endMustBeLaterThanStart() throws {
        let yaml = """
            connection:
              schedule: { start: "18:00", end: "09:00" }
            """
        let error = try #require(loadError(yaml))
        #expect(error.kind == .scheduleEndNotAfterStart(start: TimeOfDay(hour: 18, minute: 0), end: TimeOfDay(hour: 9, minute: 0)))
        #expect(error.location == loc(2, 36))
        #expect(loadError("connection:\n  schedule: { start: \"18:00\" }\n")?.kind
            == .scheduleEndNotAfterStart(start: TimeOfDay(hour: 18, minute: 0), end: TimeOfDay(hour: 18, minute: 0)))
        #expect(loadError("connection:\n  schedule: { end: \"09:01\" }\n") == nil)
    }

    @Test func daysAreNormalized() throws {
        let config = try decodeYAML("connection:\n  schedule: { days: [sun, fri, mon, \"fri\"] }\n").config
        #expect(config.connection.schedule.days == [.mon, .fri, .sun])
        #expect(try decodeYAML("connection:\n  schedule: { days: [] }\n").config.connection.schedule.days == [])
        #expect(try decodeYAML("connection:\n  schedule: { days: ~ }\n").config.connection.schedule.days
            == AppConfig().connection.schedule.days)
    }

    @Test("Unknown day names are errors at the item", arguments: ["Mon", "monday", "1", "~", "true"])
    func unknownDays(raw: String) throws {
        let error = try #require(loadError("connection:\n  schedule: { days: [mon, \(raw)] }\n"))
        guard case .invalidChoice(let path, _, let allowed) = error.kind else {
            Issue.record("expected invalidChoice, got \(error.kind)")
            return
        }
        #expect(path == "connection.schedule.days")
        #expect(allowed == ["mon", "tue", "wed", "thu", "fri", "sat", "sun"])
        #expect(error.location == loc(2, 27))
    }

    @Test func daysMustBeAList() throws {
        let error = try #require(loadError("connection:\n  schedule: { days: mon }\n"))
        #expect(error.kind == .typeMismatch(path: "connection.schedule.days", expected: .sequence))
    }

    @Test func unknownScheduleKeysAreWarnings() throws {
        let result = try decodeYAML("connection:\n  schedule: { enabled: true, holidays: [] }\n")
        #expect(result.config.connection.schedule.enabled)
        #expect(result.warnings == [ConfigWarning(.unknownKey(path: "connection.schedule.holidays"), at: loc(2, 30))])
    }

    @Test func validateChecksTheSchedule() {
        var config = AppConfig()
        config.connection.schedule.end = config.connection.schedule.start
        #expect(throws: ConfigLoadError(.scheduleEndNotAfterStart(start: config.connection.schedule.start,
                                                                   end: config.connection.schedule.start))) {
            try config.validate()
        }
        config = AppConfig()
        config.connection.schedule.awayMinutes = 0
        #expect(throws: ConfigLoadError(.outOfRange(path: "connection.schedule.away_minutes", raw: "0",
                                                    range: ConfigLimits.awayMinutes))) {
            try config.validate()
        }
    }

    @Test func templateWritesNormalizedDaysAndQuotedTimes() throws {
        var config = AppConfig()
        config.connection.schedule = .init(
            enabled: true, days: [.sun, .sat], start: TimeOfDay(hour: 7, minute: 5), end: TimeOfDay(hour: 23, minute: 59), awayMinutes: 5)
        let text = ConfigTemplate.render(config)
        #expect(text.contains(
            "  schedule: { enabled: true, days: [sat, sun], start: \"07:05\", end: \"23:59\", away_minutes: 5 }\n"))
        #expect(try ConfigCodec.decode(Data(text.utf8)).config == config)

        config.connection.schedule.days = []
        #expect(ConfigTemplate.render(config).contains("days: [], "))
        #expect(try ConfigCodec.decode(ConfigCodec.encode(config)).config == config)
    }

    @Test("TimeOfDay parsing and formatting", arguments: [
        ("0:00", "00:00"), ("9:30", "09:30"), ("12:00", "12:00"), ("23:59", "23:59"),
    ])
    func timeOfDay(raw: String, formatted: String) throws {
        let time = try #require(TimeOfDay(parsing: raw))
        #expect(time.description == formatted)
        #expect(TimeOfDay(parsing: formatted) == time)
    }
}
