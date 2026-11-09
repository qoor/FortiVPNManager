import AppKit
import Observation
import SwiftUI

/// The tab the settings window shows; shared by the window and whoever opens it.
@MainActor
@Observable
final class SettingsNavigation {
    var selection: SettingsTab = .account
}

/// The settings window: one reusable `NSWindow` hosting the SwiftUI tabs. While it is open the app
/// is a regular app (Dock icon, key focus); closing it returns the app to the menu bar only.
@MainActor
final class SettingsWindowController: NSObject, NSWindowDelegate {
    let navigation = SettingsNavigation()

    private let ui: AppUI
    private var window: SettingsWindow?

    init(ui: AppUI) {
        self.ui = ui
    }

    /// Opens (or brings forward) the window, optionally switching to `tab`. Deferred to the next
    /// main-queue turn so a menu that is still tracking cannot swallow the activation.
    func show(_ tab: SettingsTab? = nil) {
        if let tab { navigation.selection = tab }
        DispatchQueue.main.async { [self] in present() }
    }

    private func present() {
        let window = self.window ?? makeWindow()
        self.window = window
        ui.send(.settingsOpened)

        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
        if !window.isKeyWindow {
            window.orderFrontRegardless()
        }
    }

    private func makeWindow() -> SettingsWindow {
        let hosting = NSHostingController(rootView: SettingsView(ui: ui, navigation: navigation))
        hosting.sizingOptions = [.minSize]

        let window = SettingsWindow(contentViewController: hosting)
        window.title = String(localized: "FortiVPNManager Settings")
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable]
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.setContentSize(NSSize(width: 680, height: 620))
        if !window.setFrameUsingName(Self.frameName) {
            window.center()
        }
        window.setFrameAutosaveName(Self.frameName)
        return window
    }

    func windowWillClose(_ notification: Notification) {
        ui.send(.settingsClosed)
        NSApp.setActivationPolicy(.accessory)
    }

    private static let frameName = "SettingsWindow"
}

/// Handles the standard editing shortcuts itself: a menu-bar-only app has no Edit menu, so without
/// this ⌘V would never reach a `SecureField`, and ⌘F would not open the log's find bar.
final class SettingsWindow: NSWindow {
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.type == .keyDown, let command = Self.command(for: event), perform(command) {
            return true
        }
        return super.performKeyEquivalent(with: event)
    }

    private enum Command {
        case action(Selector)
        case find
        case close
    }

    private static func command(for event: NSEvent) -> Command? {
        let modifiers = event.modifierFlags.intersection([.command, .shift, .option, .control])
        switch (key(of: event), modifiers) {
        case ("x", [.command]): return .action(#selector(NSText.cut(_:)))
        case ("c", [.command]): return .action(#selector(NSText.copy(_:)))
        case ("v", [.command]): return .action(#selector(NSText.paste(_:)))
        case ("a", [.command]): return .action(#selector(NSText.selectAll(_:)))
        case ("z", [.command]): return .action(Selector(("undo:")))
        case ("z", [.command, .shift]): return .action(Selector(("redo:")))
        case ("f", [.command]): return .find
        case ("w", [.command]): return .close
        default: return nil
        }
    }

    /// The letter of the key. Falls back to the ANSI key position when the layout or input source
    /// reports a non-Latin character.
    private static func key(of event: NSEvent) -> String? {
        if let characters = event.charactersIgnoringModifiers?.lowercased(),
           characters.count == 1, characters.first?.isASCII == true {
            return characters
        }
        return ansiKeys[event.keyCode]
    }

    /// kVK_ANSI_* key codes of the shortcuts above.
    private static let ansiKeys: [UInt16: String] = [0: "a", 3: "f", 6: "z", 7: "x", 8: "c", 9: "v", 13: "w"]

    private func perform(_ command: Command) -> Bool {
        switch command {
        case let .action(selector):
            return NSApp.sendAction(selector, to: nil, from: self)
        case .find:
            return showFindBar()
        case .close:
            performClose(nil)
            return true
        }
    }

    /// Opens the find bar of the visible log text view.
    private func showFindBar() -> Bool {
        guard let contentView, let textView = Self.findableTextView(in: contentView) else { return false }
        makeFirstResponder(textView)
        let sender = NSMenuItem()
        sender.tag = NSTextFinder.Action.showFindInterface.rawValue
        textView.performTextFinderAction(sender)
        return true
    }

    private static func findableTextView(in view: NSView) -> NSTextView? {
        if let textView = view as? NSTextView, textView.usesFindBar, !textView.isHiddenOrHasHiddenAncestor {
            return textView
        }
        for subview in view.subviews {
            if let found = findableTextView(in: subview) { return found }
        }
        return nil
    }
}

struct SettingsView: View {
    let ui: AppUI
    @Bindable var navigation: SettingsNavigation

    var body: some View {
        TabView(selection: $navigation.selection) {
            Tab("Account", systemImage: "person.crop.circle", value: SettingsTab.account) {
                AccountTab(ui: ui)
            }
            Tab("Connection", systemImage: "network", value: SettingsTab.connection) {
                ConnectionTab(ui: ui)
            }
            Tab("Log", systemImage: "doc.plaintext", value: SettingsTab.log) {
                LogTab(ui: ui)
            }
        }
        .padding(.top, 8)
        .frame(minWidth: 600, minHeight: 520)
    }
}

/// How a line of status is marked.
enum NoticeStyle {
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

/// One line of status with a coloured symbol.
struct NoticeRow: View {
    let text: String
    let style: NoticeStyle

    var body: some View {
        Label {
            Text(verbatim: text).fixedSize(horizontal: false, vertical: true)
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
@MainActor
func revealInFinder(_ path: String) {
    let url = URL(filePath: path)
    if FileManager.default.fileExists(atPath: url.path) {
        NSWorkspace.shared.activateFileViewerSelecting([url])
    } else {
        NSWorkspace.shared.open(url.deletingLastPathComponent())
    }
}

/// `~/…` form of a file path.
func displayPath(_ path: String) -> String {
    (path as NSString).abbreviatingWithTildeInPath
}

/// Shows an open panel as a sheet on the settings window (a free-standing panel if that window is
/// gone). Never `runModal()`: a modal loop entered from a main-actor task does not drain the main
/// queue, so the messages from the runtime would stall until the panel closes.
@MainActor
func chooseFile(_ panel: NSOpenPanel) async -> URL? {
    let host = NSApp.windows.first { $0 is SettingsWindow && $0.isVisible && $0.attachedSheet == nil }
    let response = if let host {
        await panel.beginSheetModal(for: host)
    } else {
        await panel.begin()
    }
    return response == .OK ? panel.url : nil
}
