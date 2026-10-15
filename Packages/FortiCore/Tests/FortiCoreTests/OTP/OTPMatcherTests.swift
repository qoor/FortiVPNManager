import Foundation
import Testing
@testable import FortiCore

@Suite("OTP matcher")
struct OTPMatcherTests {
    static let promptedAt = Date(timeIntervalSince1970: 1_791_940_740) // 14-Oct-2026 10:19:00 +0900
    static let snapshot = MailboxSnapshot(uidValidity: 11, uidNext: 500)

    static func message(
        uid: UInt32,
        secondsAfterPrompt offset: TimeInterval = 3,
        subject: String = "AuthCode: 482913"
    ) -> FetchedHeaderMessage {
        FetchedHeaderMessage(
            uid: uid,
            internalDate: promptedAt.addingTimeInterval(offset),
            headerBlock: Data("Subject: \(subject)\r\nDate: Wed, 14 Oct 2026 10:19:03 +0900\r\n\r\n".utf8)
        )
    }

    private func select(
        _ messages: [FetchedHeaderMessage],
        uidValidity: UInt32 = 11
    ) -> OTPSelection? {
        OTPMatcher.select(snapshot: Self.snapshot, currentUIDValidity: uidValidity, otpPromptedAt: Self.promptedAt, messages: messages)
    }

    // MARK: Subject pattern

    @Test(arguments: [
        ("AuthCode: 123456", "123456"),
        ("AuthCode:123456", "123456"),
        ("AuthCode:   12345678", "12345678"),
        ("AuthCode:\t1234567 (valid for 60 seconds)", "1234567"),
        ("AuthCode:\u{00A0}654321", "654321"),
        ("Fwd: AuthCode: 654321", "654321"),
        ("[SPAM] \u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 000001.", "000001"),
        ("AuthCode: abc / AuthCode: 111222", "111222"),
    ])
    func codeIsExtracted(_ subject: String, _ code: String) {
        #expect(OTPMatcher.code(inSubject: subject) == code)
    }

    @Test(arguments: [
        "AuthCode: 12345",          // too short
        "AuthCode: 123456789",      // too long: must not truncate to 8 digits
        "authcode: 123456",         // marker is case-sensitive
        "AuthCode 123456",          // colon required
        "AuthCode: １２３４５６",     // full-width digits are not ASCII
        "AuthCode: 12 3456",
        "Your code is 123456",
        "",
    ])
    func nonMatchingSubjects(_ subject: String) {
        #expect(OTPMatcher.code(inSubject: subject) == nil)
    }

    // MARK: select

    @Test func selectsFreshMessageAtOrAfterSnapshot() {
        #expect(select([Self.message(uid: 500)]) == OTPSelection(code: "482913", uid: 500, allCandidateUIDs: [500]))
    }

    @Test func uidBelowSnapshotIsExcluded() {
        // `UID SEARCH UID 500:*` returns the newest message even when its UID is 499.
        #expect(select([Self.message(uid: 499)]) == nil)
        #expect(select([Self.message(uid: 499), Self.message(uid: 500, subject: "AuthCode: 111111")])
            == OTPSelection(code: "111111", uid: 500, allCandidateUIDs: [500]))
    }

    @Test(arguments: [
        (-6.0, false),
        (-5.5, false),
        (-5.0, true),
        (-4.0, true),
        (0.0, true),
        (40.0, true),
    ])
    func dateWindowBoundary(offset: TimeInterval, included: Bool) {
        let result = select([Self.message(uid: 501, secondsAfterPrompt: offset)])
        #expect((result?.uid == 501) == included)
    }

    @Test func uidValidityChangeYieldsNil() {
        #expect(select([Self.message(uid: 501)], uidValidity: 12) == nil)
    }

    @Test func highestQualifyingUIDWinsAndAllCandidatesAreReported() {
        let messages = [
            Self.message(uid: 503, subject: "AuthCode: 333333"),
            Self.message(uid: 499, subject: "AuthCode: 999999"),                         // below snapshot
            Self.message(uid: 505, secondsAfterPrompt: -60, subject: "AuthCode: 555555"), // stale, still trashed
            Self.message(uid: 504, subject: "Lunch on Friday?"),                         // not an OTP
            Self.message(uid: 502, subject: "AuthCode: 222222"),
            Self.message(uid: 503, subject: "AuthCode: 333333"),                         // duplicate fetch
        ]
        #expect(select(messages) == OTPSelection(code: "333333", uid: 503, allCandidateUIDs: [502, 503, 505]))
    }

    @Test func onlyStaleCandidatesYieldNil() {
        #expect(select([Self.message(uid: 501, secondsAfterPrompt: -30)]) == nil)
        #expect(select([]) == nil)
        #expect(select([Self.message(uid: 501, subject: "Hello")]) == nil)
    }

    @Test(arguments: [
        "=?UTF-8?B?7J247Kad7L2U65OcIEF1dGhDb2RlOiA0ODI5MTM=?=",
        "=?UTF-8?Q?AuthCode:_482913?=",
        "=?UTF-8?Q?=EC=9D=B8=EC=A6=9D?= AuthCode: 482913",
        "=?EUC-KR?B?wM7B9cTateUgQXV0aENvZGU6IDQ4MjkxMw==?=",
    ])
    func encodedSubjects(_ subject: String) {
        #expect(select([Self.message(uid: 500, subject: subject)])?.code == "482913")
    }

    @Test func foldedEncodedSubjectInHeaderBlock() {
        let message = FetchedHeaderMessage(
            uid: 600,
            internalDate: Self.promptedAt,
            headerBlock: Data("Date: x\r\nSubject: =?UTF-8?B?7J247Kad7L2U65OcIA==?=\r\n =?UTF-8?Q?AuthCode:_24680135?=\r\n\r\n".utf8)
        )
        #expect(message.subject == "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 24680135")
        #expect(select([message])?.code == "24680135")
    }

    // MARK: staleAuthCodeUIDs

    @Test func staleCleanupIgnoresDateButNotSnapshotOrSubject() {
        let messages = [
            Self.message(uid: 499),
            Self.message(uid: 500, secondsAfterPrompt: -3600),
            Self.message(uid: 502, secondsAfterPrompt: 3600),
            Self.message(uid: 501, subject: "Newsletter"),
            Self.message(uid: 502),
        ]
        #expect(OTPMatcher.staleAuthCodeUIDs(snapshot: Self.snapshot, currentUIDValidity: 11, messages: messages) == [500, 502])
        #expect(OTPMatcher.staleAuthCodeUIDs(snapshot: Self.snapshot, currentUIDValidity: 12, messages: messages) == [])
        #expect(OTPMatcher.staleAuthCodeUIDs(snapshot: Self.snapshot, currentUIDValidity: 11, messages: []) == [])
    }

    // MARK: Junk check

    @Test func freshAuthCodeInJunk() {
        #expect(OTPMatcher.containsFreshAuthCode(otpPromptedAt: Self.promptedAt, messages: [Self.message(uid: 3, secondsAfterPrompt: 10)]))
        #expect(!OTPMatcher.containsFreshAuthCode(otpPromptedAt: Self.promptedAt, messages: [Self.message(uid: 3, secondsAfterPrompt: -6)]))
        #expect(!OTPMatcher.containsFreshAuthCode(otpPromptedAt: Self.promptedAt, messages: [Self.message(uid: 3, subject: "Hi")]))
    }

    // MARK: Integration with the parser

    @Test func fetchRepliesFeedTheMatcher() throws {
        let header = "Subject: AuthCode: 482913\r\nDate: Wed, 14 Oct 2026 10:19:02 +0900\r\n\r\n"
        let stale = "Subject: AuthCode: 111111\r\nDate: Wed, 14 Oct 2026 10:00:00 +0900\r\n\r\n"
        let transcript = IMAPFixture.lines("* SEARCH 499 500 501")
            + #"* 10 FETCH (UID 499 INTERNALDATE "14-Oct-2026 10:19:01 +0900" BODY[HEADER.FIELDS (SUBJECT DATE)] "#
            + IMAPFixture.literal(header) + ")\r\n"
            + #"* 11 FETCH (UID 500 INTERNALDATE "14-Oct-2026 10:00:00 +0900" BODY[HEADER.FIELDS (SUBJECT DATE)] "#
            + IMAPFixture.literal(stale) + ")\r\n"
            + #"* 12 FETCH (UID 501 INTERNALDATE "14-Oct-2026 10:19:02 +0900" BODY[HEADER.FIELDS (SUBJECT DATE)] "#
            + IMAPFixture.literal(header) + ")\r\n"
            + #"* 12 FETCH (FLAGS (\Seen))"# + "\r\n"
            + IMAPFixture.lines("A0010 OK Success")
        let responses = try IMAPFixture.parseAll(transcript)
        let messages = responses.compactMap { response -> FetchedHeaderMessage? in
            if case .fetch(let fetch) = response { FetchedHeaderMessage(fetch) } else { nil }
        }
        #expect(messages.map(\.uid) == [499, 500, 501])
        let selection = select(messages)
        #expect(selection == OTPSelection(code: "482913", uid: 501, allCandidateUIDs: [500, 501]))
        let move = try IMAPCommand.uidMove(try #require(IMAPUIDSet(selection?.allCandidateUIDs ?? [])), mailbox: "[Gmail]/&1zTJwNG1-")
            .encoded(tag: "A0011")
        #expect(String(decoding: move, as: UTF8.self) == "A0011 UID MOVE 500:501 \"[Gmail]/&1zTJwNG1-\"\r\n")
    }

    @Test func fetchedHeaderMessageRequiresAllParts() {
        let header = Data("Subject: AuthCode: 1\r\n\r\n".utf8)
        #expect(FetchedHeaderMessage(IMAPFetchResponse(sequenceNumber: 1, uid: 5, internalDate: Self.promptedAt, headerBytes: header))
            == FetchedHeaderMessage(uid: 5, internalDate: Self.promptedAt, headerBlock: header))
        #expect(FetchedHeaderMessage(IMAPFetchResponse(sequenceNumber: 1, internalDate: Self.promptedAt, headerBytes: header)) == nil)
        #expect(FetchedHeaderMessage(IMAPFetchResponse(sequenceNumber: 1, uid: 5, headerBytes: header)) == nil)
        #expect(FetchedHeaderMessage(IMAPFetchResponse(sequenceNumber: 1, uid: 5, internalDate: Self.promptedAt)) == nil)
    }
}
