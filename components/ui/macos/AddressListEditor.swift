import AppKit
import SwiftUI

/// One list of the Exception Rules: a single column with + / − below. Every finished edit reports the
/// whole list through `onChange`; nothing is saved here. `onEditingChanged` tells whether a row is
/// being typed (its text reaches `onChange` only when the edit ends).
struct AddressListEditor: NSViewRepresentable {
    let rows: [String]
    let problemRows: Set<Int>
    /// The + button: off once the list holds as many entries as the settings file allows.
    let canAdd: Bool
    /// Shown over the empty list.
    let emptyText: String
    /// Shown in a row being typed.
    let placeholder: String
    /// Help tag of the + button.
    let addHelp: String
    let onChange: ([String]) -> Void
    let onEditingChanged: (Bool) -> Void

    func makeNSView(context: Context) -> AddressListEditorView {
        let view = AddressListEditorView()
        updateNSView(view, context: context)
        return view
    }

    func updateNSView(_ view: AddressListEditorView, context: Context) {
        view.onChange = onChange
        view.onEditingChanged = onEditingChanged
        view.configure(emptyText: emptyText, placeholder: placeholder, addHelp: addHelp)
        view.canAdd = canAdd
        view.show(rows: rows, problemRows: problemRows)
    }
}

final class AddressListEditorView: NSView, NSTableViewDataSource, NSTableViewDelegate, NSTextFieldDelegate {
    var onChange: (([String]) -> Void)?
    var onEditingChanged: ((Bool) -> Void)?
    var canAdd = true {
        didSet { if canAdd != oldValue { updateButtons() } }
    }

    private let scrollView = NSScrollView()
    private let tableView = RulesTableView()
    private let buttons = NSSegmentedControl()
    private let emptyLabel = PassthroughLabel(labelWithString: "")
    private var placeholder = ""

    /// The rows as last shown.
    private var rows: [String] = []
    private var problemRows: Set<Int> = []
    /// A row added with + or a double-click, shown after `rows` until its edit ends.
    private var hasNewRow = false
    /// Row being edited and its text before the edit (`nil` for the new row).
    private var editingRow: Int?
    private var editingOriginal: String?
    private var cancelsEdit = false
    /// Rows that arrived while an edit was in progress; shown when it ends.
    private var pending: (rows: [String], problemRows: Set<Int>)?

    private static let cellID = NSUserInterfaceItemIdentifier("addressRow")
    private static let addSegment = 0
    private static let removeSegment = 1

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setUp()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setUp()
    }

    private var displayCount: Int { rows.count + (hasNewRow ? 1 : 0) }

    func configure(emptyText: String, placeholder: String, addHelp: String) {
        if emptyLabel.stringValue != emptyText { emptyLabel.stringValue = emptyText }
        self.placeholder = placeholder
        buttons.setToolTip(addHelp, forSegment: Self.addSegment)
    }

    func show(rows newRows: [String], problemRows newProblems: Set<Int>) {
        if editingRow != nil {
            pending = (newRows, newProblems)
            return
        }
        guard newRows != rows || newProblems != problemRows else { return }
        rows = newRows
        problemRows = newProblems
        reload()
    }

    // MARK: - Layout

    private func setUp() {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("address"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.style = .fullWidth
        tableView.usesAlternatingRowBackgroundColors = true
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
        scrollView.borderType = .bezelBorder

        buttons.segmentStyle = .smallSquare
        buttons.trackingMode = .momentary
        buttons.segmentCount = 2
        buttons.setImage(NSImage(systemSymbolName: "plus", accessibilityDescription: String(localized: "Add")), forSegment: Self.addSegment)
        buttons.setImage(NSImage(systemSymbolName: "minus", accessibilityDescription: String(localized: "Remove")), forSegment: Self.removeSegment)
        buttons.setToolTip(String(localized: "Remove the selected rows"), forSegment: Self.removeSegment)
        for segment in 0..<2 { buttons.setWidth(26, forSegment: segment) }
        buttons.target = self
        buttons.action = #selector(buttonClicked)

        emptyLabel.textColor = .secondaryLabelColor
        emptyLabel.alignment = .center
        emptyLabel.lineBreakMode = .byWordWrapping

        for view in [scrollView, buttons, emptyLabel] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            buttons.topAnchor.constraint(equalTo: scrollView.bottomAnchor, constant: -1),
            buttons.leadingAnchor.constraint(equalTo: leadingAnchor),
            buttons.bottomAnchor.constraint(equalTo: bottomAnchor),
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
        updateButtons()
    }

    private func updateButtons() {
        buttons.setEnabled(canAdd, forSegment: Self.addSegment)
        buttons.setEnabled(!tableView.selectedRowIndexes.isEmpty, forSegment: Self.removeSegment)
    }

    // MARK: - Table

    func numberOfRows(in tableView: NSTableView) -> Int {
        displayCount
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = tableView.makeView(withIdentifier: Self.cellID, owner: self) as? NSTableCellView ?? makeCell()
        if let field = cell.textField {
            field.stringValue = row < rows.count ? rows[row] : ""
            field.placeholderString = placeholder
            field.textColor = problemRows.contains(row) ? .systemRed : .labelColor
            cell.imageView?.isHidden = !problemRows.contains(row)
            field.isEditable = false
            field.isSelectable = false
        }
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        updateButtons()
    }

    /// A problem row is marked by a warning icon at its end as well as by red text, so the colour is
    /// not the only sign.
    private func makeCell() -> NSTableCellView {
        let cell = NSTableCellView()
        cell.identifier = Self.cellID
        let field = NSTextField()
        field.isBordered = false
        field.drawsBackground = false
        field.font = .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        field.lineBreakMode = .byTruncatingTail
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        field.delegate = self
        field.translatesAutoresizingMaskIntoConstraints = false
        let icon = NSImageView(image: NSImage(
            systemSymbolName: "exclamationmark.triangle.fill", accessibilityDescription: String(localized: "Problem")
        ) ?? NSImage())
        icon.contentTintColor = .systemOrange
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.setContentHuggingPriority(.required, for: .horizontal)
        cell.addSubview(field)
        cell.addSubview(icon)
        cell.textField = field
        cell.imageView = icon
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 4),
            field.trailingAnchor.constraint(equalTo: icon.leadingAnchor, constant: -4),
            field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            icon.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -6),
            icon.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }

    // MARK: - Actions

    @objc private func buttonClicked() {
        switch buttons.selectedSegment {
        case Self.addSegment: addRow()
        case Self.removeSegment: removeSelected()
        default: break
        }
    }

    /// A double-click below the last row reports row -1.
    @objc private func doubleClicked() {
        let row = tableView.clickedRow
        if row < 0 {
            addRow()
        } else {
            beginEditing(row)
        }
    }

    private func addRow() {
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

    private func removeSelected() {
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
        reload()
        onChange?(remaining)
    }

    private func beginEditing(_ row: Int) {
        guard row < displayCount else { return }
        tableView.scrollRowToVisible(row)
        tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        guard let cell = tableView.view(atColumn: 0, row: row, makeIfNecessary: true) as? NSTableCellView,
              let field = cell.textField
        else { return }
        editingRow = row
        editingOriginal = row < rows.count ? rows[row] : nil
        field.isEditable = true
        field.isSelectable = true
        field.textColor = .labelColor
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
