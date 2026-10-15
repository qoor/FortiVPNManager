import AppKit
import SwiftUI

/// Root of the settings window: Account / Connection / Log (DESIGN §5.6).
struct SettingsView: View {
    let environment: AppEnvironment
    @Bindable var navigation: SettingsNavigation
    let accountEditor: AccountEditor
    let clock: StatusClock
    let accountNotice: AccountChangeNotice

    var body: some View {
        TabView(selection: $navigation.selection) {
            Tab("Account", systemImage: "person.crop.circle", value: SettingsTab.account) {
                AccountTab(environment: environment, editor: accountEditor, accountNotice: accountNotice)
            }
            Tab("Connection", systemImage: "network", value: SettingsTab.connection) {
                ConnectionTab(environment: environment, clock: clock, accountNotice: accountNotice)
            }
            Tab("Log", systemImage: "doc.plaintext", value: SettingsTab.log) {
                LogTab(logStore: environment.logStore)
            }
        }
        .padding(.top, 8)
        .frame(minWidth: 600, minHeight: 520)
    }
}

/// One line of status with a coloured symbol.
struct NoticeRow: View {
    enum Style {
        case info
        case success
        case warning
        case error

        var symbol: String {
            switch self {
            case .info: "info.circle"
            case .success: "checkmark.circle.fill"
            case .warning: "exclamationmark.triangle.fill"
            case .error: "xmark.octagon.fill"
            }
        }

        var color: Color {
            switch self {
            case .info: .secondary
            case .success: .green
            case .warning: .orange
            case .error: .red
            }
        }
    }

    let text: String
    let style: Style

    var body: some View {
        Label {
            Text(text).fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: style.symbol).foregroundStyle(style.color)
        }
    }
}

extension View {
    /// Secondary explanatory text under a control or section (wrapped lines stay leading-aligned,
    /// which section footers do not do by default).
    func footnoteStyle() -> some View {
        font(.callout)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.leading)
    }
}

/// "Show in Finder": selects the file, or opens its folder when the file does not exist (yet).
func revealInFinder(_ url: URL) {
    if FileManager.default.fileExists(atPath: url.path) {
        NSWorkspace.shared.activateFileViewerSelecting([url])
    } else {
        NSWorkspace.shared.open(url.deletingLastPathComponent())
    }
}

/// `~/…` form of a file path.
func displayPath(_ url: URL) -> String {
    (url.path as NSString).abbreviatingWithTildeInPath
}
