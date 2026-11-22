import Foundation
import UserNotifications
import os

/// macOS notifications, for problems and updates the user should see while Settings is closed. The
/// text comes from the UI's string code. Permission is asked the first time something is posted;
/// clicking a notification calls `onOpen` with its identifier, and its button calls `onAction`.
@MainActor
final class UserNotifier: NSObject, UNUserNotificationCenterDelegate {
  var onOpen: ((String) -> Void)?
  var onAction: ((String) -> Void)?

  private nonisolated static let actionIdentifier = "action"
  private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "notifications")

  func start() {
    UNUserNotificationCenter.current().delegate = self
  }

  /// Replaces an earlier notification with the same `identifier`.
  func post(identifier: String, title: String, body: String, action: String?) {
    let content = UNMutableNotificationContent()
    content.title = title
    content.body = body
    content.sound = .default
    if let action {
      content.categoryIdentifier = identifier
      UNUserNotificationCenter.current().setNotificationCategories([
        UNNotificationCategory(
          identifier: identifier,
          actions: [
            UNNotificationAction(
              identifier: Self.actionIdentifier, title: action, options: .foreground)
          ],
          intentIdentifiers: []
        )
      ])
    }
    let request = UNNotificationRequest(identifier: identifier, content: content, trigger: nil)
    Task {
      let center = UNUserNotificationCenter.current()
      do {
        guard try await center.requestAuthorization(options: [.alert, .sound]) else {
          logger.notice(
            "notifications are not allowed; not showing \(identifier, privacy: .public)")
          return
        }
        try await center.add(request)
      } catch {
        logger.error(
          "posting a notification failed: \(error.localizedDescription, privacy: .public)")
      }
    }
  }

  /// Also show banners while the app is active.
  nonisolated func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification
  ) async -> UNNotificationPresentationOptions {
    [.banner, .sound]
  }

  nonisolated func userNotificationCenter(
    _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse
  ) async {
    let identifier = response.notification.request.identifier
    let action = response.actionIdentifier == Self.actionIdentifier
    await MainActor.run { [weak self] in
      if action {
        self?.onAction?(identifier)
      } else {
        self?.onOpen?(identifier)
      }
    }
  }
}
