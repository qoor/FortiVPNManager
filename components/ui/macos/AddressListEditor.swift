import AppKit
import Observation
import SwiftUI

/// One list of the Exception Rules: a single column without a border that spans its form row, with
/// + and − in the row below (`AddressListButtons`), as in the settings mockup. Every finished edit
/// reports the whole list through `onChange`; nothing is saved here. `onEditingChanged` tells whether a
/// row is being typed (its text reaches `onChange` only when the edit ends).
struct AddressListEditor: NSViewRepresentable {
    let rows: [String]
    let problemRows: Set<Int>
    /// What is wrong with a problem row, shown beside it (the runtime explains one problem at a time).
    let problemMessages: [Int: RowMessage]
    /// Off once the list holds as many entries as the settings file allows.
    let canAdd: Bool
    /// Shown over the empty list.
    let emptyText: String
    /// Shown in a row being typed.
    let placeholder: String
    /// The height of a short list (at least one row, for the empty text). Beyond it the list grows by
    /// its rows rather than scrolling inside the scrolling form (HIG: no nested scroll views of the
    /// same orientation).
    var minHeight: CGFloat = 0
    let actions: AddressListActions
    let onChange: ([String]) -> Void
    let onEditingChanged: (Bool) -> Void

    struct RowMessage: Equatable {
        /// The short form beside the row.
        let text: String
        /// The full explanation, as its help tag.
        let help: String
    }

    func makeNSView(context: Context) -> AddressListEditorView {
        let view = AddressListEditorView()
        updateNSView(view, context: context)
        return view
    }

    func updateNSView(_ view: AddressListEditorView, context: Context) {
        view.onChange = onChange
        view.onEditingChanged = onEditingChanged
        // Each update reloads the list and reports its selection again; only a change is passed on,
        // so the update doesn't invalidate the buttons for nothing.
        view.onSelectionChanged = { [weak actions] selected in
            if let actions, actions.hasSelection != selected { actions.hasSelection = selected }
        }
        actions.view = view
        view.configure(emptyText: emptyText, placeholder: placeholder)
        view.minHeight = minHeight
        view.canAdd = canAdd
        view.show(rows: rows, problemRows: problemRows, messages: problemMessages)
    }
}

/// The + and − of a list, borderless in the form row below it.
struct AddressListButtons: View {
    let actions: AddressListActions
    let canAdd: Bool
    let addTitle: LocalizedStringKey
    let removeTitle: LocalizedStringKey
    /// Help tag of the + button.
    let addHelp: String

    var body: some View {
        HStack(spacing: 10) {
            Button(addTitle, systemImage: "plus") { actions.add() }
                .disabled(!canAdd)
                .help(addHelp)
            Button(removeTitle, systemImage: "minus") { actions.removeSelected() }
                .disabled(!actions.hasSelection)
                .help("Remove the selected rows")
        }
        .labelStyle(.iconOnly)
        .buttonStyle(.borderless)
    }
}

/// Connects a list with its buttons in the row below.
@MainActor
@Observable
final class AddressListActions {
    fileprivate(set) var hasSelection = false
    @ObservationIgnored fileprivate weak var view: AddressListEditorView?

    func add() {
        view?.addRow()
    }

    func removeSelected() {
        view?.removeSelected()
    }
}

final class AddressListEditorView: NSView, NSTableViewDataSource, NSTableViewDelegate, NSTextFieldDelegate {
    var onChange: (([String]) -> Void)?
    var onEditingChanged: ((Bool) -> Void)?
    var onSelectionChanged: ((Bool) -> Void)?
    var canAdd = true
    var minHeight: CGFloat = 0 {
        didSet { if minHeight != oldValue { invalidateIntrinsicContentSize() } }
    }

    private let scrollView = NSScrollView()
    private let tableView = RulesTableView()
    private let emptyLabel = PassthroughLabel(labelWithString: "")
    private var placeholder = ""

    /// The rows as last shown.
    private var rows: [String] = []
    private var problemRows: Set<Int> = []
    private var messages: [Int: AddressListEditor.RowMessage] = [:]
    /// A row added with + or a double-click, shown after `rows` until its edit ends.
    private var hasNewRow = false
    /// Row being edited and its text before the edit (`nil` for the new row).
    private var editingRow: Int?
    private var editingOriginal: String?
    private var cancelsEdit = false
    /// Rows that arrived while an edit was in progress; shown when it ends.
    private var pending: (rows: [String], problemRows: Set<Int>, messages: [Int: AddressListEditor.RowMessage])?

    private static let cellID = NSUserInterfaceItemIdentifier("addressRow")
    /// Where a row's text starts: the inset of the other rows' labels, the list spanning the form row.
    static let textInset: CGFloat = 10

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setUp()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setUp()
    }

    private var displayCount: Int { rows.count + (hasNewRow ? 1 : 0) }

    func configure(emptyText: String, placeholder: String) {
        if emptyLabel.stringValue != emptyText { emptyLabel.stringValue = emptyText }
        self.placeholder = placeholder
    }

    func show(rows newRows: [String], problemRows newProblems: Set<Int>, messages newMessages: [Int: AddressListEditor.RowMessage]) {
        if editingRow != nil {
            pending = (newRows, newProblems, newMessages)
            return
        }
        guard newRows != rows || newProblems != problemRows || newMessages != messages else { return }
        rows = newRows
        problemRows = newProblems
        messages = newMessages
        reload()
    }

    // MARK: - Layout

    override var intrinsicContentSize: NSSize {
        let rowsHeight = CGFloat(max(displayCount, 1)) * tableView.rowHeight
        return NSSize(width: NSView.noIntrinsicMetric, height: max(minHeight, rowsHeight))
    }

    private func setUp() {
        setContentHuggingPriority(.required, for: .vertical)
        setContentCompressionResistancePriority(.required, for: .vertical)
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("address"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.style = .fullWidth
        // Rows on the form's own background; `AddressRowView` shades every other one.
        tableView.backgroundColor = .clear
        tableView.intercellSpacing = .zero
        tableView.allowsMultipleSelection = true
        tableView.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.doubleAction = #selector(doubleClicked)
        tableView.onDelete = { [weak self] in self?.removeSelected() }
        tableView.onReturn = { [weak self] in self?.editSelected() }

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.drawsBackground = false

        emptyLabel.textColor = .secondaryLabelColor
        emptyLabel.alignment = .center
        emptyLabel.lineBreakMode = .byWordWrapping

        for view in [scrollView, emptyLabel] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),
            emptyLabel.centerYAnchor.constraint(equalTo: scrollView.centerYAnchor),
            emptyLabel.leadingAnchor.constraint(equalTo: scrollView.leadingAnchor, constant: 16),
            emptyLabel.trailingAnchor.constraint(equalTo: scrollView.trailingAnchor, constant: -16),
        ])
        reload()
    }

    private func reload() {
        let selection = tableView.selectedRowIndexes.filteredIndexSet { $0 < displayCount }
        tableView.reloadData()
        tableView.selectRowIndexes(selection, byExtendingSelection: false)
        emptyLabel.isHidden = displayCount > 0
        invalidateIntrinsicContentSize()
        selectionChanged()
    }

    private func selectionChanged() {
        onSelectionChanged?(!tableView.selectedRowIndexes.isEmpty)
    }

    // MARK: - Table

    func numberOfRows(in tableView: NSTableView) -> Int {
        displayCount
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        let view = AddressRowView()
        view.isShaded = row % 2 == 1
        return view
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = tableView.makeView(withIdentifier: Self.cellID, owner: self) as? AddressCellView ?? makeCell()
        cell.field.stringValue = row < rows.count ? rows[row] : ""
        cell.field.placeholderString = placeholder
        cell.field.isEditable = false
        cell.field.isSelectable = false
        cell.showProblem(problemRows.contains(row), message: messages[row])
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        selectionChanged()
    }

    private func makeCell() -> AddressCellView {
        let cell = AddressCellView()
        cell.identifier = Self.cellID
        cell.field.delegate = self
        return cell
    }

    // MARK: - Actions

    /// A double-click below the last row reports row -1.
    @objc private func doubleClicked() {
        let row = tableView.clickedRow
        if row < 0 {
            addRow()
        } else {
            beginEditing(row)
        }
    }

    fileprivate func addRow() {
        if editingRow != nil { window?.makeFirstResponder(tableView) }
        guard canAdd else { return }
        if !hasNewRow {
            hasNewRow = true
            reload()
        }
        beginEditing(rows.count)
    }

    private func editSelected() {
        let selected = tableView.selectedRowIndexes
        if selected.count == 1, let row = selected.first { beginEditing(row) }
    }

    fileprivate func removeSelected() {
        guard editingRow == nil else { return }
        let selected = tableView.selectedRowIndexes
        guard !selected.isEmpty else { return }
        if hasNewRow, selected.contains(rows.count) { hasNewRow = false }
        let remaining = rows.indices.filter { !selected.contains($0) }.map { rows[$0] }
        tableView.deselectAll(nil)
        if remaining == rows {
            reload()
            return
        }
        rows = remaining
        problemRows = []
        messages = [:]
        reload()
        onChange?(remaining)
    }

    private func beginEditing(_ row: Int) {
        guard row < displayCount else { return }
        tableView.scrollRowToVisible(row)
        tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        guard let cell = tableView.view(atColumn: 0, row: row, makeIfNecessary: true) as? AddressCellView else { return }
        let field = cell.field
        editingRow = row
        editingOriginal = row < rows.count ? rows[row] : nil
        field.isEditable = true
        field.isSelectable = true
        cell.showProblem(false, message: nil)
        window?.makeFirstResponder(field)
        onEditingChanged?(true)
    }

    // MARK: - Editing

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard commandSelector == #selector(NSResponder.cancelOperation(_:)) else { return false }
        cancelsEdit = true
        window?.makeFirstResponder(tableView)
        return true
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        guard let field = notification.object as? NSTextField, let row = editingRow else { return }
        let movement = (notification.userInfo?["NSTextMovement"] as? Int).flatMap(NSTextMovement.init(rawValue:))
        let text = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let original = editingOriginal
        let cancelled = cancelsEdit
        editingRow = nil
        editingOriginal = nil
        cancelsEdit = false
        hasNewRow = false
        // Rows that changed meanwhile may have moved the edited row; then it is found by its text.
        var rowMoved = false
        if let pending {
            rowMoved = pending.rows != rows
            rows = pending.rows
            problemRows = pending.problemRows
            messages = pending.messages
            self.pending = nil
        }

        var proposed: [String]?
        if !cancelled, !text.isEmpty, text != original {
            var updated = rows
            if let original {
                if !rowMoved, row < updated.count, updated[row] == original {
                    updated[row] = text
                } else if let index = updated.firstIndex(of: original) {
                    updated[index] = text
                } else {
                    updated.append(text)
                }
            } else {
                updated.append(text)
            }
            proposed = updated
        }
        if let proposed {
            rows = proposed
            problemRows = []
            messages = [:]
        }
        reload()
        // Return keeps the keyboard in the list; a click elsewhere keeps its own focus.
        if movement == .return { window?.makeFirstResponder(tableView) }
        if let proposed { onChange?(proposed) }
        onEditingChanged?(false)
    }
}

/// Delete removes and Return edits the selected rows.
private final class RulesTableView: NSTableView {
    var onDelete: (() -> Void)?
    var onReturn: (() -> Void)?

    override func keyDown(with event: NSEvent) {
        switch event.specialKey {
        case .delete?, .deleteForward?: onDelete?()
        case .carriageReturn?, .enter?: onReturn?()
        default: super.keyDown(with: event)
        }
    }
}

/// A label over the empty table that lets clicks through to it.
private final class PassthroughLabel: NSTextField {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// A row of a list, every other one shaded on the form's background, as the settings mockup's list.
private final class AddressRowView: NSTableRowView {
    var isShaded = false

    override func drawBackground(in dirtyRect: NSRect) {
        guard isShaded else { return }
        NSColor.labelColor.withAlphaComponent(0.04).setFill()
        bounds.fill()
    }
}

/// A row's address, and for a problem row a red mark with what is wrong beside it, so the colour is
/// not the only sign.
private final class AddressCellView: NSTableCellView {
    let field = NSTextField()
    private let mark = NSImageView(image: NSImage(
        systemSymbolName: "exclamationmark.circle.fill", accessibilityDescription: String(localized: "Problem")
    ) ?? NSImage())
    private let message = NSTextField(labelWithString: "")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        field.isBordered = false
        field.drawsBackground = false
        field.font = .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        field.lineBreakMode = .byTruncatingTail
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        field.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        textField = field
        mark.contentTintColor = .systemRed
        mark.setContentHuggingPriority(.required, for: .horizontal)
        message.font = .systemFont(ofSize: NSFont.smallSystemFontSize + 1)
        message.textColor = .systemRed
        message.lineBreakMode = .byTruncatingTail
        message.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let problem = NSStackView(views: [mark, message])
        problem.spacing = 4
        for view in [field, problem] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: AddressListEditorView.textInset - 2),
            field.centerYAnchor.constraint(equalTo: centerYAnchor),
            problem.leadingAnchor.constraint(greaterThanOrEqualTo: field.trailingAnchor, constant: 8),
            problem.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -AddressListEditorView.textInset),
            problem.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used")
    }

    func showProblem(_ isProblem: Bool, message problemMessage: AddressListEditor.RowMessage?) {
        mark.isHidden = !isProblem
        message.isHidden = !isProblem || problemMessage == nil
        message.stringValue = problemMessage?.text ?? ""
        let help = isProblem ? problemMessage?.help : nil
        mark.toolTip = help
        message.toolTip = help
    }
}
