import Foundation
import Testing
@testable import FortiCore

@Suite("IMAP command builders")
struct IMAPCommandTests {
    private func wire(_ command: IMAPCommand, tag: String = "A0001") throws -> String {
        String(decoding: try command.encoded(tag: tag), as: UTF8.self)
    }

    @Test func tagGeneratorProducesSequentialZeroPaddedTags() {
        var tags = IMAPTagGenerator()
        #expect(tags.next() == "A0001")
        #expect(tags.next() == "A0002")
        var copy = tags // value semantics: the copy continues independently
        #expect(copy.next() == "A0003")
        #expect(tags.next() == "A0003")

        var custom = IMAPTagGenerator(prefix: "OTP")
        for _ in 0..<9_999 { _ = custom.next() }
        #expect(custom.next() == "OTP10000")
    }

    @Test func authenticatePlainUsesSASLInitialResponse() throws {
        let command = IMAPCommand.authenticatePlain(username: "user@example.com", password: "abcd efgh ijkl mnop")
        #expect(try wire(command) == "A0001 AUTHENTICATE PLAIN AHVzZXJAZXhhbXBsZS5jb20AYWJjZCBlZmdoIGlqa2wgbW5vcA==\r\n")
    }

    @Test func authenticatePlainEncodesUTF8Credentials() throws {
        let command = IMAPCommand.authenticatePlain(username: "\u{C0AC}\u{C6A9}\u{C790}@example.com", password: "pässwörd")
        let line = try wire(command)
        #expect(line.hasPrefix("A0001 AUTHENTICATE PLAIN ") && line.hasSuffix("\r\n"))
        let base64 = try #require(line.split(separator: " ").last).trimmingCharacters(in: .newlines)
        let decoded = try #require(Data(base64Encoded: base64))
        #expect(decoded == Data([0] + Array("\u{C0AC}\u{C6A9}\u{C790}@example.com".utf8) + [0] + Array("pässwörd".utf8)))
    }

    @Test(arguments: [
        IMAPCommand.authenticatePlain(username: "", password: "pw"),
        .authenticatePlain(username: "us\0er", password: "pw"),
        .authenticatePlain(username: "user", password: "p\0w"),
    ])
    func authenticatePlainRejectsInvalidCredentials(_ command: IMAPCommand) {
        #expect(throws: IMAPCommandError.invalidCredentials) { try command.encoded(tag: "A1") }
    }

    @Test func descriptionRedactsPassword() {
        let command = IMAPCommand.authenticatePlain(username: "user@example.com", password: "s3cret-app-pw")
        #expect(!command.description.contains("s3cret"))
        #expect(!String(reflecting: command).contains("s3cret"))
        #expect(IMAPCommand.list.description == #"LIST "" "*""#)
    }

    @Test(arguments: [
        (IMAPCommand.list, #"A0001 LIST "" "*""#),
        (.select(mailbox: "INBOX"), #"A0001 SELECT "INBOX""#),
        (.select(mailbox: "[Gmail]/&yATMtLz0rQDVaA-"), #"A0001 SELECT "[Gmail]/&yATMtLz0rQDVaA-""#),
        (.select(mailbox: #"odd "name" \ here"#), #"A0001 SELECT "odd \"name\" \\ here""#),
        (.examine(mailbox: "[Gmail]/&wqTTONVo-"), #"A0001 EXAMINE "[Gmail]/&wqTTONVo-""#),
        (.uidSearch(.from(42)), "A0001 UID SEARCH UID 42:*"),
        (.uidSearchSubject("AuthCode"), #"A0001 UID SEARCH SUBJECT "AuthCode""#),
        (.uidFetchHeaders(IMAPUIDSet([10, 5, 6, 7])!),
         "A0001 UID FETCH 5:7,10 (UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT DATE)])"),
        (.uidFetchHeaders(.from(300)),
         "A0001 UID FETCH 300:* (UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT DATE)])"),
        (.idle, "A0001 IDLE"),
        (.uidMove(IMAPUIDSet([9, 7])!, mailbox: "[Gmail]/&1zTJwNG1-"), #"A0001 UID MOVE 7,9 "[Gmail]/&1zTJwNG1-""#),
        (.noop, "A0001 NOOP"),
        (.logout, "A0001 LOGOUT"),
    ])
    func commandsSerializeExactly(_ command: IMAPCommand, _ expected: String) throws {
        #expect(try wire(command) == expected + "\r\n")
    }

    @Test func idleDoneIsUntagged() {
        #expect(IMAPCommand.idleDone == Data("DONE\r\n".utf8))
    }

    @Test(arguments: ["Trash\r\nA2 LOGOUT", "Trash\n", "Tr\0sh", "[Gmail]/\u{D734}\u{C9C0}\u{D1B5}"])
    func mailboxNamesThatCannotBeQuotedAreRejected(_ name: String) {
        #expect(throws: IMAPCommandError.invalidMailboxName) { try IMAPCommand.select(mailbox: name).encoded(tag: "A1") }
        #expect(throws: IMAPCommandError.invalidMailboxName) {
            try IMAPCommand.uidMove(IMAPUIDSet([1])!, mailbox: name).encoded(tag: "A1")
        }
    }

    @Test func searchStringWithLineBreakIsRejected() {
        #expect(throws: IMAPCommandError.invalidSearchString) {
            try IMAPCommand.uidSearchSubject("AuthCode\r\nA2 LOGOUT").encoded(tag: "A1")
        }
    }

    @Test(arguments: ["", "A 1", "A1\r\n", "A+1", "*", "\u{D0DC}\u{ADF8}"])
    func invalidTagsAreRejected(_ tag: String) {
        #expect(throws: IMAPCommandError.invalidTag) { try IMAPCommand.noop.encoded(tag: tag) }
    }

    @Test func modifiedUTF7NamesFromLISTRoundTripIntoSELECT() throws {
        let entry = IMAPListEntry(attributes: ["\\HasNoChildren", "\\Trash"], delimiter: "/", name: ModifiedUTF7.encode("[Gmail]/\u{D734}\u{C9C0}\u{D1B5}"))
        #expect(entry.name == "[Gmail]/&1zTJwNG1-")
        #expect(try wire(.select(mailbox: entry.name)) == "A0001 SELECT \"[Gmail]/&1zTJwNG1-\"\r\n")
    }
}

@Suite("IMAP UID sets")
struct IMAPUIDSetTests {
    @Test func emptyOrZeroIsRejected() {
        #expect(IMAPUIDSet([UInt32]()) == nil)
        #expect(IMAPUIDSet([0, 1]) == nil)
    }

    @Test(arguments: [
        ([UInt32(1)], "1"),
        ([3, 1, 2], "1:3"),
        ([5, 5, 6, 9, 10, 11, 20], "5:6,9:11,20"),
        ([UInt32.max, 1], "1,4294967295"),
        ([UInt32.max - 1, UInt32.max], "4294967294:4294967295"),
    ])
    func explicitSetsAreCompacted(_ uids: [UInt32], _ expected: String) throws {
        #expect(try #require(IMAPUIDSet(uids)).description == expected)
    }

    @Test func openEndedRange() {
        #expect(IMAPUIDSet.from(7).description == "7:*")
        #expect(IMAPUIDSet.from(0).description == "1:*")
    }
}
