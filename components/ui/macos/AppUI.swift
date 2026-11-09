import Foundation
import Observation
import SwiftUI

/// What the menu and the settings window draw: the latest domain sections and log lines from the
/// runtime, as they came. User actions go back as commands through `send`.
@MainActor
@Observable
final class AppUI {
    private(set) var connection: AppInternal.ConnectionSection?
    private(set) var schedule: AppInternal.ScheduleSection?
    private(set) var readiness: AppInternal.ReadinessSection?
    private(set) var gmail: AppInternal.GmailSection?
    private(set) var bypassRules: AppInternal.BypassRulesSection?
    private(set) var config: AppInternal.ConfigSection?

    let log = LogLines()
    let clock = StatusClock()
    /// The Account tab's fields as typed.
    let accountDraft = AccountDraft()

    @ObservationIgnored let send: (AppInternal.Command) -> Void
    @ObservationIgnored private(set) lazy var settings = SettingsWindowController(ui: self)

    init(send: @escaping (AppInternal.Command) -> Void) {
        self.send = send
    }

    func apply(_ section: AppInternal.Section) {
        switch section {
        case let .connection(section):
            connection = section
            clock.restart(countdownUntil: section.nextAttempt)
        case let .schedule(section): schedule = section
        case let .readiness(section): readiness = section
        case let .gmail(section): gmail = section
        case let .bypassRules(section): bypassRules = section
        case let .config(section):
            config = section
            accountDraft.replace(with: section.account)
        }
    }

    /// The runtime asked to show Settings.
    func openSettings() {
        settings.show(tabToOpen)
    }

    /// Account when the config section has an import report (the automatic first-launch import),
    /// otherwise the tab of the first setup item.
    var tabToOpen: SettingsTab {
        if config?.account.importReport != nil { return .account }
        if case let .setupRequired(items) = connection?.status, let first = items.first { return first.settingsTab }
        return .account
    }

    /// Sends the Account tab's fields after an edit, against the revision they were typed on.
    func accountEdited() {
        send(.editAccount(revision: accountDraft.revision, form: accountDraft.form))
    }
}

/// The Account tab's fields as typed. The runtime keeps the draft that counts; these fields are
/// replaced only when it replaces what was typed, which it marks with a new revision.
@MainActor
@Observable
final class AccountDraft {
    var host = ""
    var port = ""
    var username = ""
    var realm = ""
    var gmailAddress = ""
    var vpnPassword = ""
    var appPassword = ""
    private(set) var revision: UInt64 = 0
    @ObservationIgnored private var loaded = false

    func replace(with state: AppInternal.AccountFormState) {
        guard !loaded || state.revision != revision else { return }
        loaded = true
        revision = state.revision
        host = state.host
        port = state.port
        username = state.username
        realm = state.realm
        gmailAddress = state.gmailAddress
        vpnPassword = ""
        appPassword = ""
    }

    var form: AppInternal.AccountForm {
        AppInternal.AccountForm(
            host: host, port: port, username: username, realm: realm, gmailAddress: gmailAddress,
            vpnPassword: vpnPassword, appPassword: appPassword
        )
    }
}

/// "Now" for the relative times in the menu and the Connection tab: set by every connection
/// section, every 30 seconds, and every second while a reconnect countdown runs.
@MainActor
@Observable
final class StatusClock {
    private(set) var now = Date()
    @ObservationIgnored private var ticker: Task<Void, Never>?

    func restart(countdownUntil: Date?) {
        ticker?.cancel()
        now = Date()
        ticker = Task { [weak self] in
            while true {
                let counting = countdownUntil.map { $0 > Date() } ?? false
                do { try await Task.sleep(for: .seconds(counting ? 1 : 30)) } catch { return }
                guard let self else { return }
                now = Date()
            }
        }
    }
}

extension TimeStyle {
    /// The locale, calendar and time zone of a view's environment.
    init(now: Date, environment: EnvironmentValues) {
        var calendar = environment.calendar
        calendar.timeZone = environment.timeZone
        self.init(now: now, locale: environment.locale, calendar: calendar)
    }
}
