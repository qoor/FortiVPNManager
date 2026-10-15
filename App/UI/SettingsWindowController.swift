import AppKit
import Observation
import SwiftUI

/// Tabs of the settings window (DESIGN §5.6).
enum SettingsTab: Hashable {
    case account
    case connection
    case log
}

/// The tab the settings window shows; shared by the window and whoever opens it.
@Observable
final class SettingsNavigation {
    var selection: SettingsTab = .account
}

/// The settings window (DESIGN §5.6): one reusable `NSWindow` hosting the three SwiftUI tabs.
/// While it is open the app is a regular app (Dock icon, key focus); closing it returns the app
/// to the menu bar only.
final class SettingsWindowController: NSObject, NSWindowDelegate {
    let navigation = SettingsNavigation()
    let accountEditor: AccountEditor

    private let environment: AppEnvironment
    private let clock: StatusClock
    private let accountNotice: AccountChangeNotice
    private var window: SettingsWindow?

    init(environment: AppEnvironment, clock: StatusClock, accountNotice: AccountChangeNotice) {
        self.environment = environment
        self.clock = clock
        self.accountNotice = accountNotice
        accountEditor = AccountEditor(environment: environment)
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
        accountEditor.reloadIfUnchanged()
        environment.loginItems.refresh()
        environment.connection.recheckReadiness()

        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
        if !window.isKeyWindow {
            window.orderFrontRegardless()
        }
    }

    private func makeWindow() -> SettingsWindow {
        let root = SettingsView(
            environment: environment,
            navigation: navigation,
            accountEditor: accountEditor,
            clock: clock,
            accountNotice: accountNotice
        )
        let hosting = NSHostingController(rootView: root)
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
        accountEditor.windowClosed()
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
