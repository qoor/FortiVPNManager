import AppKit
import FortiShared
import SwiftUI

/// Tab 3, Log (DESIGN §5.6): raw openfortivpn / pppd output and the helper's meta lines.
struct LogTab: View {
    let logStore: LogStore

    @AppStorage("logAutoScroll") private var autoScroll = true

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Toggle("Auto-scroll", isOn: $autoScroll)
                    .toggleStyle(.checkbox)
                Spacer()
                Button("Clear") { logStore.clearView() }
                Button("Copy All") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(logStore.allText, forType: .string)
                }
                Button("Show in Finder") { revealInFinder(logStore.fileURL) }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            Divider()
            LogTextView(
                store: logStore,
                lastID: logStore.entries.last?.id,
                clearCount: logStore.clearCount,
                autoScroll: autoScroll
            )
        }
    }
}

/// Read-only, selectable TextKit 1 text view with a find bar. Appends `LogStore` entries
/// incrementally and drops the ones the store has trimmed, keeping the visible text in place.
struct LogTextView: NSViewRepresentable {
    let store: LogStore
    /// Newest entry and clear generation: passed so that SwiftUI calls `updateNSView` when they change.
    let lastID: UInt64?
    let clearCount: Int
    let autoScroll: Bool

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSScrollView {
        let textView = NSTextView(usingTextLayoutManager: false)
        textView.isEditable = false
        textView.isSelectable = true
        textView.usesFindBar = true
        textView.isIncrementalSearchingEnabled = true
        textView.font = Coordinator.font
        textView.textContainerInset = NSSize(width: 6, height: 6)
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
        scrollView.documentView = textView
        context.coordinator.textView = textView
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.sync(with: store, clearCount: clearCount, autoScroll: autoScroll)
    }

    final class Coordinator {
        static let font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        private static let markerFont = NSFont.monospacedSystemFont(ofSize: 9, weight: .regular)
        private static let timeFormatter: DateFormatter = {
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.dateFormat = "HH:mm:ss"
            return formatter
        }()

        /// Lines the store dropped leave the view in chunks: removing text at the top re-lays out
        /// everything below it, while appending only lays out the new lines.
        private static let trimChunk = 500

        weak var textView: NSTextView?

        /// Entries currently shown, oldest first, with their length in UTF-16 units.
        private var shown: [(id: UInt64, length: Int)] = []
        private var clearCount: Int?
        private var autoScroll = false

        func sync(with store: LogStore, clearCount: Int, autoScroll: Bool) {
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
            let added = store.entries(after: shown.last?.id)
            let stale = store.entries.first.map { first in shown.prefix { $0.id < first.id }.count } ?? 0
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
        private static func render(_ line: OutputLine) -> NSAttributedString {
            let result = NSMutableAttributedString(
                string: timeFormatter.string(from: line.date) + " ",
                attributes: [.font: font, .foregroundColor: NSColor.tertiaryLabelColor]
            )
            let textColor: NSColor
            switch line.source {
            case .helper:
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
