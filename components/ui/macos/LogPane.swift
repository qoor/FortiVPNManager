import AppKit
import SwiftUI

/// The Log pane: the VPN client's raw output and the explanation lines of the app and the Helper,
/// in a box like the other panes' groups. Its actions sit in the toolbar (and the Edit and File menus).
struct LogPane: View {
    let ui: AppUI
    let navigation: SettingsNavigation
    let logView: LogTextView.Coordinator

    @AppStorage("logAutoScroll") private var autoScroll = true
    @Environment(\.timeZone) private var timeZone
    @State private var findRequest = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Toggle("Auto-scroll", isOn: $autoScroll)
                .toggleStyle(.checkbox)
                .searchAnchor("Auto-scroll", navigation)
            LogTextView(
                log: ui.log,
                lastID: ui.log.entries.last?.id,
                clearCount: ui.log.clearCount,
                autoScroll: autoScroll,
                findRequest: findRequest,
                coordinator: logView
            )
            .padding(4)
            .background(.fill.quaternary, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
        .padding(20)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button("Find", systemImage: "magnifyingglass") { findRequest += 1 }
                    .help("Find in log (⌘F)")
                Button("Clear", systemImage: "trash") { ui.log.clear() }
                    .help("Clear the log view (⌘K) — the log file is kept")
                Button("Copy All", systemImage: "doc.on.doc") { ui.copyEntireLog(timeZone: timeZone) }
                    .help("Copy Entire Log")
                Button("Show in Finder", systemImage: "folder") {
                    if let logFile = ui.config?.logFile { revealInFinder(logFile) }
                }
                .disabled(ui.config == nil)
                .help("Show the log file in Finder")
            }
        }
    }
}

/// Read-only, selectable TextKit 1 text view with a find bar. Appends `LogLines` entries
/// incrementally and drops the ones trimmed there, keeping the visible text in place.
struct LogTextView: NSViewRepresentable {
    let log: LogLines
    /// Newest entry and clear generation: passed so that SwiftUI calls `updateNSView` when they change.
    let lastID: UInt64?
    let clearCount: Int
    let autoScroll: Bool
    /// Counts the toolbar's Find clicks; each opens the find bar.
    let findRequest: Int
    /// Kept by the settings window with the text view it made, which shows again when the pane does.
    let coordinator: Coordinator

    func makeCoordinator() -> Coordinator { coordinator }

    func makeNSView(context: Context) -> NSScrollView {
        // The count starts again with the pane.
        context.coordinator.findRequest = findRequest
        if let scrollView = context.coordinator.scrollView { return scrollView }
        let textView = NSTextView(usingTextLayoutManager: false)
        textView.isEditable = false
        textView.isSelectable = true
        textView.usesFindBar = true
        textView.isIncrementalSearchingEnabled = true
        textView.font = Coordinator.font
        textView.textContainerInset = NSSize(width: 6, height: 6)
        // On the pane's box, as the settings mockup's log.
        textView.drawsBackground = false
        textView.minSize = .zero
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: .greatestFiniteMagnitude)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true

        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.drawsBackground = false
        scrollView.documentView = textView
        context.coordinator.scrollView = scrollView
        context.coordinator.textView = textView
        scrollView.postsFrameChangedNotifications = true
        NotificationCenter.default.addObserver(
            context.coordinator, selector: #selector(Coordinator.scrollViewResized(_:)),
            name: NSView.frameDidChangeNotification, object: scrollView
        )
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.sync(with: log, clearCount: clearCount, autoScroll: autoScroll)
        context.coordinator.find(request: findRequest)
    }

    @MainActor
    final class Coordinator: NSObject {
        static let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        private static let markerFont = NSFont.monospacedSystemFont(ofSize: 10, weight: .regular)
        private static let timeFormatter: DateFormatter = {
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.dateFormat = "HH:mm:ss"
            return formatter
        }()

        /// Lines trimmed from `LogLines` leave the view in chunks: removing text at the top re-lays out
        /// everything below it, while appending only lays out the new lines.
        private static let trimChunk = 500

        /// Held here, as the view outlives the pane that shows it.
        var scrollView: NSScrollView?
        weak var textView: NSTextView?

        /// Entries currently shown, oldest first, with their length in UTF-16 units.
        private var shown: [(id: UInt64, length: Int)] = []
        private var clearCount: Int?
        private var autoScroll = false
        fileprivate var findRequest = 0

        /// Opens the find bar, as Edit > Find does, with the keyboard in the log.
        func find(request: Int) {
            guard request != findRequest else { return }
            findRequest = request
            DispatchQueue.main.async { [weak self] in
                guard let textView = self?.textView, let window = textView.window else { return }
                window.makeFirstResponder(textView)
                let item = NSMenuItem()
                item.tag = NSTextFinder.Action.showFindInterface.rawValue
                textView.performTextFinderAction(item)
            }
        }

        /// The pane's view is first laid out after its lines are in, so the scroll to the end made
        /// then, on a view without a size, has no effect; it is made again once the view has one.
        @objc func scrollViewResized(_ notification: Notification) {
            if autoScroll { textView?.scrollToEndOfDocument(nil) }
        }

        func sync(with log: LogLines, clearCount: Int, autoScroll: Bool) {
            guard let textView, let storage = textView.textStorage, let scrollView = textView.enclosingScrollView else {
                return
            }
            let autoScrollTurnedOn = autoScroll && !self.autoScroll
            self.autoScroll = autoScroll
            if clearCount != self.clearCount {
                self.clearCount = clearCount
                storage.setAttributedString(NSAttributedString())
                shown.removeAll()
            }

            let wasAtBottom = Self.isAtBottom(scrollView)
            let added = log.entries(after: shown.last?.id)
            let stale = log.entries.first.map { first in shown.prefix { $0.id < first.id }.count } ?? 0
            let staleCount = stale >= Self.trimChunk ? stale : 0
            // Keep the visible lines in place when lines above them go away.
            let removedHeight = staleCount > 0 && !(autoScroll && wasAtBottom) ? height(ofFirst: staleCount) : 0

            if staleCount > 0 || !added.isEmpty {
                storage.beginEditing()
                if staleCount > 0 {
                    let length = shown.prefix(staleCount).reduce(0) { $0 + $1.length }
                    storage.deleteCharacters(in: NSRange(location: 0, length: length))
                    shown.removeFirst(staleCount)
                }
                for entry in added {
                    let text = Self.render(entry.line)
                    storage.append(text)
                    shown.append((entry.id, text.length))
                }
                storage.endEditing()
            }

            if autoScroll, wasAtBottom || autoScrollTurnedOn {
                textView.scrollToEndOfDocument(nil)
            } else if removedHeight > 0 {
                let clipView = scrollView.contentView
                clipView.scroll(to: NSPoint(x: clipView.bounds.minX, y: max(0, clipView.bounds.minY - removedHeight)))
                scrollView.reflectScrolledClipView(clipView)
            }
        }

        /// Height of the first `count` shown lines, measured before they are deleted.
        private func height(ofFirst count: Int) -> CGFloat {
            guard let textView, let layoutManager = textView.layoutManager else { return 0 }
            let length = shown.prefix(count).reduce(0) { $0 + $1.length }
            let glyphs = layoutManager.glyphRange(forCharacterRange: NSRange(location: 0, length: length), actualCharacterRange: nil)
            guard glyphs.length > 0 else { return 0 }
            return layoutManager.lineFragmentRect(forGlyphAt: NSMaxRange(glyphs) - 1, effectiveRange: nil).maxY
        }

        private static func isAtBottom(_ scrollView: NSScrollView) -> Bool {
            guard let documentView = scrollView.documentView else { return true }
            return scrollView.contentView.bounds.maxY >= documentView.frame.height - 8
        }

        /// `HH:mm:ss [err|▸] text`. Colour follows the openfortivpn prefix, not the stream:
        /// stderr is mostly ordinary pppd chatter, so it only gets a small "err" marker.
        private static func render(_ line: AppInternal.LogLine) -> NSAttributedString {
            let result = NSMutableAttributedString(
                string: timeFormatter.string(from: line.time) + " ",
                attributes: [.font: font, .foregroundColor: NSColor.tertiaryLabelColor]
            )
            let textColor: NSColor
            switch line.source {
            case .note:
                result.append(NSAttributedString(string: "▸ ", attributes: [.font: font, .foregroundColor: NSColor.secondaryLabelColor]))
                textColor = .secondaryLabelColor
            case .stderr:
                result.append(NSAttributedString(string: "err ", attributes: [.font: markerFont, .foregroundColor: NSColor.tertiaryLabelColor]))
                textColor = color(forPrefixOf: line.text)
            case .stdout:
                textColor = color(forPrefixOf: line.text)
            }
            result.append(NSAttributedString(string: line.text + "\n", attributes: [.font: font, .foregroundColor: textColor]))
            return result
        }

        private static func color(forPrefixOf text: String) -> NSColor {
            if text.hasPrefix("ERROR:") { return .systemRed }
            if text.hasPrefix("WARN:") { return .systemOrange }
            return .labelColor
        }
    }
}
