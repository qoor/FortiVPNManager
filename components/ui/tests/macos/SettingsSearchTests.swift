import Foundation
import Testing

/// The settings search: the Korean matching measured against macOS 27 System Settings, and what the
/// sidebar lists and opens. The test bundle has no Korean strings, so names are the English ones.
@MainActor
struct SettingsSearchTests {
    /// Each term against a name, as macOS 27 System Settings in Korean matched it (typed with the
    /// 2-Set Korean input method and set directly).
    @Test(arguments: [
        ("배터리", "배터리 성능 상태", true),
        ("뱉", "배터리 성능 상태", true),
        ("배털", "배터리 성능 상태", false),
        ("배텨", "배터리", false),
        ("배터ㄹ", "배터리", true),
        ("배ㅌ", "배터리", true),
        ("디스프", "디스플레이", true),
        ("디스플렝", "디스플레이", false),
        ("디슾", "디스플레이", false),
        ("디스플레ㅇ", "디스플레이", true),
        ("ㄷ", "디스플레이", true),
        ("ㄷ", "사운드", true),
        ("ㅂㅌㄹ", "배터리", false),
        ("ㄷ스플레이", "디스플레이", false),
        ("디ㅅ플레이", "디스플레이", false),
        ("털", "배터리", false),
        ("렝", "키보드 레이아웃", true),
        ("슾", "스피커", true),
        ("터ㄹ", "배터리", true),
        ("스플레", "디스플레이", true),
        ("배ㅌㅓ", "배터리", false),
        ("batt", "Battery", true),
        ("BATTERY", "배터리 Battery", true),
        ("디습", "디스플레이", false),
        ("블루투스", "Bluetooth", false),
        ("렝", "디스플레이", false),
    ])
    func matchesLikeSystemSettings(term: String, name: String, matches: Bool) {
        #expect(SettingsSearchMatch.matches(term, in: name) == matches)
    }

    @Test func listsTheMatchingSettingsUnderTheirPane() {
        let navigation = SettingsNavigation()
        navigation.searchText = "port"
        #expect(navigation.visiblePanes.contains(.account))
        #expect(navigation.results(in: .account).map(\.key).contains("Port"))
        navigation.searchText = "zzzz"
        #expect(navigation.visiblePanes.isEmpty)
        navigation.searchText = ""
        #expect(navigation.visiblePanes == SettingsPane.allCases)
    }

    @Test func aPaneMatchedByItsNameIsTheResultItself() {
        let navigation = SettingsNavigation()
        navigation.searchText = "exception rules"
        #expect(navigation.visiblePanes.contains(.exceptionRules))
        #expect(navigation.results(in: .exceptionRules).isEmpty)
    }

    @Test func listsOnlyTheSettingsThePanesShowNow() {
        let navigation = SettingsNavigation()
        navigation.isShown = { $0 != .gmailApi }
        navigation.searchText = "oauth"
        #expect(navigation.results(in: .account).isEmpty)
        #expect(!navigation.visiblePanes.contains(.account))
        navigation.isShown = { _ in true }
        #expect(navigation.results(in: .account).map(\.key).contains("OAuth client"))
    }

    @Test func everyTermMustMatch() {
        let navigation = SettingsNavigation()
        navigation.searchText = "gmail test"
        #expect(navigation.results(in: .account).map(\.key) == ["Test Gmail Connection"])
    }

    @Test func choosingAResultOpensItsGroupAndHighlightsItsRow() throws {
        let navigation = SettingsNavigation()
        navigation.select(.general)
        navigation.searchText = "realm"
        let realm = try #require(navigation.results(in: .account).first)
        navigation.reveal(realm)
        #expect(navigation.pane == .account)
        #expect(navigation.showsRealm)
        #expect(navigation.highlightedRow == "Realm")
        #expect(navigation.chosenResult == realm)
    }

    @Test func settingsSharingARowScrollToIt() throws {
        let navigation = SettingsNavigation()
        navigation.searchText = "reconnect now"
        let item = try #require(navigation.results(in: .general).first)
        navigation.reveal(item)
        #expect(navigation.highlightedRow == SettingsSearch.connectionButtons)
    }

    @Test func returnOpensTheFirstResult() {
        let navigation = SettingsNavigation()
        navigation.select(.general)
        navigation.searchText = "oauth"
        navigation.revealFirstResult()
        #expect(navigation.pane == .account)
        #expect(navigation.showsOAuthClient)
    }

    @Test func backAndForwardRetraceThePanes() {
        let navigation = SettingsNavigation()
        navigation.select(.general)
        navigation.select(.account)
        navigation.select(.log)
        navigation.goBack()
        #expect(navigation.pane == .account)
        navigation.goBack()
        #expect(navigation.pane == .general)
        navigation.goForward()
        #expect(navigation.pane == .account)
        // A new pane drops the panes ahead.
        navigation.select(.advanced)
        #expect(navigation.forwardHistory.isEmpty)
    }
}
