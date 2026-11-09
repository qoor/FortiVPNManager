import Foundation
import FortiShared
import os
import UserNotifications

/// macOS notifications, for problems the user should see while Settings is closed. The text comes
/// from the UI's string code. Permission is asked the first time something is posted; clicking a
/// notification calls `onOpen`.
@MainActor
final class UserNotifier: NSObject, UNUserNotificationCenterDelegate {
    var onOpen: (() -> Void)?

    private let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "notifications")

    func start() {
        UNUserNotificationCenter.current().delegate = self
    }

    /// Replaces an earlier notification with the same `identifier`.
    func post(identifier: String, title: String, body: String) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        let request = UNNotificationRequest(identifier: identifier, content: content, trigger: nil)
        Task {
            let center = UNUserNotificationCenter.current()
            do {
                guard try await center.requestAuthorization(options: [.alert, .sound]) else {
                    logger.notice("notifications are not allowed; not showing \(identifier, privacy: .public)")
                    return
                }
                try await center.add(request)
            } catch {
                logger.error("posting a notification failed: \(error.localizedDescription, privacy: .public)")
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
        await MainActor.run { [weak self] in self?.onOpen?() }
    }
}
