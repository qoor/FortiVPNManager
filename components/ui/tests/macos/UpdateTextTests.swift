import Foundation
import Testing

@MainActor
struct UpdateTextTests {
  @Test func failedInstallLines() throws {
    let sections: [AppInternal.AutoUpdateSection] = try AppFixtures.decoded(
      "auto_update_sections.json")
    #expect(
      sections.map(\.failedInstallMessage) == ["Couldn't install version 26.2.0."]
        + Array(repeating: nil, count: 11))
  }

  @Test func downloadLines() throws {
    let sections: [AppInternal.AutoUpdateSection] = try AppFixtures.decoded(
      "auto_update_sections.json")
    let lines = sections.compactMap { section -> String? in
      guard case .downloading(let received, let total, _, let remaining) = section.progress else {
        return nil
      }
      return AppInternal.AutoUpdateSection.downloadLine(
        received: received, total: total, remaining: remaining)
    }
    #expect(
      lines == [
        "0 KB of 21.7 MB — Calculating time remaining…",
        "4.2 MB of 21.7 MB — About 2 minutes remaining",
        "19.9 MB of 21.7 MB — About 5 seconds remaining",
      ])
  }

  @Test func checkFailures() throws {
    let failures: [AppInternal.UpdateCheckFailure] = try AppFixtures.decoded(
      "update_check_failures.json")
    #expect(
      failures.map(\.message) == [
        "Couldn't check for updates (HTTP 403).",
        "Couldn't check for updates: error sending request: dns error",
        "Couldn't check for updates (unexpected reply).",
        "Couldn't check for updates (installer missing).",
      ])
  }

  @Test func installFailures() throws {
    let failures: [AppInternal.InstallFailure] = try AppFixtures.decoded("install_failures.json")
    #expect(
      failures.map(\.message) == [
        "Couldn't download the update. Check the network connection.",
        "Couldn't prepare the install. Try installing again.",
        "Couldn't reach the Helper, so the update wasn't installed.",
        "Couldn't reach the Helper, so the update wasn't installed.",
        "Couldn't reach the Helper, so the update wasn't installed.",
        "Couldn't verify the update file. Try installing again.",
        "Couldn't verify the update file. Try installing again.",
        "The same or a newer version is already installed.",
        "Another update is already being installed.",
        "Couldn't verify the update file. Try installing again.",
        "Couldn't prepare the install. Try installing again.",
      ])
  }

  @Test func releaseNotesInTheAppLanguageOrElseEnglish() {
    let url = "https://github.com/qoor/FortiVPNManager/releases/tag/v26.2.0"
    func available(_ titles: [String: [String]]) -> AppInternal.UpdateStatus {
      .available(
        version: "26.2.0", releasePageUrl: url, installerBytes: 21_733_376,
        releaseNoteTitles: titles)
    }
    let both = available(["en": ["New"], "ko": ["새 기능"]])
    let english = available(["en": ["New"]])
    let none = available([:])
    #expect(both.releaseNoteTitles(in: "ko") == ["새 기능"])
    #expect(both.releaseNoteTitles(in: "en") == ["New"])
    #expect(english.releaseNoteTitles(in: "ko") == ["New"])
    #expect(none.releaseNoteTitles(in: "ko") == [])
    #expect(AppInternal.UpdateStatus.upToDate.releaseNoteTitles(in: "en") == [])
  }

  @Test func updateNotification() {
    let notification = AppInternal.UserNotification.updateAvailable(version: "26.2.0")
    #expect(notification.identifier == AppInternal.UserNotification.updateAvailableIdentifier)
    #expect(notification.title == "Software Update Available")
    #expect(notification.body == "Update to FortiVPNManager 26.2.0 to get the latest features.")
    #expect(notification.actionTitle == "Update Now")
  }
}
