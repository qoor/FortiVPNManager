import AppKit
import SwiftUI

/// The Connection tab. The schedule and connection options are saved as they change; the exception
/// rules only by Apply.
struct ConnectionTab: View {
    let ui: AppUI
    @Environment(\.self) private var environment

    var body: some View {
        if let connection = ui.connection, let schedule = ui.schedule, let readiness = ui.readiness,
           let config = ui.config, let bypassRules = ui.bypassRules {
            let time = TimeStyle(now: ui.clock.now, environment: environment)
            Form {
                statusSection(connection, outlook: schedule.outlook, time: time)
                scheduleSection(schedule, time: time)
                optionsSection(config, scheduleEnabled: schedule.enabled)
                helperSection(readiness)
                settingsFileSection(config, accountChanged: connection.accountAppliesNextConnection)
                ExceptionRulesSection(ui: ui, rules: bypassRules, time: time)
            }
            .formStyle(.grouped)
        }
    }

    // MARK: - Status

    private func statusSection(
        _ connection: AppInternal.ConnectionSection, outlook: AppInternal.ScheduleOutlook?, time: TimeStyle
    ) -> some View {
        Section("Status") {
            LabeledContent("Current status", value: connection.statusLine(outlook: outlook, time: time))
            if let reason = connection.waitReason {
                LabeledContent("Reason", value: reason.summary(time: time))
            }
            if let attention = connection.attention(time: time) {
                NoticeRow(text: attention.message, style: .error)
            }
            if case let .connected(since, tunnelIp, proactiveReconnect) = connection.status {
                LabeledContent(
                    "Connected at",
                    value: String(localized: "\(time.clockTime(since)) (\(time.duration(time.now.timeIntervalSince(since))) ago)")
                )
                LabeledContent("Tunnel IP", value: tunnelIp ?? String(localized: "Unknown"))
                LabeledContent("Next proactive reconnect", value: Self.text(of: proactiveReconnect, time: time))
            }
            if let nextAttempt = connection.nextAttempt {
                LabeledContent("Next attempt", value: time.clockTime(nextAttempt))
            }
            HStack {
                StateButton("Connect", state: connection.connect) { ui.send(.connect) }
                StateButton("Disconnect", state: connection.disconnect) { ui.send(.disconnect) }
                StateButton("Reconnect Now", state: connection.reconnectNow) { ui.send(.reconnectNow) }
            }
        }
    }

    private static func text(of proactiveReconnect: AppInternal.ProactiveReconnect, time: TimeStyle) -> String {
        switch proactiveReconnect {
        case let .at(at): "\(time.clockTime(at)) (\(time.plannedReconnect(at)))"
        case .pausedBySchedule: String(localized: "Paused by the schedule")
        case .off: String(localized: "Off")
        }
    }

    // MARK: - Schedule

    private func scheduleSection(_ schedule: AppInternal.ScheduleSection, time: TimeStyle) -> some View {
        Section {
            Toggle("Connect on a schedule", isOn: option(schedule.enabled) { .scheduleEnabled(enabled: $0) })
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
            .disabled(!schedule.enabled)
            ScheduleHoursFields(start: schedule.start, end: schedule.end, error: schedule.hoursError) { start, end in
                ui.send(.setScheduleHours(start: start, end: end))
            }
            .disabled(!schedule.enabled)
            MinutesField(title: "Stop reconnecting after being away", setting: schedule.awayMinutes) {
                ui.send(.setConnectionOption(option: .awayMinutes(minutes: $0)))
            }
            .disabled(!schedule.enabled)
            if schedule.enabled, let outlook = schedule.outlook {
                LabeledContent("Next", value: outlook.summary(time: time))
            }
        } header: {
            Text("Schedule")
        } footer: {
            Text("Connects at the start time on the selected days. The end time never disconnects: after it, reconnects wait while you're away (screen locked or off, or the Mac asleep) and stop once you've been away for the time above. The same applies when you connect yourself.")
                .footnoteStyle()
        }
    }

    // MARK: - Options

    private func optionsSection(_ config: AppInternal.ConfigSection, scheduleEnabled: Bool) -> some View {
        let options = config.options
        let openAtLogin = config.openAtLogin
        return Section {
            Toggle("Connect automatically at launch", isOn: option(options.autoConnectOnLaunch) {
                .autoConnectOnLaunch(enabled: $0)
            })
            if scheduleEnabled {
                Text("With the schedule on, this connects only during the scheduled hours.")
                    .footnoteStyle()
            }
            Toggle("Open at login", isOn: Binding(
                get: { openAtLogin.enabled },
                set: { ui.send(.setOpenAtLogin(enabled: $0)) }
            ))
            .disabled(!openAtLogin.available)
            if !openAtLogin.available {
                Text("Open at login can only be set for the app installed in /Applications.")
                    .footnoteStyle()
            } else if openAtLogin.requiresApproval {
                Text("Needs approval in System Settings > General > Login Items & Extensions.")
                    .footnoteStyle()
            }
            if let error = openAtLogin.error {
                NoticeRow(text: error.loginItemMessage, style: .error)
            }

            MinutesField(title: "Maximum session length", setting: options.sessionMaxMinutes) {
                ui.send(.setConnectionOption(option: .sessionMaxMinutes(minutes: $0)))
            }
            Toggle("Proactive reconnect", isOn: option(options.proactiveReconnect) { .proactiveReconnect(enabled: $0) })
            MinutesField(title: "Margin", setting: options.marginMinutes) {
                ui.send(.setConnectionOption(option: .proactiveMarginMinutes(minutes: $0)))
            }
            .disabled(!options.proactiveReconnect)
            Toggle("Reconnect after sleep", isOn: option(options.reconnectAfterSleep) { .reconnectAfterSleep(enabled: $0) })
            MinutesField(title: "Minimum sleep time", setting: options.minSleepMinutes) {
                ui.send(.setConnectionOption(option: .minSleepMinutes(minutes: $0)))
            }
            .disabled(!options.reconnectAfterSleep)
            if let error = config.optionsError {
                NoticeRow(text: error.message, style: .error)
            }
        } header: {
            Text("Options")
        } footer: {
            Text("Proactive reconnect connects again before the server ends the session (maximum session length − margin). Reconnect after sleep starts a new connection when the Mac wakes from sleeping at least the minimum sleep time.")
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

    // MARK: - Helper

    private func helperSection(_ readiness: AppInternal.ReadinessSection) -> some View {
        Section("Helper") {
            LabeledContent("Status") {
                NoticeRow(text: readiness.helper.label, style: readiness.helper.style)
            }
            StateButton("Register Helper", state: readiness.registerHelper) { ui.send(.registerHelper) }
            if readiness.helper == .requiresApproval {
                Text("Allow background activity for FortiVPNManager in System Settings.")
                    .footnoteStyle()
            }
            LabeledContent("General > Login Items & Extensions > App Background Activity") {
                Button("Open System Settings") { ui.send(.openHelperApprovalSettings) }
            }
            if let error = readiness.registerError {
                NoticeRow(text: error.helperMessage, style: .error)
            }
            if let client = readiness.client {
                LabeledContent("openfortivpn", value: client.path ?? String(localized: "Not found"))
                LabeledContent("Version", value: client.version ?? String(localized: "Unknown"))
                if let untested = client.untestedVersion {
                    NoticeRow(text: untested.message, style: .warning)
                }
                if let problem = client.problem {
                    NoticeRow(text: problem.message, style: .error)
                }
            }
        }
    }

    // MARK: - Settings file

    private func settingsFileSection(_ config: AppInternal.ConfigSection, accountChanged: Bool) -> some View {
        let file = config.file
        return Section("Settings File") {
            LabeledContent(displayPath(file.path)) {
                Button("Show in Finder") { revealInFinder(file.path) }
                    .disabled(!file.exists)
            }
            if !file.exists {
                Text("Not created yet. Saving in the Account tab creates it.")
                    .footnoteStyle()
            }
            if let error = file.loadError {
                NoticeRow(text: String(localized: "\(error.message) Using the last valid settings."), style: .error)
                    .textSelection(.enabled)
            }
            ForEach(Array(file.warnings.enumerated()), id: \.offset) { _, warning in
                NoticeRow(text: warning.displayMessage, style: .warning)
            }
            if accountChanged {
                NoticeRow(text: String(localized: "Account details changed. They apply from the next connection."), style: .info)
            }
        }
    }
}

// MARK: - Exception rules

private struct ExceptionRulesSection: View {
    let ui: AppUI
    let rules: AppInternal.BypassRulesSection
    let time: TimeStyle

    /// A row of a list is being typed: its text reaches the runtime when the edit ends, which Apply
    /// and Discard Changes bring about first, so they take clicks meanwhile.
    @State private var isEditingNetworkRow = false
    @State private var isEditingTargetRow = false

    var body: some View {
        let isEditing = isEditingNetworkRow || isEditingTargetRow
        Section {
            LabeledContent("Current network", value: rules.currentNetworkText)
            LabeledContent("Networks where the rules apply") {
                StateButton("Add Current Network", state: rules.addCurrentNetwork) { ui.send(.addCurrentNetwork) }
            }
            AddressListEditor(
                rows: rules.networks.rows,
                problemRows: Set(rules.networks.problemRows),
                canAdd: rules.networks.addRow == .enabled,
                emptyText: String(localized: "Double-click here or click + to add a network such as 10.20.161.0/24."),
                placeholder: String(localized: "e.g. 10.20.161.0/24"),
                addHelp: String(localized: "Add a network"),
                onChange: { ui.send(.setBypassRows(list: .networks, rows: $0)) },
                onEditingChanged: { isEditingNetworkRow = $0 }
            )
            .frame(height: 96)
            Text("Exception targets")
            AddressListEditor(
                rows: rules.targets.rows,
                problemRows: Set(rules.targets.problemRows),
                canAdd: rules.targets.addRow == .enabled,
                emptyText: String(localized: "Double-click here or click + to add a host or network."),
                placeholder: String(localized: "e.g. 10.30.161.16 or 10.30.170.0/24"),
                addHelp: String(localized: "Add a host or network"),
                onChange: { ui.send(.setBypassRows(list: .targets, rows: $0)) },
                onEditingChanged: { isEditingTargetRow = $0 }
            )
            .frame(height: 160)
            if let notice = rules.notice {
                NoticeRow(text: notice.message, style: .error)
            }
            LabeledContent("Status") {
                let status = rules.status.line(time: time)
                NoticeRow(text: status.text, style: status.style)
            }
            if let message = rules.stoppedRetryingMessage {
                NoticeRow(text: message, style: .error)
            }
            HStack {
                if rules.hasChanges {
                    Text("You have changes that aren't applied yet.")
                        .footnoteStyle()
                }
                Spacer()
                StateButton("Discard Changes", state: whileEditing(rules.discard, isEditing)) {
                    endTextEditing()
                    ui.send(.discardBypassChanges)
                }
                StateButton("Apply", state: whileEditing(rules.apply, isEditing)) {
                    endTextEditing()
                    ui.send(.applyBypassRules)
                }
            }
        } header: {
            Text("Exception Rules")
        } footer: {
            Text("While this Mac's address is in one of the networks above, the exception targets skip the VPN and go out directly through that network's router; on any other network everything goes through the VPN. The routes are saved in the macOS network settings, so they stay after a restart. Edits take effect when you click Apply. On a listed network, routes added elsewhere through the same router appear here; routes through other gateways are left alone.")
                .footnoteStyle()
        }
    }

    private func whileEditing(_ state: AppInternal.ButtonState, _ isEditing: Bool) -> AppInternal.ButtonState {
        isEditing && state == .disabled ? .enabled : state
    }

    /// Finishes a row being typed so its text is sent first (buttons do not take focus).
    private func endTextEditing() {
        NSApp.keyWindow?.makeFirstResponder(nil)
    }
}

/// Start and end of the scheduled hours. Every change of either picker is sent; the runtime saves
/// them only while the end is later than the start, so the pickers keep what was picked meanwhile.
private struct ScheduleHoursFields: View {
    let start: AppInternal.TimeOfDay
    let end: AppInternal.TimeOfDay
    let error: AppInternal.ScheduleHoursError?
    let send: (AppInternal.TimeOfDay, AppInternal.TimeOfDay) -> Void

    @State private var draftStart = Date()
    @State private var draftEnd = Date()

    var body: some View {
        Group {
            DatePicker("Start time", selection: $draftStart, displayedComponents: .hourAndMinute)
            DatePicker("End time", selection: $draftEnd, displayedComponents: .hourAndMinute)
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
/// within the range the settings file accepts.
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
                    .frame(width: 64)
                    .focused($isFocused)
                    .onSubmit(commitDraft)
                Text("min")
                Stepper(
                    title,
                    value: Binding(get: { setting.value }, set: { commit($0) }),
                    in: setting.minimum...setting.maximum
                )
                .labelsHidden()
            }
        }
        .onAppear { draft = String(setting.value) }
        .onChange(of: setting) { draft = String(setting.value) }
        .onChange(of: isFocused) {
            if !isFocused { commitDraft() }
        }
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
