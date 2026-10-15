import Foundation
import Testing
@testable import FortiCore

@Suite("OTP matcher (Gmail API)")
struct GmailOTPMatcherTests {
    static let promptedAt = Date(timeIntervalSince1970: 1_791_940_740) // 14-Oct-2026 10:19:00 +0900

    static func message(
        _ id: String,
        secondsAfterPrompt offset: TimeInterval? = 3.2,
        subject: String? = "AuthCode: 482913",
        labels: [String] = ["UNREAD", "INBOX"]
    ) -> GmailMessageMetadata {
        GmailMessageMetadata(
            id: id,
            labelIds: labels,
            internalDate: offset.map { promptedAt.addingTimeInterval($0) },
            headers: [GmailMessageMetadata.Header(name: "Date", value: "Wed, 14 Oct 2026 10:19:03 +0900")]
                + (subject.map { [GmailMessageMetadata.Header(name: "Subject", value: $0)] } ?? [])
        )
    }

    private func select(_ messages: [GmailMessageMetadata], notBefore: Date? = nil) -> GmailOTPSelection? {
        OTPMatcher.select(otpPromptedAt: Self.promptedAt, messages: messages, candidatesNotBefore: notBefore)
    }

    @Test func selectsAFreshMessage() {
        #expect(select([Self.message("19a01")]) == GmailOTPSelection(
            code: "482913", messageID: "19a01", internalDate: Self.promptedAt.addingTimeInterval(3.2), allCandidateIDs: ["19a01"]
        ))
    }

    @Test(arguments: [
        (-6.0, false), (-5.001, false), (-5.0, true), (-4.0, true), (0.0, true), (45.0, true),
    ])
    func dateWindowBoundary(offset: TimeInterval, included: Bool) {
        #expect((select([Self.message("m", secondsAfterPrompt: offset)])?.messageID == "m") == included)
    }

    @Test func newestByInternalDateWinsAndEveryCandidateIsReported() {
        let messages = [
            Self.message("19a05", secondsAfterPrompt: 2, subject: "AuthCode: 222222"),
            Self.message("19a03", secondsAfterPrompt: 6, subject: "AuthCode: 333333"),    // newest by date
            Self.message("19a09", secondsAfterPrompt: -60, subject: "AuthCode: 999999"),  // stale, still trashed
            Self.message("19a07", subject: "Lunch on Friday?"),
            Self.message("19a08", subject: nil),
        ]
        #expect(select(messages) == GmailOTPSelection(
            code: "333333", messageID: "19a03", internalDate: Self.promptedAt.addingTimeInterval(6),
            allCandidateIDs: ["19a09", "19a05", "19a03"]
        ))
    }

    @Test func sameInstantPrefersTheLaterID() {
        let messages = [
            Self.message("19a0f", secondsAfterPrompt: 4, subject: "AuthCode: 111111"),
            Self.message("19a100", secondsAfterPrompt: 4, subject: "AuthCode: 222222"),
            Self.message("19a0e", secondsAfterPrompt: 4, subject: "AuthCode: 333333"),
        ]
        let selection = select(messages)
        #expect(selection?.code == "222222")
        #expect(selection?.allCandidateIDs == ["19a0e", "19a0f", "19a100"])
    }

    @Test(arguments: [["SPAM"], ["TRASH", "INBOX"], ["DRAFT"]])
    func excludedLabels(labels: [String]) {
        #expect(select([Self.message("m", labels: labels)]) == nil)
        let selection = select([Self.message("m", labels: labels), Self.message("n", subject: "AuthCode: 654321")])
        #expect(selection?.code == "654321")
        #expect(selection?.allCandidateIDs == ["n"])
    }

    @Test func noLabelsStillCount() {
        #expect(select([Self.message("m", labels: [])])?.messageID == "m")
    }

    @Test func missingInternalDateIsACandidateButNeverSelected() {
        #expect(select([Self.message("m", secondsAfterPrompt: nil)]) == nil)
        let selection = select([Self.message("m", secondsAfterPrompt: nil), Self.message("n")])
        #expect(selection?.messageID == "n")
        #expect(selection?.allCandidateIDs == ["m", "n"])
    }

    @Test func candidatesNotBeforeLimitsTrashingButNotSelection() {
        let notBefore = Self.promptedAt.addingTimeInterval(-30)
        let messages = [
            Self.message("old", secondsAfterPrompt: -3600, subject: "AuthCode: 111111"),
            Self.message("nodate", secondsAfterPrompt: nil, subject: "AuthCode: 222222"),
            Self.message("recent", secondsAfterPrompt: -20, subject: "AuthCode: 333333"),
            Self.message("fresh", secondsAfterPrompt: 1, subject: "AuthCode: 444444"),
        ]
        #expect(select(messages, notBefore: notBefore) == GmailOTPSelection(
            code: "444444", messageID: "fresh", internalDate: Self.promptedAt.addingTimeInterval(1),
            allCandidateIDs: ["recent", "fresh"]
        ))
        #expect(select(messages)?.allCandidateIDs == ["nodate", "old", "recent", "fresh"])
    }

    @Test func onlyStaleOrUnrelatedMessagesYieldNil() {
        #expect(select([]) == nil)
        #expect(select([Self.message("m", secondsAfterPrompt: -30)]) == nil)
        #expect(select([Self.message("m", subject: "AuthCode: 12345")]) == nil)
        #expect(select([Self.message("m", subject: "Your code is 123456")]) == nil)
    }

    @Test(arguments: [
        "=?UTF-8?B?7J247Kad7L2U65OcIEF1dGhDb2RlOiA0ODI5MTM=?=",
        "=?UTF-8?Q?AuthCode:_482913?=",
        "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 482913",
        "[External] AuthCode: 482913 (valid 60 s)",
    ])
    func subjectForms(subject: String) {
        #expect(select([Self.message("m", subject: subject)])?.code == "482913")
    }

    @Test func junkCheckIgnoresLabelsButNotTheWindow() {
        #expect(OTPMatcher.containsFreshAuthCode(otpPromptedAt: Self.promptedAt, messages: [Self.message("s", labels: ["SPAM"])]))
        #expect(!OTPMatcher.containsFreshAuthCode(
            otpPromptedAt: Self.promptedAt, messages: [Self.message("s", secondsAfterPrompt: -6, labels: ["SPAM"])]
        ))
        #expect(!OTPMatcher.containsFreshAuthCode(
            otpPromptedAt: Self.promptedAt, messages: [Self.message("s", secondsAfterPrompt: nil, labels: ["SPAM"])]
        ))
        #expect(!OTPMatcher.containsFreshAuthCode(
            otpPromptedAt: Self.promptedAt, messages: [Self.message("s", subject: "Hi", labels: ["SPAM"])]
        ))
    }
}
