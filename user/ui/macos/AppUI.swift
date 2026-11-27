/// What the main service has for the UI to show.
struct UIUpdate {}

/// What the user did in the UI.
struct UserAction {}

/// The app's UI as a whole: it makes the three UIs and stands between them and the main service.
@MainActor
final class AppUI {
  private lazy var menuBar = MenuBarUI(app: self)
  private lazy var settings = SettingsUI(app: self)
  private let notification = NotificationUI()

  /// Makes what the UI needs before it can show anything.
  func setUp() {
    menuBar.setUp()
    settings.setUp()
    notification.setUp()
  }

  /// Takes what the main service has for the UI. It can be called from any thread.
  nonisolated func push(_ update: UIUpdate) {
    fatalError("hand what the main service has to the UI it concerns")
    // 1. Move over to the UI thread.
    // 2. The connection and updates go to the menu bar UI and the settings UI.
    // 3. The settings, the result of a load, the accounts, the Helper's approval, the exception
    //    routes and the log lines go to the settings UI.
    // 4. A notice goes to the notification UI.
    // 5. Being told that the app can quit ends the UI loop.
  }

  /// Sends what the user did to the main service.
  func send(_ action: UserAction) {
    fatalError("hand what the user did to the main service")
  }

  /// Shows the settings window. The menu bar UI asks for it.
  func openSettings() {
    fatalError("have the settings UI show its window")
  }
}
