import AppKit
import SwiftUI

/// The Connection pane: the schedule and the reconnect options, saved as they change.
struct ConnectionPane: View {
    let ui: AppUI
    let navigation: SettingsNavigation
    @Environment(\.self) private var environment

    var body: some View {
        SettingsForm(navigation: navigation) {
            if let schedule = ui.schedule, let config = ui.config {
                scheduleSection(schedule, time: TimeStyle(now: ui.clock.now, environment: environment))
                reconnectSection(config)
            }
        }
        .toggleStyle(.switch)
    }

    // MARK: - Schedule

    /// The schedule's switch leads; the rows under it belong to it and are indented.
    private func scheduleSection(_ schedule: AppInternal.ScheduleSection, time: TimeStyle) -> some View {
        Section {
            Toggle("Connect on a schedule", isOn: option(schedule.enabled) { .scheduleEnabled(enabled: $0) })
                .searchAnchor("Connect on a schedule", navigation)
            Group {
                LabeledContent("Days") {
                    HStack(spacing: 4) {
                        ForEach(AppInternal.Weekday.allCases, id: \.self) { day in
                            Toggle(day.shortName, isOn: option(schedule.days.contains(day)) {
                                .scheduleDay(day: day, selected: $0)
                            })
                            .toggleStyle(.button)
                        }
                    }
                }
                .searchAnchor("Days", navigation)
                ScheduleHoursFields(start: schedule.start, end: schedule.end, error: schedule.hoursError, navigation: navigation) { start, end in
                    ui.send(.setScheduleHours(start: start, end: end))
                }
                MinutesField(title: "Stop reconnecting after being away", setting: schedule.awayMinutes) {
                    ui.send(.setConnectionOption(option: .awayMinutes(minutes: $0)))
                }
                .help("After the end time, reconnects wait while you're away (screen locked or off, Mac asleep) and stop after this long.")
                .searchAnchor("Stop reconnecting after being away", navigation)
                if schedule.enabled, let outlook = schedule.outlook {
                    LabeledContent("Next", value: outlook.summary(time: time))
                }
            }
            .subordinate()
            .disabled(!schedule.enabled)
        } header: {
            Text("Schedule")
        } footer: {
            Text("Connects at the start time on the selected days. The end time never disconnects.")
                .footnoteStyle()
        }
    }

    // MARK: - Reconnect

    private func reconnectSection(_ config: AppInternal.ConfigSection) -> some View {
        let options = config.options
        return Section {
            MinutesField(title: "Maximum session length", setting: options.sessionMaxMinutes) {
                ui.send(.setConnectionOption(option: .sessionMaxMinutes(minutes: $0)))
            }
            .searchAnchor("Maximum session length", navigation)
            Toggle("Proactive reconnect", isOn: option(options.proactiveReconnect) { .proactiveReconnect(enabled: $0) })
                .searchAnchor("Proactive reconnect", navigation)
            MinutesField(title: "Margin", setting: options.marginMinutes) {
                ui.send(.setConnectionOption(option: .proactiveMarginMinutes(minutes: $0)))
            }
            .searchAnchor("Margin", navigation)
            .subordinate()
            .disabled(!options.proactiveReconnect)
            Toggle("Reconnect after sleep", isOn: option(options.reconnectAfterSleep) { .reconnectAfterSleep(enabled: $0) })
                .help("Starts a new connection when the Mac wakes from sleeping at least the minimum sleep time.")
                .searchAnchor("Reconnect after sleep", navigation)
            MinutesField(title: "Minimum sleep time", setting: options.minSleepMinutes) {
                ui.send(.setConnectionOption(option: .minSleepMinutes(minutes: $0)))
            }
            .searchAnchor("Minimum sleep time", navigation)
            .subordinate()
            .disabled(!options.reconnectAfterSleep)
            if let error = config.optionsError {
                NoticeRow(text: error.message, style: .error)
            }
        } header: {
            Text("Reconnect")
        } footer: {
            Text("Connects again before the server ends the session (maximum session length − margin).")
                .footnoteStyle()
        }
    }

    /// A switch of the connection or the schedule, saved when flipped.
    private func option(_ value: Bool, _ change: @escaping (Bool) -> AppInternal.ConnectionOption) -> Binding<Bool> {
        Binding(
            get: { value },
            set: { ui.send(.setConnectionOption(option: change($0))) }
        )
    }
}

/// Start and end of the scheduled hours. Every change of either picker is sent; the runtime saves
/// them only while the end is later than the start, so the pickers keep what was picked meanwhile.
private struct ScheduleHoursFields: View {
    let start: AppInternal.TimeOfDay
    let end: AppInternal.TimeOfDay
    let error: AppInternal.ScheduleHoursError?
    let navigation: SettingsNavigation
    let send: (AppInternal.TimeOfDay, AppInternal.TimeOfDay) -> Void

    @State private var draftStart = Date()
    @State private var draftEnd = Date()

    var body: some View {
        Group {
            DatePicker("Start time", selection: $draftStart, displayedComponents: .hourAndMinute)
                .searchAnchor("Start time", navigation)
            DatePicker("End time", selection: $draftEnd, displayedComponents: .hourAndMinute)
                .searchAnchor("End time", navigation)
            if let error {
                NoticeRow(text: error.message, style: .error)
            }
        }
        .onAppear(perform: load)
        .onChange(of: start, load)
        .onChange(of: end, load)
        .onChange(of: draftStart, picked)
        .onChange(of: draftEnd, picked)
    }

    private func load() {
        draftStart = Self.date(start)
        draftEnd = Self.date(end)
    }

    /// Loading the saved times changes the pickers too; that is not sent again unless an earlier
    /// choice is still unsaved.
    private func picked() {
        let newStart = Self.time(draftStart)
        let newEnd = Self.time(draftEnd)
        if newStart != start || newEnd != end || error != nil {
            send(newStart, newEnd)
        }
    }

    private static func date(_ time: AppInternal.TimeOfDay) -> Date {
        Calendar.current.date(bySettingHour: time.hour, minute: time.minute, second: 0, of: Date()) ?? Date()
    }

    private static func time(_ date: Date) -> AppInternal.TimeOfDay {
        let parts = Calendar.current.dateComponents([.hour, .minute], from: date)
        return AppInternal.TimeOfDay(hour: parts.hour ?? 0, minute: parts.minute ?? 0)
    }
}

/// A minutes setting: a text field that commits on Return or when it loses focus (never per
/// keystroke, which would save intermediate values such as `18` on the way to `180`), plus a stepper
/// within the range the settings file accepts. Shift-click steps by 10 across ranges that wide
/// (HIG Steppers, macOS).
private struct MinutesField: View {
    let title: LocalizedStringKey
    let setting: AppInternal.MinutesSetting
    let commit: (Int) -> Void

    @State private var draft = ""
    @FocusState private var isFocused: Bool

    var body: some View {
        LabeledContent(title) {
            HStack(spacing: 6) {
                TextField(title, text: $draft)
                    .labelsHidden()
                    .multilineTextAlignment(.trailing)
                    .frame(width: 60)
                    .focused($isFocused)
                    .onSubmit(commitDraft)
                Text("min")
                    .foregroundStyle(.secondary)
                // Bound to the value, so the arrows stop at the range's ends and VoiceOver reads it.
                Stepper(title, value: stepped, in: setting.minimum...setting.maximum)
                    .labelsHidden()
                    .help("Shift-click to change by 10 minutes")
            }
        }
        .onAppear { draft = String(setting.value) }
        .onChange(of: setting) { draft = String(setting.value) }
        .onChange(of: isFocused) {
            if !isFocused { commitDraft() }
        }
        // Going to another pane takes the field away before its focus change is seen.
        .onDisappear(perform: commitDraft)
    }

    /// The stepper's value: a step it makes is 10 minutes while Shift is held.
    private var stepped: Binding<Int> {
        Binding(
            get: { setting.value },
            set: { value in
                let size = NSEvent.modifierFlags.contains(.shift) ? 10 : 1
                let next = min(max(setting.value + (value - setting.value).signum() * size, setting.minimum), setting.maximum)
                if next != setting.value { commit(next) }
            }
        )
    }

    /// A rejected number stays in the field with the reason below it until the setting changes.
    private func commitDraft() {
        guard let minutes = Int(draft.trimmingCharacters(in: .whitespaces)), minutes != setting.value else {
            draft = String(setting.value)
            return
        }
        commit(minutes)
    }
}
