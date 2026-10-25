import FortiConfig
import SwiftUI

/// Tab 2, Connection (DESIGN §5.6): status and actions, the schedule and connection options (saved to
/// config.yaml right away), the Helper, and the settings file.
struct ConnectionTab: View {
    let environment: AppEnvironment
    let clock: StatusClock
    let accountNotice: AccountChangeNotice

    @State private var optionsError: String?
    @State private var loginItemError: String?
    @State private var helperError: String?

    private var connection: ConnectionController { environment.connection }
    private var configStore: ConfigStore { environment.configStore }
    private var loginItems: LoginItemService { environment.loginItems }

    var body: some View {
        Form {
            statusSection
            scheduleSection
            optionsSection
            helperSection
            configFileSection
        }
        .formStyle(.grouped)
    }

    // MARK: - Status

    private var statusSection: some View {
        Section("Status") {
            let now = clock.now
            let otpMethod = configStore.otpMethod
            LabeledContent("Current status", value: StatusText.line(connection.statusInputs, now: now, otpMethod: otpMethod))
            if let reason = StatusText.waitReason(connection.model.state, otpMethod: otpMethod) {
                LabeledContent("Reason", value: reason)
            }
            if let attention = connection.attention {
                NoticeRow(text: attention.message(otpMethod: otpMethod), style: .error)
            }
            if let since = connection.connectedSince {
                LabeledContent(
                    "Connected at",
                    value: String(localized: "\(StatusText.clockTime(since, now: now)) (\(StatusText.duration(now.timeIntervalSince(since))) ago)")
                )
                LabeledContent("Tunnel IP", value: connection.tunnelIP ?? String(localized: "Unknown"))
                LabeledContent("Next proactive reconnect", value: connection.plannedReconnectAt.map {
                    "\(StatusText.clockTime($0, now: now)) (\(StatusText.plannedReconnect($0, now: now)))"
                } ?? (connection.model.nextPlannedReconnect != nil
                    ? String(localized: "Paused by the schedule")
                    : String(localized: "Off")))
            }
            if let retryAt = connection.nextRetryAt {
                LabeledContent("Next attempt", value: StatusText.clockTime(retryAt, now: now))
            }
            HStack {
                let actions = connection.menuActions
                if actions.showsConnect {
                    Button("Connect") { connection.connect() }
                }
                if actions.showsDisconnect {
                    Button("Disconnect") { connection.disconnect() }
                }
                Button("Reconnect Now") { connection.reconnectNow() }
                    .disabled(!actions.reconnectNowEnabled)
            }
        }
    }

    // MARK: - Schedule

    private var scheduleSection: some View {
        let schedule = configStore.config.connection.schedule
        return Section {
            Toggle("Connect on a schedule", isOn: option(\.schedule.enabled))
            LabeledContent("Days") {
                HStack(spacing: 4) {
                    ForEach(AppConfig.Connection.Schedule.Day.allCases, id: \.self) { day in
                        Toggle(day.shortName, isOn: scheduleDay(day))
                            .toggleStyle(.button)
                    }
                }
            }
            .disabled(!schedule.enabled)
            ScheduleHoursFields(start: schedule.start, end: schedule.end) { start, end in
                do throws(ConfigStoreError) {
                    try configStore.update {
                        $0.connection.schedule.start = start
                        $0.connection.schedule.end = end
                    }
                    return nil
                } catch {
                    return error.message
                }
            }
            .disabled(!schedule.enabled)
            MinutesField(
                title: "Stop reconnecting after being away",
                value: schedule.awayMinutes,
                range: ConfigLimits.awayMinutes
            ) { minutes in
                saveOption { $0.schedule.awayMinutes = minutes }
            }
            .disabled(!schedule.enabled)
            if schedule.enabled, let outlook = connection.scheduleOutlook {
                LabeledContent("Next", value: outlook.summary(now: clock.now))
            }
        } header: {
            Text("Schedule")
        } footer: {
            Text("Connects at the start time on the selected days. The end time never disconnects: after it, reconnects wait while you're away (screen locked or off, or the Mac asleep) and stop once you've been away for the time above. The same applies when you connect yourself.")
                .footnoteStyle()
        }
    }

    /// One weekday button; the list is saved in Monday-first order without repeats.
    private func scheduleDay(_ day: AppConfig.Connection.Schedule.Day) -> Binding<Bool> {
        Binding(
            get: { configStore.config.connection.schedule.days.contains(day) },
            set: { selected in
                saveOption { connection in
                    var days = Set(connection.schedule.days)
                    if selected { days.insert(day) } else { days.remove(day) }
                    connection.schedule.days = AppConfig.Connection.Schedule.normalizedDays(days)
                }
            }
        )
    }

    // MARK: - Options

    private var optionsSection: some View {
        Section {
            Toggle("Connect automatically at launch", isOn: option(\.autoConnectOnLaunch))
            if configStore.config.connection.schedule.enabled {
                Text("With the schedule on, this connects only during the scheduled hours.")
                    .footnoteStyle()
            }
            Toggle("Open at login", isOn: loginItemEnabled)
                .disabled(!loginItems.isInstalledCopy)
            if !loginItems.isInstalledCopy {
                Text("Open at login can only be set for the app installed in /Applications.")
                    .footnoteStyle()
            } else if loginItems.status == .requiresApproval {
                Text("Needs approval in System Settings > General > Login Items & Extensions.")
                    .footnoteStyle()
            }
            if let loginItemError {
                NoticeRow(text: loginItemError, style: .error)
            }

            let options = configStore.config.connection
            MinutesField(title: "Maximum session length", value: options.sessionMaxMinutes, range: ConfigLimits.sessionMaxMinutes) { minutes in
                saveOption { $0.sessionMaxMinutes = minutes }
            }
            Toggle("Proactive reconnect", isOn: option(\.proactiveReconnect.enabled))
            MinutesField(title: "Margin", value: options.proactiveReconnect.marginMinutes, range: ConfigLimits.marginMinutes) { minutes in
                saveOption { $0.proactiveReconnect.marginMinutes = minutes }
            }
            .disabled(!options.proactiveReconnect.enabled)
            Toggle("Reconnect after sleep", isOn: option(\.reconnectAfterSleep.enabled))
            MinutesField(
                title: "Minimum sleep time",
                value: options.reconnectAfterSleep.minSleepMinutes,
                range: ConfigLimits.minSleepMinutes
            ) { minutes in
                saveOption { $0.reconnectAfterSleep.minSleepMinutes = minutes }
            }
            .disabled(!options.reconnectAfterSleep.enabled)
            if let optionsError {
                NoticeRow(text: optionsError, style: .error)
            }
        } header: {
            Text("Options")
        } footer: {
            Text("Proactive reconnect connects again before the server ends the session (maximum session length − margin). Reconnect after sleep starts a new connection when the Mac wakes from sleeping at least the minimum sleep time.")
                .footnoteStyle()
        }
    }

    /// A `connection.*` switch, written to config.yaml when flipped.
    private func option(_ keyPath: WritableKeyPath<AppConfig.Connection, Bool>) -> Binding<Bool> {
        Binding(
            get: { configStore.config.connection[keyPath: keyPath] },
            set: { enabled in saveOption { $0[keyPath: keyPath] = enabled } }
        )
    }

    /// Writes `connection.*` to config.yaml; a rejected value (range, margin ≥ session) is explained below.
    @discardableResult
    private func saveOption(_ mutate: (inout AppConfig.Connection) -> Void) -> Bool {
        do throws(ConfigStoreError) {
            try configStore.update { mutate(&$0.connection) }
            optionsError = nil
            return true
        } catch {
            optionsError = error.message
            return false
        }
    }

    private var loginItemEnabled: Binding<Bool> {
        Binding(
            get: { loginItems.isEnabled },
            set: { enabled in
                do {
                    try loginItems.setEnabled(enabled)
                    loginItemError = nil
                } catch {
                    loginItemError = error.localizedDescription
                }
            }
        )
    }

    // MARK: - Helper

    private var helperSection: some View {
        Section("Helper") {
            let health = environment.readinessChecker.helperHealth
            let state = HelperDisplayState(health: health, appBuild: environment.helperClient.appBuild)
            LabeledContent("Status") {
                NoticeRow(text: state.label, style: Self.style(for: state))
            }
            if state == .notRegistered {
                Button("Register Helper") {
                    do {
                        try environment.registerHelper()
                        helperError = nil
                    } catch {
                        helperError = error.localizedDescription
                    }
                }
            }
            if state == .requiresApproval {
                Text("Allow background activity for FortiVPNManager in System Settings.")
                    .footnoteStyle()
            }
            LabeledContent("General > Login Items & Extensions > App Background Activity") {
                Button("Open System Settings") { environment.openHelperSystemSettings() }
            }
            if let helperError {
                NoticeRow(text: helperError, style: .error)
            }
            if let status = health?.status {
                LabeledContent("openfortivpn", value: status.openfortivpnPath ?? String(localized: "Not found"))
                LabeledContent("Version", value: status.openfortivpnVersion ?? String(localized: "Unknown"))
                if let version = status.openfortivpnVersion, !Self.isTestedVersion(version) {
                    NoticeRow(
                        text: String(localized: "This isn't a tested version (1.24.x). Its output may differ and connections may fail."),
                        style: .warning
                    )
                }
                if let problem = status.binaryProblem {
                    NoticeRow(text: problem, style: .error)
                }
            }
        }
    }

    private static func style(for state: HelperDisplayState) -> NoticeRow.Style {
        switch state {
        case .running: .success
        case .checking: .info
        case .notRegistered, .requiresApproval, .versionMismatch, .unreachable: .warning
        }
    }

    private static func isTestedVersion(_ version: String) -> Bool {
        version == "1.24" || version.hasPrefix("1.24.")
    }

    // MARK: - Settings file

    private var configFileSection: some View {
        Section("Settings File") {
            LabeledContent(displayPath(configStore.fileURL)) {
                Button("Show in Finder") { revealInFinder(configStore.fileURL) }
                    .disabled(!configStore.fileExists)
            }
            if !configStore.fileExists {
                Text("Not created yet. Saving in the Account tab creates it.")
                    .footnoteStyle()
            }
            if let message = configStore.loadErrorMessage {
                NoticeRow(text: String(localized: "\(message) Using the last valid settings."), style: .error)
                    .textSelection(.enabled)
            }
            ForEach(Array(configStore.warnings.enumerated()), id: \.offset) { _, warning in
                NoticeRow(text: warning.displayMessage, style: .warning)
            }
            if accountNotice.isPending {
                NoticeRow(text: String(localized: "Account details changed. They apply from the next connection."), style: .info)
            }
        }
    }
}

/// Start and end of the scheduled hours. Edits are kept as drafts and saved together only while
/// the end is later than the start, so typing a time digit by digit never trips the check.
private struct ScheduleHoursFields: View {
    let start: TimeOfDay
    let end: TimeOfDay
    /// Saves both times; returns an error message when the save was refused.
    let commit: (TimeOfDay, TimeOfDay) -> String?

    @State private var draftStart = Date()
    @State private var draftEnd = Date()
    @State private var error: String?

    var body: some View {
        Group {
            DatePicker("Start time", selection: $draftStart, displayedComponents: .hourAndMinute)
            DatePicker("End time", selection: $draftEnd, displayedComponents: .hourAndMinute)
            if let error {
                NoticeRow(text: error, style: .error)
            }
        }
        .onAppear(perform: load)
        .onChange(of: start, load)
        .onChange(of: end, load)
        .onChange(of: draftStart, save)
        .onChange(of: draftEnd, save)
    }

    private func load() {
        draftStart = Self.date(start)
        draftEnd = Self.date(end)
    }

    private func save() {
        let newStart = Self.time(draftStart)
        let newEnd = Self.time(draftEnd)
        guard newStart < newEnd else {
            error = String(localized: "The end time must be later than the start time.")
            return
        }
        error = nil
        if newStart != start || newEnd != end {
            error = commit(newStart, newEnd)
        }
    }

    private static func date(_ time: TimeOfDay) -> Date {
        Calendar.current.date(bySettingHour: time.hour, minute: time.minute, second: 0, of: Date()) ?? Date()
    }

    private static func time(_ date: Date) -> TimeOfDay {
        let parts = Calendar.current.dateComponents([.hour, .minute], from: date)
        return TimeOfDay(hour: parts.hour ?? 0, minute: parts.minute ?? 0)
    }
}

/// A minutes value: a text field that commits on Return or when it loses focus (never per keystroke,
/// which would write intermediate values such as `18` on the way to `180`), plus a stepper.
private struct MinutesField: View {
    let title: LocalizedStringKey
    let value: Int
    let range: ClosedRange<Int>
    /// Saves a new value; `false` when it was rejected.
    let commit: (Int) -> Bool

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
                Stepper(title, value: Binding(get: { value }, set: { _ = commit($0) }), in: range)
                    .labelsHidden()
            }
        }
        .onAppear { draft = String(value) }
        .onChange(of: value) { draft = String(value) }
        .onChange(of: isFocused) {
            if !isFocused { commitDraft() }
        }
    }

    private func commitDraft() {
        guard let minutes = Int(draft.trimmingCharacters(in: .whitespaces)), minutes != value else {
            draft = String(value)
            return
        }
        if !commit(minutes) {
            draft = String(value)
        }
    }
}
