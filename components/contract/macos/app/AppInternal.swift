import Foundation

/// Messages between the app's Swift parts (the UI and the OS integration) and the runtime (Rust),
/// defined in components/contract/src/app_internal; case and label names are the JSON keys, and
/// `MessageCoding` codes them. Swift encodes what it sends and decodes what it receives.
nonisolated enum AppInternal {
    /// Sent to Rust.
    enum ToRuntime: Encodable, Sendable {
        /// Always the first message.
        case launched(facts: AppFacts)
        case command(Command)
        /// Answers the `FromRuntime.portRequest` with the same `id`.
        case portResponse(id: UInt64, response: PortResponse)
        case event(SystemEvent)
    }

    /// Sent by Rust.
    enum FromRuntime: Decodable, Equatable, Sendable {
        /// One domain section, whole. Every section comes once first, then only changed ones.
        case section(Section)
        /// Log lines not sent before, oldest first.
        case logLines(lines: [LogLine])
        /// Answer with `ToRuntime.portResponse` and the same `id`.
        case portRequest(id: UInt64, request: PortRequest)
        /// Carry out without an answer.
        case portAction(PortAction)
        /// Show the settings window, at most once per launch (the rule is in
        /// components/contract/src/app_internal/mod.rs). Pick the pane only: Account when the config
        /// section has an import report, otherwise the pane of the first setup item in the
        /// connection status.
        case openSettings
    }

    struct AppFacts: Encodable, Sendable {
        /// `CFBundleVersion`, which the Helper must also report.
        var appBuild: String
        /// The app runs from /Applications, the only copy allowed to register the Helper and the
        /// login item.
        var installedCopy: Bool
        /// The OAuth client JSON in the app bundle, if this build has one.
        var builtInOauthClient: String?
        var timeZone: String
        var signInPages: SignInPages
    }

    /// The pages the Google sign-in listener shows in the browser, in the UI's language.
    struct SignInPages: Encodable, Sendable {
        var completed: String
        var notCompleted: String
        var invalid: String
    }

    /// One line for the Log pane.
    struct LogLine: Decodable, Equatable, Sendable {
        var time: Date
        var source: LogSource
        var text: String
    }

    enum LogSource: String, Decodable, Sendable {
        case stdout
        case stderr
        /// An explanation line from the app or the Helper.
        case note
    }

    enum OtpMethod: String, Codable, Sendable {
        case imap
        case gmailApi
    }

    enum Weekday: String, Codable, Sendable, CaseIterable {
        case mon, tue, wed, thu, fri, sat, sun
    }

    /// A local wall-clock time.
    struct TimeOfDay: Codable, Equatable, Sendable {
        var hour: Int
        var minute: Int
    }

    enum ButtonState: String, Decodable, Sendable {
        case hidden
        case enabled
        case disabled
    }

    /// A setting in minutes with the range the settings file accepts.
    struct MinutesSetting: Decodable, Equatable, Sendable {
        var value: Int
        var minimum: Int
        var maximum: Int
    }
}
