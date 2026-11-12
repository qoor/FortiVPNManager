import AppKit
import SwiftUI

/// Panes of the settings window, in sidebar order. The settings panes come first; the log comes last
/// because it is not a setting (the user chose it as a pane over a separate window).
enum SettingsPane: String, CaseIterable {
    case general
    case account
    case connection
    case exceptionRules
    case advanced
    case log

    var title: String {
        String(localized: titleResource)
    }

    /// The English name, which the settings search matches as well as `title`.
    var englishTitle: String {
        titleResource.key
    }

    private var titleResource: LocalizedStringResource {
        switch self {
        case .general: "General"
        case .account: "Account"
        case .connection: "Connection"
        case .exceptionRules: "Exception Rules"
        case .advanced: "Advanced"
        case .log: "Log"
        }
    }

    var symbolName: String {
        switch self {
        case .general: "gearshape"
        case .account: "person.crop.circle"
        case .connection: "network"
        case .exceptionRules: "arrow.triangle.branch"
        case .advanced: "wrench.and.screwdriver"
        case .log: "doc.plaintext"
        }
    }
}

/// The settings window in the style of macOS System Settings (`SettingsView`). Like System Settings it
/// keeps one size on every pane, can't go full screen, opens centered the first time and then where it
/// was left, on the last pane shown. While it is open the app is a regular app (Dock icon, key focus,
/// the standard menus); closing it returns the app to the menu bar only.
@MainActor
final class SettingsWindowController: NSObject, NSWindowDelegate, NSMenuItemValidation {
    /// System Settings' window size on macOS 27 (measured).
    static let size = CGSize(width: 740, height: 625)
    static let windowID = NSUserInterfaceItemIdentifier("settings")

    let navigation = SettingsNavigation()
    private let ui: AppUI
    private var window: NSWindow?
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
        if let pane { navigation.select(pane) }
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
        // After SwiftUI has laid out the pane.
        DispatchQueue.main.async { [self] in placeFocus() }
    }

    private func makeWindow() -> NSWindow {
        let content = NSHostingController(rootView: SettingsView(ui: ui, navigation: navigation))
        // The window keeps its size and takes the toolbar from SwiftUI; the title is set below (a
        // navigation title inside the split view doesn't reach an AppKit window).
        content.sizingOptions = []
        content.sceneBridgingOptions = [.toolbars]
        let window = NSWindow(contentViewController: content)
        window.identifier = Self.windowID
        window.styleMask = [.titled, .closable, .miniaturizable, .fullSizeContentView]
        window.toolbarStyle = .unified
        window.collectionBehavior.insert(.fullScreenNone)
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.title = navigation.pane.title
        navigation.onPaneChange = { [weak window] in window?.title = $0.title }
        navigation.onPaneWillChange = { [weak window] in
            // Not the search field's typing, which goes on in the sidebar.
            guard let window, let editor = window.firstResponder as? NSTextView, editor.isFieldEditor,
                  !(editor.delegate is NSSearchField) else { return }
            window.makeFirstResponder(nil)
        }
        navigation.isShown = { [weak ui] in ui?.showsSettings(when: $0) ?? false }
        // The content extends under the toolbar, so this is the whole window.
        window.setContentSize(Self.size)
        // Where it was left; AppKit brings a saved frame back on screen when the displays changed.
        if window.setFrameUsingName(Self.frameName) {
            // At this size, from the same top edge, and still on the screen.
            var frame = NSRect(origin: NSPoint(x: window.frame.minX, y: window.frame.maxY - Self.size.height), size: Self.size)
            if let visible = window.screen?.visibleFrame {
                frame.origin.x = min(max(frame.minX, visible.minX), visible.maxX - frame.width)
                frame.origin.y = min(max(frame.minY, visible.minY), visible.maxY - frame.height)
            }
            window.setFrame(frame, display: false)
        } else if let visible = (window.screen ?? NSScreen.main)?.visibleFrame {
            // Exactly centered, as System Settings opens the first time (`center()` sits higher).
            window.setFrameOrigin(NSPoint(x: visible.midX - Self.size.width / 2, y: visible.midY - Self.size.height / 2))
        }
        window.setFrameAutosaveName(Self.frameName)
        return window
    }

    func windowWillClose(_ notification: Notification) {
        ui.send(.settingsClosed)
        NSApp.setActivationPolicy(.accessory)
    }

    /// Not the names the earlier toolbar windows used ("SettingsWindow", "SettingsPanesWindow"): their
    /// saved sizes don't fit this window.
    private static let frameName = "SettingsSidebarWindow"

    /// No control takes the keyboard when the window opens, so a stray Space cannot press a button
    /// such as Disconnect; Tab still reaches them. On the Log pane its text view does, so Edit > Find
    /// (⌘F) opens its find bar.
    private func placeFocus() {
        guard let window, window.isVisible else { return }
        window.makeFirstResponder(navigation.pane == .log ? logTextView : nil)
    }

    private var logTextView: NSTextView? {
        window?.contentView?.firstDescendant(NSTextView.self) { $0.usesFindBar }
    }

    // MARK: - Menu commands

    @objc func goBack(_ sender: Any?) {
        navigation.goBack()
    }

    @objc func goForward(_ sender: Any?) {
        navigation.goForward()
    }

    @objc func searchSettings(_ sender: Any?) {
        navigation.focusSearch()
    }

    /// File > Save and Revert act on the shown pane's changes: the account, or the exception rules
    /// (Apply and Discard Changes).
    @objc func saveChanges(_ sender: Any?) {
        guard changesButton(save: true) == .enabled else { return }
        switch navigation.pane {
        case .account: ui.send(.saveAccount)
        case .exceptionRules: ui.applyBypassRules()
        default: break
        }
    }

    @objc func revertChanges(_ sender: Any?) {
        guard changesButton(save: false) == .enabled else { return }
        switch navigation.pane {
        case .account: ui.send(.revertAccount)
        case .exceptionRules: ui.discardBypassChanges()
        default: break
        }
    }

    /// The report of the import shows in the General pane.
    @objc func importClientSettings(_ sender: Any?) {
        navigation.select(.general)
        Task { await ui.importClientSettings() }
    }

    @objc func showLogInFinder(_ sender: Any?) {
        if let logFile = ui.config?.logFile { revealInFinder(logFile) }
    }

    @objc func copyEntireLog(_ sender: Any?) {
        ui.copyEntireLog()
    }

    @objc func clearLog(_ sender: Any?) {
        ui.log.clear()
    }

    /// Edit > Find: on the Log pane it acts on the log, wherever the keyboard is; elsewhere Find…
    /// searches the settings. Sent here rather than along the responder chain, where the search
    /// field's editor would take it.
    @objc func find(_ sender: Any?) {
        if navigation.pane == .log, let logTextView {
            // The keyboard stays in the log's find bar, so Find Next leaves the typing there.
            let inLog = logTextView.enclosingScrollView.map { (window?.firstResponder as? NSView)?.isDescendant(of: $0) == true }
            if inLog != true { window?.makeFirstResponder(logTextView) }
            logTextView.performTextFinderAction(sender)
        } else if (sender as? NSMenuItem)?.tag == NSTextFinder.Action.showFindInterface.rawValue {
            navigation.focusSearch()
        }
    }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(goBack(_:)): !navigation.backHistory.isEmpty
        case #selector(goForward(_:)): !navigation.forwardHistory.isEmpty
        case #selector(saveChanges(_:)):
            validate(item, title: navigation.pane == .exceptionRules ? String(localized: "Apply") : String(localized: "Save"),
                     state: changesButton(save: true))
        case #selector(revertChanges(_:)):
            validate(item, title: navigation.pane == .exceptionRules ? String(localized: "Discard Changes") : String(localized: "Revert"),
                     state: changesButton(save: false))
        case #selector(importClientSettings(_:)): ui.config != nil
        case #selector(showLogInFinder(_:)): ui.config?.logFile != nil
        case #selector(copyEntireLog(_:)), #selector(clearLog(_:)): navigation.pane == .log
        case #selector(find(_:)):
            navigation.pane == .log || item.tag == NSTextFinder.Action.showFindInterface.rawValue
        default: true
        }
    }

    private func validate(_ item: NSMenuItem, title: String, state: AppInternal.ButtonState) -> Bool {
        item.title = title
        return state == .enabled
    }

    /// The state of the shown pane's Save or Apply (`save`), or Revert or Discard Changes.
    private func changesButton(save: Bool) -> AppInternal.ButtonState {
        switch navigation.pane {
        case .account:
            guard let account = ui.config?.account else { return .hidden }
            return save ? account.save : account.revert
        case .exceptionRules:
            guard let rules = ui.bypassRules else { return .hidden }
            return rules.whileEditing(save ? rules.apply : rules.discard, navigation.isEditingRuleRow)
        default:
            return .hidden
        }
    }
}

extension AppUI {
    /// Whether the panes now show the settings that are shown only when `condition` holds.
    func showsSettings(when condition: SettingsSearch.Condition) -> Bool {
        switch condition {
        case .connected:
            if case .connected? = connection?.status { true } else { false }
        case .gmailApi:
            gmail?.method == .gmailApi
        case .vpnClientFound:
            readiness?.client != nil
        }
    }

    /// The VPN client's settings file at its default location, or one the user picks when there is
    /// none. Its report shows in the General pane.
    func importClientSettings() async {
        guard let account = config?.account else { return }
        if account.defaultImportFile != nil {
            send(.importClientSettings(file: nil))
            return
        }
        let panel = NSOpenPanel()
        panel.message = String(localized: "Choose an openfortivpn config file.")
        panel.prompt = String(localized: "Import")
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.showsHiddenFiles = true
        panel.directoryURL = FileManager.default.homeDirectoryForCurrentUser.appending(path: ".config")
        guard let url = await chooseFile(panel) else { return }
        send(.importClientSettings(file: url.path))
    }

    /// Finishes a row being typed so its text is sent first (buttons do not take focus).
    func applyBypassRules() {
        NSApp.keyWindow?.makeFirstResponder(nil)
        send(.applyBypassRules)
    }

    func discardBypassChanges() {
        NSApp.keyWindow?.makeFirstResponder(nil)
        send(.discardBypassChanges)
    }

    func copyEntireLog(timeZone: TimeZone = .current) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(log.copyAllText(timeZone: timeZone), forType: .string)
    }
}

extension AppInternal.BypassRulesSection {
    /// Apply and Discard Changes take clicks while a row is typed, whose text the runtime hasn't seen.
    func whileEditing(_ state: AppInternal.ButtonState, _ isEditing: Bool) -> AppInternal.ButtonState {
        isEditing && state == .disabled ? .enabled : state
    }
}

/// The menu bar's standard menus, shown while the settings window makes the app a regular app, so
/// the editing, find and close shortcuts work as in any Mac app. Every action of the window's toolbar
/// is here too (HIG Toolbars, macOS).
@MainActor
private enum StandardMenus {
    static func make(settings: SettingsWindowController) -> NSMenu {
        let main = NSMenu()
        main.addItem(menu("FortiVPNManager", [
            item(String(localized: "About FortiVPNManager"), #selector(NSApplication.orderFrontStandardAboutPanel(_:))),
            .separator(),
            item(String(localized: "Settings…"), #selector(SettingsWindowController.showFromMenu(_:)), ",", target: settings),
            .separator(),
            item(String(localized: "Hide FortiVPNManager"), #selector(NSApplication.hide(_:)), "h"),
            item(String(localized: "Hide Others"), #selector(NSApplication.hideOtherApplications(_:)), "h", [.command, .option]),
            item(String(localized: "Show All"), #selector(NSApplication.unhideAllApplications(_:))),
            .separator(),
            item(String(localized: "Quit FortiVPNManager"), #selector(NSApplication.terminate(_:)), "q"),
        ]))
        main.addItem(menu(String(localized: "File"), [
            item(String(localized: "Save"), #selector(SettingsWindowController.saveChanges(_:)), "s", target: settings),
            item(String(localized: "Revert"), #selector(SettingsWindowController.revertChanges(_:)), target: settings),
            .separator(),
            item(String(localized: "Import openfortivpn Settings…"), #selector(SettingsWindowController.importClientSettings(_:)), target: settings),
            item(String(localized: "Show Log in Finder"), #selector(SettingsWindowController.showLogInFinder(_:)), target: settings),
            .separator(),
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
                find(String(localized: "Find…"), .showFindInterface, "f", settings: settings),
                find(String(localized: "Find Next"), .nextMatch, "g", settings: settings),
                find(String(localized: "Find Previous"), .previousMatch, "g", [.command, .shift], settings: settings),
                find(String(localized: "Use Selection for Find"), .setSearchString, "e", settings: settings),
                .separator(),
                item(String(localized: "Search Settings…"), #selector(SettingsWindowController.searchSettings(_:)), "f", [.command, .option], target: settings),
            ]),
            .separator(),
            item(String(localized: "Copy Entire Log"), #selector(SettingsWindowController.copyEntireLog(_:)), target: settings),
            item(String(localized: "Clear Log"), #selector(SettingsWindowController.clearLog(_:)), "k", target: settings),
        ]))
        main.addItem(menu(String(localized: "View"), [
            item(String(localized: "Back"), #selector(SettingsWindowController.goBack(_:)), "[", target: settings),
            item(String(localized: "Forward"), #selector(SettingsWindowController.goForward(_:)), "]", target: settings),
        ]))
        // The window can be minimized like System Settings; AppKit adds the tiling items and the
        // window list.
        let window = menu(String(localized: "Window"), [
            item(String(localized: "Minimize"), #selector(NSWindow.performMiniaturize(_:)), "m"),
            item(String(localized: "Zoom"), #selector(NSWindow.performZoom(_:))),
            .separator(),
            item(String(localized: "Bring All to Front"), #selector(NSApplication.arrangeInFront(_:))),
        ])
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
        _ title: String, _ action: Selector, _ key: String = "", _ modifiers: NSEvent.ModifierFlags = .command,
        target: AnyObject? = nil
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = modifiers
        item.target = target
        return item
    }

    /// The text finder action is the tag; the settings window's controller passes it to the log.
    private static func find(
        _ title: String, _ action: NSTextFinder.Action, _ key: String, _ modifiers: NSEvent.ModifierFlags = .command,
        settings: SettingsWindowController
    ) -> NSMenuItem {
        let item = item(title, #selector(SettingsWindowController.find(_:)), key, modifiers, target: settings)
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

    /// A setting that belongs to the switch or disclosure above it, indented under it (16pt, as the
    /// settings mockup the user chose).
    func subordinate() -> some View {
        padding(.leading, 16)
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
        $0.identifier == SettingsWindowController.windowID && $0.isVisible && $0.attachedSheet == nil
    }
    let response = if let host {
        await panel.beginSheetModal(for: host)
    } else {
        await panel.begin()
    }
    return response == .OK ? panel.url : nil
}
