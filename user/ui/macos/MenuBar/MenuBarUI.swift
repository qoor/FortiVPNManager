/// The menu bar icon and its menu: it shows the connection and takes the user's commands.
@MainActor
final class MenuBarUI {
  init(app: AppUI) {
    fatalError("create the menu bar UI")
    // The app UI is kept, to send what the user does through it.
  }

  /// Makes what the menu bar UI needs before it can show anything.
  func setUp() {
    fatalError("put the icon into the menu bar and build its menu")
    // The menu's commands are connect, disconnect, reconnect now, settings and quit.
    // Choosing settings asks the app UI to show the settings window.
    // Choosing any other command sends what the user did through the app UI.
  }

  /// Shows what changed.
  func show(_ update: UIUpdate) {
    fatalError("show what changed in the icon and the menu")
    // The connection: the icon for its state, and in the menu when it came up, the tunnel
    // address, the next reconnect time, the reason for waiting and the stage it is at.
    // Updates: the mark that a new version is there.
  }
}
