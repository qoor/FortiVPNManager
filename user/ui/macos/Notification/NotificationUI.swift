/// The notifications of the OS: it tells the user what the app has to tell.
@MainActor
final class NotificationUI {
  /// Makes what the notification UI needs before it can notify.
  func setUp() {
    fatalError("ask the OS for leave to notify the user")
  }

  /// Shows what changed.
  func show(_ update: UIUpdate) {
    fatalError("post the notice as a notification of the OS")
  }
}
