import Foundation
import Observation

/// Where the settings window is: the pane, the panes visited before and after it, and the settings
/// search with the result it went to. Shared by the window's views, toolbar and menus.
@MainActor
@Observable
final class SettingsNavigation {
    /// Reopens on the last pane shown, as System Settings does.
    private(set) var pane = UserDefaults.standard.string(forKey: lastPaneKey).flatMap(SettingsPane.init(rawValue:)) ?? .general {
        willSet {
            onPaneWillChange?()
        }
        didSet {
            UserDefaults.standard.set(pane.rawValue, forKey: Self.lastPaneKey)
            onPaneChange?(pane)
        }
    }
    /// The window's title follows the pane.
    @ObservationIgnored var onPaneChange: ((SettingsPane) -> Void)?
    /// Ends the typing in the pane being left, so it is kept as when the field loses focus: its views
    /// go with the pane.
    @ObservationIgnored var onPaneWillChange: (() -> Void)?
    /// Whether the rows of settings shown only in some states are shown now; the search lists only
    /// the settings it can go to.
    @ObservationIgnored var isShown: (SettingsSearch.Condition) -> Bool = { _ in true }
    private(set) var backHistory: [SettingsPane] = []
    private(set) var forwardHistory: [SettingsPane] = []

    var searchText = ""
    /// The result last chosen in the sidebar; the sidebar marks it while it is listed.
    private(set) var chosenResult: SettingsSearch.Item?
    /// The row a chosen result scrolls to and briefly highlights.
    private(set) var highlightedRow: String?
    /// Counts the results chosen, so choosing the same one again scrolls again.
    private(set) var highlightRequest = 0
    /// Counts the requests for the search field to take the keyboard.
    private(set) var searchFocusRequest = 0

    /// Disclosure groups of the Account pane, which a search result opens.
    var showsRealm = false
    var showsIMAPFallback = false
    var showsOAuthClient = false
    /// A row of an exception list is being typed: its text reaches the runtime when the edit ends,
    /// which Apply and Discard Changes bring about first, so they take clicks meanwhile.
    var isEditingRuleRow = false

    private static let lastPaneKey = "settingsPane"

    // MARK: - Panes

    /// Goes to `pane`, after which Back returns to the one shown now.
    func select(_ pane: SettingsPane) {
        chosenResult = nil
        // Picking a pane while searching opens the groups its results are in.
        results(in: pane).forEach(reveal(groupOf:))
        guard pane != self.pane else { return }
        backHistory.append(self.pane)
        forwardHistory.removeAll()
        self.pane = pane
    }

    func goBack() {
        guard let previous = backHistory.popLast() else { return }
        chosenResult = nil
        forwardHistory.append(pane)
        pane = previous
    }

    func goForward() {
        guard let next = forwardHistory.popLast() else { return }
        chosenResult = nil
        backHistory.append(pane)
        pane = next
    }

    // MARK: - Search

    var searchTerms: [String] {
        searchText.split(whereSeparator: \.isWhitespace).map(String.init)
    }

    /// The panes listed in the sidebar: all of them, or those with a result.
    var visiblePanes: [SettingsPane] {
        guard !searchTerms.isEmpty else { return SettingsPane.allCases }
        return SettingsPane.allCases.filter { titleMatches($0) || !results(in: $0).isEmpty }
    }

    /// The settings of `pane` that match the search. A pane whose own name matches is the result
    /// itself, so it lists none.
    func results(in pane: SettingsPane) -> [SettingsSearch.Item] {
        let terms = searchTerms.map(SettingsSearchMatch.Term.init)
        guard !terms.isEmpty, !Self.matches(terms, SettingsSearch.name(of: pane)) else { return [] }
        return SettingsSearch.items(in: pane).filter { item in
            (item.condition.map(isShown) ?? true) && Self.matches(terms, item.name)
        }
    }

    /// Goes to the result's pane, opens the group it is in, then scrolls to its row and highlights it.
    func reveal(_ item: SettingsSearch.Item) {
        select(item.pane)
        chosenResult = item
        reveal(groupOf: item)
        highlightedRow = item.row
        highlightRequest += 1
        let request = highlightRequest
        Task {
            try? await Task.sleep(for: .seconds(2))
            if highlightRequest == request { highlightedRow = nil }
        }
    }

    /// Return in the search field, and Down Arrow before it moves into the list.
    func revealFirstResult() {
        guard !searchTerms.isEmpty, let pane = visiblePanes.first else { return }
        if let item = results(in: pane).first {
            reveal(item)
        } else {
            select(pane)
        }
    }

    func focusSearch() {
        searchFocusRequest += 1
    }

    private func titleMatches(_ pane: SettingsPane) -> Bool {
        Self.matches(searchTerms.map(SettingsSearchMatch.Term.init), SettingsSearch.name(of: pane))
    }

    /// Every term must appear in one of the names, in the app's language or in English.
    private static func matches(_ terms: [SettingsSearchMatch.Term], _ name: SettingsSearchMatch.Name) -> Bool {
        terms.allSatisfy { SettingsSearchMatch.matches($0, in: name) }
    }

    private func reveal(groupOf item: SettingsSearch.Item) {
        switch item.group {
        case .realm?: showsRealm = true
        case .imapFallback?: showsIMAPFallback = true
        case .oauthClient?: showsOAuthClient = true
        case nil: break
        }
    }
}

/// The settings the sidebar search finds, by pane.
enum SettingsSearch {
    struct Item: Hashable, Sendable {
        let pane: SettingsPane
        /// The English label, which is also its localization key.
        let key: String
        /// The label in the app's language.
        let title: String
        /// The row the result scrolls to: its own, or the one it shares. Nil for a toolbar action.
        let row: String?
        /// The collapsed group the setting is in.
        let group: Group?
        /// The state its row is shown in, if only in some.
        let condition: Condition?
        /// The pane's names and the setting's, as the search matches them.
        let name: SettingsSearchMatch.Name
    }

    enum Group: Sendable {
        case realm
        case imapFallback
        case oauthClient
    }

    enum Condition: Sendable {
        /// The VPN is connected (the General pane's connection details).
        case connected
        /// The OTP is read with the Gmail API (the Account pane's Google rows).
        case gmailApi
        /// The VPN client was found (the Advanced pane's client rows).
        case vpnClientFound
    }

    /// The settings of `pane`, built once: the search goes through them on every keystroke.
    static func items(in pane: SettingsPane) -> [Item] {
        index[pane] ?? []
    }

    /// The pane's name in the app's language and in English, as the search matches it.
    static func name(of pane: SettingsPane) -> SettingsSearchMatch.Name {
        paneNames[pane] ?? SettingsSearchMatch.Name("")
    }

    private static let index = Dictionary(uniqueKeysWithValues: SettingsPane.allCases.map { ($0, makeItems(in: $0)) })
    private static let paneNames = Dictionary(
        uniqueKeysWithValues: SettingsPane.allCases.map { ($0, SettingsSearchMatch.Name("\($0.title) \($0.englishTitle)")) }
    )

    private static func makeItems(in pane: SettingsPane) -> [Item] {
        switch pane {
        case .general:
            [
                item(pane, "Current status"),
                item(pane, "Connected at", when: .connected),
                item(pane, "Tunnel IP", when: .connected),
                item(pane, "Next proactive reconnect", when: .connected),
                item(pane, "Connect", row: connectionButtons),
                item(pane, "Disconnect", row: connectionButtons),
                item(pane, "Reconnect Now", row: connectionButtons),
                item(pane, "Open at login"),
                item(pane, "Connect automatically at launch"),
                item(pane, "Import openfortivpn Settings…", row: importRow),
            ]
        case .account:
            [
                item(pane, "Server address"),
                item(pane, "Port"),
                item(pane, "Username"),
                item(pane, "Password"),
                item(pane, "Realm", group: .realm),
                item(pane, "Gmail address"),
                item(pane, "Connection method"),
                item(pane, "Google account", when: .gmailApi),
                item(pane, "IMAP fallback (optional)", group: .imapFallback, when: .gmailApi),
                item(pane, "App password", group: .imapFallback),
                item(pane, "OAuth client", group: .oauthClient, when: .gmailApi),
                item(pane, "Import JSON…", row: oauthClientButtons, group: .oauthClient, when: .gmailApi),
                item(pane, "Use Built-in Client", row: oauthClientButtons, group: .oauthClient, when: .gmailApi),
                item(pane, "Test Gmail Connection"),
            ]
        case .connection:
            [
                item(pane, "Connect on a schedule"),
                item(pane, "Days"),
                item(pane, "Start time"),
                item(pane, "End time"),
                item(pane, "Stop reconnecting after being away"),
                item(pane, "Maximum session length"),
                item(pane, "Proactive reconnect"),
                item(pane, "Margin"),
                item(pane, "Reconnect after sleep"),
                item(pane, "Minimum sleep time"),
            ]
        case .exceptionRules:
            [
                item(pane, "Current network"),
                item(pane, "Networks where the rules apply"),
                item(pane, "Add Current Network"),
                item(pane, "Exception targets"),
                item(pane, "Status"),
            ]
        case .advanced:
            [
                item(pane, "Helper"),
                item(pane, "Register Helper"),
                item(pane, "App Background Activity"),
                item(pane, "Open System Settings…", row: "App Background Activity"),
                item(pane, "openfortivpn", when: .vpnClientFound),
                item(pane, "Version", when: .vpnClientFound),
                item(pane, "Settings File"),
                item(pane, "Show in Finder", row: "Settings File"),
            ]
        case .log:
            [
                item(pane, "Auto-scroll"),
                item(pane, "Find", toolbar: true),
                item(pane, "Clear", toolbar: true),
                item(pane, "Copy All", toolbar: true),
                item(pane, "Show in Finder", toolbar: true),
            ]
        }
    }

    /// Rows that hold more than one searchable setting.
    static let connectionButtons = "Connect"
    static let importRow = "Import openfortivpn Settings…"
    static let oauthClientButtons = "Import JSON…"

    /// A setting labelled `label`, in its own row unless it shares `row` or is a toolbar action.
    private static func item(
        _ pane: SettingsPane, _ label: LocalizedStringResource, row: String? = nil, group: Group? = nil,
        when condition: Condition? = nil, toolbar: Bool = false
    ) -> Item {
        let title = String(localized: label)
        return Item(
            pane: pane, key: label.key, title: title, row: toolbar ? nil : row ?? label.key, group: group,
            condition: condition, name: SettingsSearchMatch.Name("\(pane.title) \(pane.englishTitle) \(title) \(label.key)")
        )
    }
}

/// Settings search matching, measured against macOS 27 System Settings in Korean (both typed with the
/// 2-Set Korean input method and set directly):
/// - Characters before the last one match as whole syllables, anywhere in a name (터ㄹ matches 배터리).
/// - The last character may still be composing, so it matches at the jamo level: a lone consonant
///   matches a syllable starting with it (배ㅌ, ㄷ), and a syllable matches one it begins
///   (디스프 → 디스플레이).
/// - A one-syllable term's final consonant may become the next syllable's first one, at the start of a
///   word only (뱉 → 배터리, 렝 → 레이아웃, but 털 and 배털 don't match).
/// - Earlier jamo (ㄷ스플레이), initial-consonant abbreviations (ㅂㅌㄹ), and Hangul spellings of
///   English words (블루투스) don't match. Latin text matches case-insensitively.
enum SettingsSearchMatch {
    /// A name prepared once for matching: normalized, with each character's jamo.
    struct Name: Hashable, Sendable {
        fileprivate let characters: [Character]
        fileprivate let jamo: [String]

        init(_ text: String) {
            characters = Array(text.precomposedStringWithCanonicalMapping.lowercased())
            jamo = characters.map(SettingsSearchMatch.jamo)
        }
    }

    /// A term prepared once per search: normalized, with the jamo of its last character.
    struct Term {
        fileprivate let characters: [Character]
        fileprivate let lastJamo: String

        init(_ term: String) {
            characters = Array(term.precomposedStringWithCanonicalMapping.lowercased())
            lastJamo = characters.last.map(SettingsSearchMatch.jamo) ?? ""
        }
    }

    static func matches(_ term: String, in text: String) -> Bool {
        matches(Term(term), in: Name(text))
    }

    static func matches(_ prepared: Term, in name: Name) -> Bool {
        let term = prepared.characters, lastJamo = prepared.lastJamo
        let text = name.characters
        guard let last = term.last else { return true }
        let head = term.dropLast()
        let partial = isHangul(last)
        if text.count >= term.count {
            for start in 0...(text.count - term.count) where text[start..<start + head.count].elementsEqual(head) {
                let end = start + head.count
                if text[end] == last || (partial && name.jamo[end].hasPrefix(lastJamo)) { return true }
            }
        }
        // 뱉 while typing 배터리: the final consonant is the next syllable's first one.
        guard term.count == 1, isSyllable(last), finalIndex(last) != 0, text.count >= 2 else { return false }
        let base = String(lastJamo.dropLast()), next = String(lastJamo.last!)
        for start in 0..<(text.count - 1) where start == 0 || !(text[start - 1].isLetter || text[start - 1].isNumber) {
            if name.jamo[start] == base, name.jamo[start + 1].hasPrefix(next) { return true }
        }
        return false
    }

    private static let initials = Array("ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ").map(String.init)
    private static let medials = ["ㅏ", "ㅐ", "ㅑ", "ㅒ", "ㅓ", "ㅔ", "ㅕ", "ㅖ", "ㅗ", "ㅗㅏ", "ㅗㅐ", "ㅗㅣ", "ㅛ", "ㅜ", "ㅜㅓ", "ㅜㅔ", "ㅜㅣ", "ㅠ", "ㅡ", "ㅡㅣ", "ㅣ"]
    private static let finals = ["", "ㄱ", "ㄲ", "ㄱㅅ", "ㄴ", "ㄴㅈ", "ㄴㅎ", "ㄷ", "ㄹ", "ㄹㄱ", "ㄹㅁ", "ㄹㅂ", "ㄹㅅ", "ㄹㅌ", "ㄹㅍ", "ㄹㅎ", "ㅁ", "ㅂ", "ㅂㅅ", "ㅅ", "ㅆ", "ㅇ", "ㅈ", "ㅊ", "ㅋ", "ㅌ", "ㅍ", "ㅎ"]
    private static let compounds: [Character: String] = [
        "ㄳ": "ㄱㅅ", "ㄵ": "ㄴㅈ", "ㄶ": "ㄴㅎ", "ㄺ": "ㄹㄱ", "ㄻ": "ㄹㅁ", "ㄼ": "ㄹㅂ", "ㄽ": "ㄹㅅ", "ㄾ": "ㄹㅌ", "ㄿ": "ㄹㅍ", "ㅀ": "ㄹㅎ", "ㅄ": "ㅂㅅ",
        "ㅘ": "ㅗㅏ", "ㅙ": "ㅗㅐ", "ㅚ": "ㅗㅣ", "ㅝ": "ㅜㅓ", "ㅞ": "ㅜㅔ", "ㅟ": "ㅜㅣ", "ㅢ": "ㅡㅣ",
    ]

    private static func syllableIndex(_ character: Character) -> Int? {
        guard character.unicodeScalars.count == 1, let value = character.unicodeScalars.first?.value,
              (0xAC00...0xD7A3).contains(value) else { return nil }
        return Int(value - 0xAC00)
    }

    private static func isSyllable(_ character: Character) -> Bool { syllableIndex(character) != nil }

    private static func isHangul(_ character: Character) -> Bool {
        isSyllable(character) || character.unicodeScalars.allSatisfy { (0x3131...0x318E).contains($0.value) }
    }

    private static func finalIndex(_ character: Character) -> Int { (syllableIndex(character) ?? 0) % 28 }

    /// Compatibility jamo with compound vowels and finals split, so a composing syllable is a prefix of
    /// its result.
    fileprivate static func jamo(_ character: Character) -> String {
        guard let index = syllableIndex(character) else { return compounds[character] ?? String(character) }
        return initials[index / 588] + medials[index % 588 / 28] + finals[index % 28]
    }
}
