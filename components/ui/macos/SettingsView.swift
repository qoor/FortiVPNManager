import AppKit
import SwiftUI

/// The settings window's content, laid out like macOS System Settings: a sidebar with the settings
/// search above the panes, and the pane on the right with Back and Forward and the pane's own actions
/// in the toolbar.
struct SettingsView: View {
    let ui: AppUI
    @Bindable var navigation: SettingsNavigation
    /// The log's text view outlives its pane, as the earlier tab view kept it: coming back keeps the
    /// place read and the find bar, and the lines aren't laid out again.
    @State private var logView = LogTextView.Coordinator()

    /// Measured against System Settings' 232pt sidebar and chosen narrower by the user.
    static let sidebarWidth: CGFloat = 220

    var body: some View {
        NavigationSplitView {
            SettingsSidebar(navigation: navigation)
                .frame(width: Self.sidebarWidth)
                .navigationSplitViewColumnWidth(Self.sidebarWidth)
                .background { SidebarWidthLock(width: Self.sidebarWidth).frame(width: 0, height: 0) }
                .toolbar(removing: .sidebarToggle)
        } detail: {
            pane
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .toolbar {
                    // Always there, so the title bar keeps its height on every pane. The View menu
                    // holds the ⌘[ and ⌘] shortcuts.
                    ToolbarItemGroup(placement: .navigation) {
                        Button("Back", systemImage: "chevron.backward") { navigation.goBack() }
                            .disabled(navigation.backHistory.isEmpty)
                            .help("Go back to the previous pane (⌘[)")
                        Button("Forward", systemImage: "chevron.forward") { navigation.goForward() }
                            .disabled(navigation.forwardHistory.isEmpty)
                            .help("Go forward to the next pane (⌘])")
                    }
                }
        }
        .navigationSplitViewStyle(.prominentDetail)
    }

    @ViewBuilder
    private var pane: some View {
        switch navigation.pane {
        case .general: GeneralPane(ui: ui, navigation: navigation)
        case .account: AccountPane(ui: ui, navigation: navigation)
        case .connection: ConnectionPane(ui: ui, navigation: navigation)
        case .exceptionRules: ExceptionRulesPane(ui: ui, navigation: navigation)
        case .advanced: AdvancedPane(ui: ui, navigation: navigation)
        case .log: LogPane(ui: ui, navigation: navigation, logView: logView)
        }
    }
}

/// The search field and the panes, or the panes with the settings that match the search.
private struct SettingsSidebar: View {
    @Bindable var navigation: SettingsNavigation

    private enum Row: Hashable {
        case pane(SettingsPane)
        case result(SettingsSearch.Item)
    }

    var body: some View {
        VStack(spacing: 0) {
            // System Settings' search field: 28pt high, 10pt from the sidebar's edges, 61pt from the
            // top of the window.
            SettingsSearchField(
                text: $navigation.searchText,
                focusRequest: navigation.searchFocusRequest,
                onSubmit: navigation.revealFirstResult
            )
            .controlSize(.large)
            .frame(height: 28)
            .padding(.horizontal, 10)
            .padding(.top, 9)
            .padding(.bottom, 8)
            List(selection: selection) {
                ForEach(navigation.visiblePanes, id: \.self) { pane in
                    Label(pane.title, systemImage: pane.symbolName)
                        .tag(Row.pane(pane))
                    ForEach(navigation.results(in: pane), id: \.self) { item in
                        // Long names wrap, as in System Settings' results.
                        Text(item.title)
                            .lineLimit(3)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.leading, 25)
                            .tag(Row.result(item))
                    }
                }
            }
            .listStyle(.sidebar)
            .overlay {
                if navigation.visiblePanes.isEmpty {
                    Text("No Results")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    /// Typing doesn't change the pane; choosing a pane or a result does.
    private var selection: Binding<Row?> {
        Binding(
            get: {
                if let item = navigation.chosenResult, navigation.results(in: item.pane).contains(item) {
                    return .result(item)
                }
                return .pane(navigation.pane)
            },
            set: { row in
                switch row {
                case let .pane(pane)?: navigation.select(pane)
                case let .result(item)?: navigation.reveal(item)
                case nil: break
                }
            }
        )
    }
}

/// The sidebar's search field. Return opens the first result; Down Arrow opens it and moves the
/// keyboard into the list, as in System Settings. A syllable the input method is still composing
/// searches too.
private struct SettingsSearchField: NSViewRepresentable {
    @Binding var text: String
    let focusRequest: Int
    let onSubmit: @MainActor () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text, focusRequest: focusRequest, onSubmit: onSubmit)
    }

    func makeNSView(context: Context) -> NSSearchField {
        let field = NSSearchField()
        field.placeholderString = String(localized: "Search")
        field.setAccessibilityLabel(String(localized: "Search"))
        field.sendsSearchStringImmediately = true
        field.isContinuous = true
        field.delegate = context.coordinator
        return field
    }

    func updateNSView(_ field: NSSearchField, context: Context) {
        context.coordinator.text = $text
        context.coordinator.onSubmit = onSubmit
        if field.stringValue != text { field.stringValue = text }
        if context.coordinator.focusRequest != focusRequest {
            context.coordinator.focusRequest = focusRequest
            DispatchQueue.main.async {
                field.window?.makeFirstResponder(field)
                field.selectText(nil)
            }
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var text: Binding<String>
        var focusRequest: Int
        var onSubmit: @MainActor () -> Void

        init(text: Binding<String>, focusRequest: Int, onSubmit: @escaping @MainActor () -> Void) {
            self.text = text
            self.focusRequest = focusRequest
            self.onSubmit = onSubmit
        }

        func controlTextDidChange(_ notification: Notification) {
            // The field editor holds the syllable being composed, which the field's value doesn't yet.
            if let editor = notification.userInfo?["NSFieldEditor"] as? NSText {
                text.wrappedValue = editor.string
            } else if let field = notification.object as? NSSearchField {
                text.wrappedValue = field.stringValue
            }
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            switch selector {
            case #selector(NSResponder.insertNewline(_:)):
                onSubmit()
            case #selector(NSResponder.moveDown(_:)):
                onSubmit()
                var ancestor = control.superview
                while let view = ancestor {
                    if let list = view.firstDescendant(NSOutlineView.self) {
                        control.window?.makeFirstResponder(list)
                        break
                    }
                    ancestor = view.superview
                }
            default:
                return false
            }
            return true
        }
    }
}

extension NSView {
    /// The first view of type `T` under this one, depth first, that passes `test`.
    func firstDescendant<T: NSView>(_ type: T.Type, where test: (T) -> Bool = { _ in true }) -> T? {
        for view in subviews {
            if let match = view as? T, test(match) { return match }
            if let match = view.firstDescendant(type, where: test) { return match }
        }
        return nil
    }
}

/// Holds the sidebar at its width. The split view restores the width it saved last, which the column
/// width alone doesn't override, so the split view item's limits and the divider are set too.
private struct SidebarWidthLock: NSViewRepresentable {
    let width: CGFloat

    func makeNSView(context: Context) -> LockView { LockView(width: width) }
    func updateNSView(_ view: LockView, context: Context) { view.schedule() }

    @MainActor
    final class LockView: NSView {
        private let width: CGFloat
        private var scheduled = false

        init(width: CGFloat) {
            self.width = width
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("not used")
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            schedule()
        }

        func schedule() {
            guard window != nil, !scheduled else { return }
            scheduled = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                scheduled = false
                var ancestor = superview
                while let view = ancestor {
                    if let split = view as? NSSplitView, split.isVertical, split.subviews.count > 1 {
                        if let controller = split.delegate as? NSSplitViewController, let sidebar = controller.splitViewItems.first {
                            sidebar.minimumThickness = width
                            sidebar.maximumThickness = width
                        }
                        split.setPosition(width, ofDividerAt: 0)
                        return
                    }
                    ancestor = view.superview
                }
            }
        }
    }
}

/// A pane's grouped form, which scrolls to the row of a chosen search result.
struct SettingsForm<Content: View>: View {
    let navigation: SettingsNavigation
    let content: Content

    init(navigation: SettingsNavigation, @ViewBuilder content: () -> Content) {
        self.navigation = navigation
        self.content = content()
    }

    var body: some View {
        ScrollViewReader { proxy in
            Form { content }
                .formStyle(.grouped)
                .task(id: navigation.highlightRequest) {
                    guard let row = navigation.highlightedRow else { return }
                    // Lets a pane just shown or a group just opened lay out first.
                    try? await Task.sleep(for: .milliseconds(100))
                    withAnimation { proxy.scrollTo(row, anchor: .center) }
                }
        }
    }
}

extension View {
    /// The row a search result for `row` scrolls to and highlights for a moment.
    func searchAnchor(_ row: String, _ navigation: SettingsNavigation) -> some View {
        id(row).overlay {
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(Color.accentColor.opacity(0.12))
                .stroke(Color.accentColor, lineWidth: 2)
                .padding(-6)
                .opacity(navigation.highlightedRow == row ? 1 : 0)
                .animation(.easeInOut(duration: 0.4), value: navigation.highlightedRow == row)
                .allowsHitTesting(false)
        }
    }
}

/// A row's label with an explanation under it, as System Settings explains a row.
struct RowTitle: View {
    let title: LocalizedStringKey
    let detail: Text?

    init(_ title: LocalizedStringKey, detail: Text? = nil) {
        self.title = title
        self.detail = detail
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
            if let detail {
                detail.footnoteStyle()
            }
        }
    }
}
