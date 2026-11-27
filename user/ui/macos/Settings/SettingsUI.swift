/// The settings window with its panes: it shows the settings and takes the user's changes.
@MainActor
final class SettingsUI {
  init(app: AppUI) {
    fatalError("create the settings UI")
    // The app UI is kept, to send what the user does through it.
  }

  /// Makes what the settings UI needs before it can show anything.
  func setUp() {
    fatalError("build the settings window with its panes")
    // The panes are general, account, connection, exception rules, advanced and log.
    // What the user confirms in a pane is sent through the app UI: a changed setting, a
    // password, linking or unlinking the Google account, the Gmail test, loading the settings
    // of an existing VPN client, and checking for an update, starting or cancelling its
    // download and installing it.
    // Copying and clearing the log pane stay in the window.
  }

  /// Shows the settings window.
  func open() {
    fatalError("show the window and bring it to the front")
  }

  /// Shows what changed.
  func show(_ update: UIUpdate) {
    fatalError("show what changed in the pane it belongs to")
    // The connection: which of connect, disconnect and reconnect now can be used.
    // The settings: the values of every pane, a problem in the settings file, and the result
    // of loading the settings of an existing VPN client.
    // The Helper: that it waits for the user's approval, and where to give it.
    // The accounts: whether they are ready and what is missing, and the result of a test.
    // The exception routes: the problem of each row.
    // Updates: the version, the progress and a failure.
    // A log line: one more line in the log pane.
  }
}
