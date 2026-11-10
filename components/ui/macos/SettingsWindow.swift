import AppKit
import SwiftUI

/// Panes of the settings window, in toolbar order. The settings panes come first; the log comes last
/// because it is not a setting (the user chose it as a pane over a separate window).
enum SettingsPane: String, CaseIterable {
    case general
    case account
    case connection
    case exceptionRules
    case advanced
    case log

    /// Every pane has the same width, so only the height changes when switching.
    static let width: CGFloat = 680

    /// Each pane's fixed size, which the window takes when the pane is shown (a pane with more
    /// content scrolls).
    var size: CGSize {
        let height: CGFloat = switch self {
        case .general: 470
        case .account: 740
        case .connection: 600
        case .exceptionRules: 660
        case .advanced: 470
        case .log: 560
        }
        return CGSize(width: Self.width, height: height)
    }

    var title: String {
        switch self {
        case .general: String(localized: "General")
        case .account: String(localized: "Account")
        case .connection: String(localized: "Connection")
        case .exceptionRules: String(localized: "Exception Rules")
        case .advanced: String(localized: "Advanced")
        case .log: String(localized: "Log")
        }
    }

    var symbolName: String {
        switch self {
        case .general: "gearshape"
        case .account: "person.crop.circle"
        case .connection: "network"
        case .exceptionRules: "arrow.triangle.branch"
        case .advanced: "gearshape.2"
        case .log: "doc.plaintext"
        }
    }
}

/// The settings window in the macOS settings style: a toolbar switches the panes, the window takes
/// each pane's size and its name as title, and it reopens on the last pane shown. While it is open
/// the app is a regular app (Dock icon, key focus, the standard menus); closing it returns the app
/// to the menu bar only.
@MainActor
final class SettingsWindowController: NSObject, NSWindowDelegate {
    private let ui: AppUI
    private var window: NSWindow?
    private var panes: SettingsPanesController?
    private lazy var menus = StandardMenus.make(settings: self)

    init(ui: AppUI) {
        self.ui = ui
    }

    /// Opens (or brings forward) the window on `pane`, or on the last pane shown. Deferred to the
    /// next main-queue turn so a menu that is still tracking cannot swallow the activation.
    func show(_ pane: SettingsPane? = nil) {
        DispatchQueue.main.async { [self] in present(pane) }
    }

    @objc func showFromMenu(_ sender: Any?) {
        show()
    }

    private func present(_ pane: SettingsPane?) {
        let window = self.window ?? makeWindow()
        self.window = window
        if let pane { panes?.select(pane) }
        ui.send(.settingsOpened)

        if NSApp.mainMenu !== menus { NSApp.mainMenu = menus }
        NSApp.setActivationPolicy(.regular)
        // With another app in front, the system declines `NSApp.activate()` even right after a
        // click in this app's menu (cooperative activation; measured on macOS 27), so the window
        // showed without key focus and shortcuts went to the other app. Taking over from the front
        // app is accepted.
        if let front = NSWorkspace.shared.frontmostApplication, front != NSRunningApplication.current {
            NSRunningApplication.current.activate(from: front, options: [])
        } else {
            NSApp.activate()
        }
        window.makeKeyAndOrderFront(nil)
        if !window.isKeyWindow {
            window.orderFrontRegardless()
        }
        panes?.placeFocus()
    }

    private func makeWindow() -> NSWindow {
        // Read first: loading the panes selects the first one, which is recorded like any selection.
        let lastPane = SettingsPanesController.lastPane
        let panes = SettingsPanesController(ui: ui)
        self.panes = panes
        // Not resizable and not minimizable: the window follows the pane, and ⌘, reopens it.
        let window = NSWindow(contentViewController: panes)
        window.styleMask = [.titled, .closable]
        window.toolbarStyle = .preference
        window.isReleasedWhenClosed = false
        window.delegate = self
        // The saved frame gives the position; selecting the pane then fits the size to it.
        if !window.setFrameUsingName(Self.frameName) {
            window.setContentSize(lastPane.size)
            window.center()
        }
        window.setFrameAutosaveName(Self.frameName)
        panes.select(lastPane)
        return window
    }

    func windowWillClose(_ notification: Notification) {
        ui.send(.settingsClosed)
        NSApp.setActivationPolicy(.accessory)
    }

    /// Not the name the resizable tab window used: its saved size does not fit the panes.
    private static let frameName = "SettingsPanesWindow"
}

/// The panes as toolbar items. Selecting one fits the window to it, keeping the top edge in place,
/// and remembers it for the next opening.
private final class SettingsPanesController: NSTabViewController {
    static var lastPane: SettingsPane {
        UserDefaults.standard.string(forKey: lastPaneKey).flatMap(SettingsPane.init(rawValue:)) ?? .general
    }

    private static let lastPaneKey = "settingsPane"

    init(ui: AppUI) {
        super.init(nibName: nil, bundle: nil)
        tabStyle = .toolbar
        for pane in SettingsPane.allCases {
            let controller = Self.controller(for: pane, ui: ui)
            controller.title = pane.title
            let item = NSTabViewItem(viewController: controller)
            item.identifier = pane.rawValue
            item.label = pane.title
            item.image = NSImage(systemSymbolName: pane.symbolName, accessibilityDescription: nil)
            addTabViewItem(item)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used")
    }

    func select(_ pane: SettingsPane) {
        guard let index = SettingsPane.allCases.firstIndex(of: pane) else { return }
        if selectedTabViewItemIndex == index {
            fitWindow(to: pane)
        } else {
            selectedTabViewItemIndex = index
        }
    }

    override func tabView(_ tabView: NSTabView, didSelect tabViewItem: NSTabViewItem?) {
        super.tabView(tabView, didSelect: tabViewItem)
        guard let pane = (tabViewItem?.identifier as? String).flatMap(SettingsPane.init(rawValue:)) else { return }
        UserDefaults.standard.set(pane.rawValue, forKey: Self.lastPaneKey)
        fitWindow(to: pane)
        placeFocus()
    }

    /// No control takes the keyboard when a pane appears, so a stray Space cannot press a button
    /// such as Disconnect; Tab still reaches them. The log's text view does take it, so Edit > Find
    /// (⌘F) opens its find bar.
    func placeFocus() {
        guard let window = view.window else { return }
        let paneView = tabViewItems[selectedTabViewItemIndex].viewController?.view
        window.makeFirstResponder(paneView.flatMap(Self.textView(in:)))
    }

    /// The window's content takes the pane's size, keeping the top edge in place. A hidden window
    /// is sized by AppKit; a visible one animates, with the title bar and toolbar height measured
    /// from its current frame.
    private func fitWindow(to pane: SettingsPane) {
        guard let window = view.window, let content = window.contentView else { return }
        window.title = pane.title
        guard content.frame.size != pane.size else { return }
        let top = window.frame.maxY
        if window.isVisible {
            let chrome = window.frame.height - content.frame.height
            let height = pane.size.height + chrome
            let frame = NSRect(x: window.frame.minX, y: top - height, width: pane.size.width, height: height)
            window.setFrame(Self.onScreen(frame, of: window), display: true, animate: true)
        } else {
            window.setContentSize(pane.size)
            window.setFrameTopLeftPoint(NSPoint(x: window.frame.minX, y: top))
            window.setFrame(Self.onScreen(window.frame, of: window), display: false)
        }
    }

    /// A taller pane moves the window up instead of past the bottom of the screen: the window
    /// cannot be resized, so what falls below the screen would be out of reach.
    private static func onScreen(_ frame: NSRect, of window: NSWindow) -> NSRect {
        guard let visible = (window.screen ?? NSScreen.main)?.visibleFrame, frame.minY < visible.minY else { return frame }
        var frame = frame
        frame.origin.y = min(visible.minY, visible.maxY - frame.height)
        return frame
    }

    private static func textView(in view: NSView) -> NSTextView? {
        if let textView = view as? NSTextView, textView.usesFindBar { return textView }
        for subview in view.subviews {
            if let found = textView(in: subview) { return found }
        }
        return nil
    }

    private static func controller(for pane: SettingsPane, ui: AppUI) -> NSViewController {
        switch pane {
        case .general: host(GeneralPane(ui: ui), pane)
        case .account: host(AccountPane(ui: ui), pane)
        case .connection: host(ConnectionPane(ui: ui), pane)
        case .exceptionRules: host(ExceptionRulesPane(ui: ui), pane)
        case .advanced: host(AdvancedPane(ui: ui), pane)
        case .log: host(LogPane(ui: ui), pane)
        }
    }

    /// The pane's view at its fixed size. No size options: the window is fitted to the pane here,
    /// not by the hosting controller.
    private static func host(_ view: some View, _ pane: SettingsPane) -> NSViewController {
        let controller = NSHostingController(rootView: view.frame(width: pane.size.width, height: pane.size.height))
        controller.sizingOptions = []
        return controller
    }
}

/// The menu bar's standard menus, shown while the settings window makes the app a regular app, so
/// the editing, find and close shortcuts work as in any Mac app.
@MainActor
private enum StandardMenus {
    static func make(settings: SettingsWindowController) -> NSMenu {
        let main = NSMenu()
        let settingsItem = item(String(localized: "Settings…"), #selector(SettingsWindowController.showFromMenu(_:)), ",")
        settingsItem.target = settings
        main.addItem(menu("FortiVPNManager", [
            item(String(localized: "About FortiVPNManager"), #selector(NSApplication.orderFrontStandardAboutPanel(_:))),
            .separator(),
            settingsItem,
            .separator(),
            item(String(localized: "Hide FortiVPNManager"), #selector(NSApplication.hide(_:)), "h"),
            item(String(localized: "Hide Others"), #selector(NSApplication.hideOtherApplications(_:)), "h", [.command, .option]),
            item(String(localized: "Show All"), #selector(NSApplication.unhideAllApplications(_:))),
            .separator(),
            item(String(localized: "Quit FortiVPNManager"), #selector(NSApplication.terminate(_:)), "q"),
        ]))
        main.addItem(menu(String(localized: "File"), [
            item(String(localized: "Close"), #selector(NSWindow.performClose(_:)), "w"),
        ]))
        main.addItem(menu(String(localized: "Edit"), [
            item(String(localized: "Undo"), Selector(("undo:")), "z"),
            item(String(localized: "Redo"), Selector(("redo:")), "z", [.command, .shift]),
            .separator(),
            item(String(localized: "Cut"), #selector(NSText.cut(_:)), "x"),
            item(String(localized: "Copy"), #selector(NSText.copy(_:)), "c"),
            item(String(localized: "Paste"), #selector(NSText.paste(_:)), "v"),
            item(String(localized: "Select All"), #selector(NSText.selectAll(_:)), "a"),
            .separator(),
            menu(String(localized: "Find"), [
                find(String(localized: "Find…"), .showFindInterface, "f"),
                find(String(localized: "Find Next"), .nextMatch, "g"),
                find(String(localized: "Find Previous"), .previousMatch, "g", [.command, .shift]),
                find(String(localized: "Use Selection for Find"), .setSearchString, "e"),
            ]),
        ]))
        let window = menu(String(localized: "Window"), [])
        main.addItem(window)
        NSApp.windowsMenu = window.submenu
        return main
    }

    private static func menu(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
        let submenu = NSMenu(title: title)
        submenu.items = items
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.submenu = submenu
        return item
    }

    private static func item(
        _ title: String, _ action: Selector, _ key: String = "", _ modifiers: NSEvent.ModifierFlags = .command
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = modifiers
        return item
    }

    private static func find(
        _ title: String, _ action: NSTextFinder.Action, _ key: String, _ modifiers: NSEvent.ModifierFlags = .command
    ) -> NSMenuItem {
        let item = item(title, #selector(NSResponder.performTextFinderAction(_:)), key, modifiers)
        item.tag = action.rawValue
        return item
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

    /// A setting that belongs to the switch above it, indented under it.
    func subordinate() -> some View {
        padding(.leading, 20)
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
    let host = NSApp.windows.first {
        $0.contentViewController is SettingsPanesController && $0.isVisible && $0.attachedSheet == nil
    }
    let response = if let host {
        await panel.beginSheetModal(for: host)
    } else {
        await panel.begin()
    }
    return response == .OK ? panel.url : nil
}
