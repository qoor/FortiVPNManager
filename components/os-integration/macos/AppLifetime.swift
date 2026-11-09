import AppKit
import os

/// The app's lifetime as the runtime sees it: a quit waits for the runtime's approval, and activity
/// is held while a connection is wanted.
@MainActor
final class AppLifetime {
    private let post: @Sendable (AppInternal.SystemEvent) -> Void
    private var quitRequested = false
    private var quitApproved = false
    private var activity: NSObjectProtocol?

    init(post: @escaping @Sendable (AppInternal.SystemEvent) -> Void) {
        self.post = post
    }

    /// For `NSApplicationDelegate.applicationShouldTerminate`: the user's or the system's quit is
    /// reported once and waits for `approveQuit()`.
    func applicationShouldTerminate() -> NSApplication.TerminateReply {
        if quitApproved { return .terminateNow }
        if !quitRequested {
            quitRequested = true
            post(.quitRequested)
        }
        return .terminateLater
    }

    func approveQuit() {
        quitApproved = true
        NSApp.reply(toApplicationShouldTerminate: true)
    }

    /// Keeps the app from being suspended or throttled; the system may still sleep.
    func holdActivity(_ hold: Bool) {
        if hold, activity == nil {
            activity = ProcessInfo.processInfo.beginActivity(
                options: .userInitiatedAllowingIdleSystemSleep,
                reason: "Keeping the VPN connection alive"
            )
        } else if !hold, let activity {
            ProcessInfo.processInfo.endActivity(activity)
            self.activity = nil
        }
    }
}

/// The unified log, under the categories the app used before the runtime so existing log filters
/// still match.
nonisolated enum SystemLog {
    static func write(_ level: AppInternal.LogLevel, _ category: AppInternal.LogCategory, _ message: String) {
        let logger = Logger(subsystem: FortiIdentifiers.logSubsystem, category: category.rawValue)
        switch level {
        case .info: logger.info("\(message, privacy: .public)")
        case .notice: logger.notice("\(message, privacy: .public)")
        case .error: logger.error("\(message, privacy: .public)")
        }
    }
}
