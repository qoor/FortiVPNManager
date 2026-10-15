import Foundation
import Testing
@testable import FortiCore

@Suite("IMAP response parser")
struct IMAPResponseParserTests {
    @Test func greetingAndCapability() throws {
        let responses = try IMAPFixture.parseAll(IMAPFixture.lines(
            "* OK Gimap ready for requests from 203.0.113.7 a1b2c3",
            "* CAPABILITY IMAP4rev1 UNSELECT IDLE NAMESPACE QUOTA ID XLIST CHILDREN X-GM-EXT-1 UIDPLUS COMPRESS=DEFLATE ENABLE MOVE CONDSTORE ESEARCH UTF8=ACCEPT LIST-EXTENDED LIST-STATUS LITERAL- SPECIAL-USE APPENDLIMIT=35651584",
            "A0001 OK [CAPABILITY IMAP4rev1 IDLE MOVE SPECIAL-USE] user@example.com authenticated (Success)"
        ))
        #expect(responses.count == 3)
        #expect(responses[0] == .untaggedStatus(.ok, code: nil, text: "Gimap ready for requests from 203.0.113.7 a1b2c3"))
        guard case .capability(let capabilities) = responses[1] else {
            Issue.record("expected capability, got \(responses[1])")
            return
        }
        #expect(capabilities.contains("IDLE") && capabilities.contains("MOVE") && capabilities.contains("SPECIAL-USE"))
        #expect(capabilities.count == 21)
        #expect(responses[2] == .tagged(
            tag: "A0001",
            status: .ok,
            code: .capability(["IMAP4rev1", "IDLE", "MOVE", "SPECIAL-USE"]),
            text: "user@example.com authenticated (Success)"
        ))
    }

    @Test func taggedNoAuthenticationFailed() throws {
        let responses = try IMAPFixture.parseAll(IMAPFixture.lines("A0001 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)"))
        #expect(responses == [.tagged(tag: "A0001", status: .no, code: .authenticationFailed, text: "Invalid credentials (Failure)")])
    }

    @Test(arguments: [
        ("A0001 NO [ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)",
         IMAPResponse.tagged(tag: "A0001", status: .no, code: .alert,
                             text: "Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)")),
        ("A0002 BAD Could not parse command",
         .tagged(tag: "A0002", status: .bad, code: nil, text: "Could not parse command")),
        ("a7 ok [READ-WRITE] [Gmail]/All Mail selected. (Success)",
         .tagged(tag: "a7", status: .ok, code: .readWrite, text: "[Gmail]/All Mail selected. (Success)")),
        ("A0003 OK",
         .tagged(tag: "A0003", status: .ok, code: nil, text: "")),
        ("A0004 NO [WEBALERT https://accounts.google.com/signin/continue?x=1] Web login required.",
         .tagged(tag: "A0004", status: .no, code: .other(name: "WEBALERT", argument: "https://accounts.google.com/signin/continue?x=1"),
                 text: "Web login required.")),
        ("A0005 NO [TRYCREATE] No folder [Gmail]/Nope (Failure)",
         .tagged(tag: "A0005", status: .no, code: .tryCreate, text: "No folder [Gmail]/Nope (Failure)")),
        ("A0006 OK [READ-ONLY] Examined",
         .tagged(tag: "A0006", status: .ok, code: .readOnly, text: "Examined")),
        ("* NO [UNAVAILABLE] Temporary System Problem",
         .untaggedStatus(.no, code: .unavailable, text: "Temporary System Problem")),
        ("* BAD [CLIENTBUG] Unknown command",
         .untaggedStatus(.bad, code: .other(name: "CLIENTBUG", argument: nil), text: "Unknown command")),
        ("* BYE System error",
         .bye(code: nil, text: "System error")),
        ("* BYE [UNAVAILABLE] Server shutting down",
         .bye(code: .unavailable, text: "Server shutting down")),
        ("+ idling", .continuation("idling")),
        ("+", .continuation("")),
        ("+ ", .continuation("")),
    ])
    func statusResponses(_ line: String, _ expected: IMAPResponse) throws {
        #expect(try IMAPFixture.parseAll(IMAPFixture.lines(line)) == [expected])
    }

    @Test func selectRepliesProduceSnapshot() throws {
        let responses = try IMAPFixture.parseAll(IMAPFixture.lines(
            #"* FLAGS (\Answered \Flagged \Draft \Deleted \Seen $NotPhishing $Phishing)"#,
            #"* OK [PERMANENTFLAGS (\Answered \Flagged \Draft \Deleted \Seen $NotPhishing $Phishing \*)] Flags permitted."#,
            "* OK [UIDVALIDITY 11] UIDs valid.",
            "* 48213 EXISTS",
            "* 0 RECENT",
            "* OK [UIDNEXT 512345] Predicted next UID.",
            "* OK [HIGHESTMODSEQ 9876543]",
            "A0003 OK [READ-WRITE] [Gmail]/&yATMtLz0rQDVaA- selected. (Success)"
        ))
        #expect(responses.count == 8)
        #expect(responses[0] == .other(#"* FLAGS (\Answered \Flagged \Draft \Deleted \Seen $NotPhishing $Phishing)"#))
        #expect(responses[1] == .untaggedStatus(
            .ok,
            code: .other(name: "PERMANENTFLAGS", argument: #"(\Answered \Flagged \Draft \Deleted \Seen $NotPhishing $Phishing \*)"#),
            text: "Flags permitted."
        ))
        #expect(responses[2] == .untaggedStatus(.ok, code: .uidValidity(11), text: "UIDs valid."))
        #expect(responses[3] == .exists(48213))
        #expect(responses[4] == .other("* 0 RECENT"))
        #expect(responses[5] == .untaggedStatus(.ok, code: .uidNext(512_345), text: "Predicted next UID."))
        #expect(responses[6] == .untaggedStatus(.ok, code: .other(name: "HIGHESTMODSEQ", argument: "9876543"), text: ""))
        #expect(responses[7] == .tagged(tag: "A0003", status: .ok, code: .readWrite, text: "[Gmail]/&yATMtLz0rQDVaA- selected. (Success)"))
        #expect(MailboxSnapshot(selectResponses: responses) == MailboxSnapshot(uidValidity: 11, uidNext: 512_345))
    }

    @Test func snapshotNeedsBothCodes() throws {
        let withoutNext = try IMAPFixture.parseAll(IMAPFixture.lines("* OK [UIDVALIDITY 11] UIDs valid.", "A1 OK [READ-WRITE] done"))
        #expect(MailboxSnapshot(selectResponses: withoutNext) == nil)
        let badNumber = try IMAPFixture.parseAll(IMAPFixture.lines("* OK [UIDVALIDITY 11]", "* OK [UIDNEXT 99999999999]"))
        #expect(badNumber[1] == .untaggedStatus(.ok, code: .other(name: "UIDNEXT", argument: "99999999999"), text: ""))
        #expect(MailboxSnapshot(selectResponses: badNumber) == nil)
    }

    @Test func gmailListWithSpecialUseAndKoreanNames() throws {
        let responses = try IMAPFixture.parseAll(IMAPFixture.lines(
            #"* LIST (\HasNoChildren) "/" "INBOX""#,
            #"* LIST (\HasChildren \Noselect) "/" "[Gmail]""#,
            #"* LIST (\HasNoChildren \All) "/" "[Gmail]/&yATMtLz0rQDVaA-""#,
            #"* LIST (\HasNoChildren \Sent) "/" "[Gmail]/&vPSwuNO4ycDVaA-""#,
            #"* LIST (\Flagged \HasNoChildren) "/" "[Gmail]/&vMTUXNO4ycDVaA-""#,
            #"* LIST (\HasNoChildren \Important) "/" "[Gmail]/&yRHGlA-""#,
            #"* LIST (\HasNoChildren \Junk) "/" "[Gmail]/&wqTTONVo-""#,
            #"* LIST (\HasNoChildren \Trash) "/" "[Gmail]/&1zTJwNG1-""#,
            #"* LIST (\HasNoChildren) "/" "Tom &- Jerry""#,
            "A0002 OK Success"
        ))
        let entries = responses.compactMap { response -> IMAPListEntry? in
            if case .list(let entry) = response { entry } else { nil }
        }
        #expect(entries.count == 9)
        #expect(entries[2] == IMAPListEntry(attributes: ["\\HasNoChildren", "\\All"], delimiter: "/", name: "[Gmail]/&yATMtLz0rQDVaA-"))
        #expect(entries[2].displayName == "[Gmail]/\u{C804}\u{CCB4}\u{BCF4}\u{AD00}\u{D568}")
        #expect(entries[2].specialUses == [.all])
        #expect(entries[1].isSelectable == false)
        #expect(entries[8].displayName == "Tom & Jerry")

        let special = IMAPSpecialMailboxes(responses: responses)
        #expect(special.all?.name == "[Gmail]/&yATMtLz0rQDVaA-")
        #expect(special.trash?.name == "[Gmail]/&1zTJwNG1-")
        #expect(special.trash?.displayName == "[Gmail]/\u{D734}\u{C9C0}\u{D1B5}")
        #expect(special.junk?.displayName == "[Gmail]/\u{C2A4}\u{D338}\u{D568}")
        #expect(responses.last == .tagged(tag: "A0002", status: .ok, code: nil, text: "Success"))
    }

    @Test func listVariants() throws {
        let responses = try IMAPFixture.parseAll(
            #"* LIST (\HasNoChildren \Trash) "/" "#.appending(IMAPFixture.literal("[Gmail]/Trash")).appending("\r\n")
                + IMAPFixture.lines(
                    #"* LIST (\Noselect) NIL """#,
                    #"* LIST () "." INBOX"#,
                    #"* LIST (\HasNoChildren) "/" "quote\"and\\slash""#,
                    #"* LIST (\HasNoChildren) "/" [Gmail]/Bare"#,
                    #"* LIST (\all \hasnochildren) "/" "lower""#,
                    #"* XLIST (\HasNoChildren \AllMail) "/" "[Gmail]/All Mail""#,
                    #"* LIST (\HasNoChildren \Trash) "/" "[Gmail]/Bin" ("CHILDINFO" ("SUBSCRIBED"))"#
                )
        )
        #expect(responses == [
            .list(IMAPListEntry(attributes: ["\\HasNoChildren", "\\Trash"], delimiter: "/", name: "[Gmail]/Trash")),
            .list(IMAPListEntry(attributes: ["\\Noselect"], delimiter: nil, name: "")),
            .list(IMAPListEntry(attributes: [], delimiter: ".", name: "INBOX")),
            .list(IMAPListEntry(attributes: ["\\HasNoChildren"], delimiter: "/", name: #"quote"and\slash"#)),
            .list(IMAPListEntry(attributes: ["\\HasNoChildren"], delimiter: "/", name: "[Gmail]/Bare")),
            .list(IMAPListEntry(attributes: ["\\all", "\\hasnochildren"], delimiter: "/", name: "lower")),
            .list(IMAPListEntry(attributes: ["\\HasNoChildren", "\\AllMail"], delimiter: "/", name: "[Gmail]/All Mail")),
            .list(IMAPListEntry(attributes: ["\\HasNoChildren", "\\Trash"], delimiter: "/", name: "[Gmail]/Bin")),
        ])
        // Attribute matching is case-insensitive.
        guard case .list(let lower) = responses[5] else { return }
        #expect(lower.specialUses == [.all])
        // The quoted name re-serializes with the same escapes.
        guard case .list(let quoted) = responses[3] else { return }
        #expect(String(decoding: try IMAPCommand.select(mailbox: quoted.name).encoded(tag: "A9"), as: UTF8.self)
            == #"A9 SELECT "quote\"and\\slash""# + "\r\n")
    }

    @Test(arguments: [
        ("* SEARCH", [UInt32]()),
        ("* SEARCH ", []),
        ("* SEARCH 4021", [4021]),
        ("* SEARCH 5 6 4294967295", [5, 6, 4_294_967_295]),
        ("* search 7 8 (MODSEQ 917162500)", [7, 8]),
    ])
    func searchResults(_ line: String, _ expected: [UInt32]) throws {
        #expect(try IMAPFixture.parseAll(IMAPFixture.lines(line)) == [.search(expected)])
    }

    @Test func fetchWithHeaderLiteral() throws {
        let header = "Subject: AuthCode: 482913\r\nDate: Wed, 14 Oct 2026 10:19:00 +0900\r\n\r\n"
        let transcript = #"* 48214 FETCH (UID 512345 INTERNALDATE "14-Oct-2026 10:19:00 +0900" BODY[HEADER.FIELDS (SUBJECT DATE)] "#
            + IMAPFixture.literal(header) + ")\r\n"
            + IMAPFixture.lines("A0005 OK Success")
        let responses = try IMAPFixture.parseAll(transcript)
        #expect(responses == [
            .fetch(IMAPFetchResponse(
                sequenceNumber: 48214,
                uid: 512_345,
                internalDate: Date(timeIntervalSince1970: 1_791_940_740),
                headerBytes: Data(header.utf8)
            )),
            .tagged(tag: "A0005", status: .ok, code: nil, text: "Success"),
        ])
    }

    @Test func fetchItemOrderFlagsAndUnknownItems() throws {
        let header = "SUBJECT: AuthCode: 111222\r\n\r\n"
        let transcript = #"* 3 FETCH (X-GM-LABELS ("\\Inbox" "\\Important" Work) FLAGS (\Seen $Label1) "#
            + #"BODY[HEADER.FIELDS ("SUBJECT" "DATE")] "# + IMAPFixture.literal(header)
            + #" RFC822.SIZE 4512 INTERNALDATE " 4-Oct-2026 00:00:00 +0000" UID 77 MODSEQ (12345))"# + "\r\n"
        #expect(try IMAPFixture.parseAll(transcript) == [
            .fetch(IMAPFetchResponse(
                sequenceNumber: 3,
                uid: 77,
                internalDate: Date(timeIntervalSince1970: 1_791_072_000),
                flags: ["\\Seen", "$Label1"],
                headerBytes: Data(header.utf8)
            )),
        ])
    }

    @Test func fetchWithQuotedOrNilHeader() throws {
        let responses = try IMAPFixture.parseAll(IMAPFixture.lines(
            #"* 1 FETCH (UID 10 BODY[HEADER.FIELDS (SUBJECT DATE)] "Subject: x\"y")"#,
            #"* 2 FETCH (UID 11 BODY[HEADER.FIELDS (SUBJECT DATE)] NIL)"#,
            #"* 3 FETCH (UID 12 RFC822.HEADER "Subject: z")"#,
            #"* 4 FETCH (UID 13 BODY[HEADER] {0}"# + "\r\n)"
        ))
        #expect(responses == [
            .fetch(IMAPFetchResponse(sequenceNumber: 1, uid: 10, headerBytes: Data(#"Subject: x"y"#.utf8))),
            .fetch(IMAPFetchResponse(sequenceNumber: 2, uid: 11)),
            .fetch(IMAPFetchResponse(sequenceNumber: 3, uid: 12, headerBytes: Data("Subject: z".utf8))),
            .fetch(IMAPFetchResponse(sequenceNumber: 4, uid: 13, headerBytes: Data())),
        ])
    }

    @Test func fetchWithUnparsableInternalDateKeepsOtherItems() throws {
        #expect(try IMAPFixture.parseAll(IMAPFixture.lines(#"* 1 FETCH (UID 10 INTERNALDATE "yesterday")"#)) == [
            .fetch(IMAPFetchResponse(sequenceNumber: 1, uid: 10, internalDate: nil)),
        ])
    }

    @Test func untaggedUpdatesInterleavedDuringIdle() throws {
        let transcript = IMAPFixture.lines(
            "+ idling",
            "* 48215 EXISTS",
            "* 120 EXPUNGE",
            #"* 48213 FETCH (FLAGS (\Seen))"#,
            "* 48214 FETCH (UID 512346 FLAGS ())",
            "* 48215 EXISTS",
            "A0007 OK IDLE terminated (Success)"
        )
        #expect(try IMAPFixture.parseAll(transcript) == [
            .continuation("idling"),
            .exists(48215),
            .expunge(120),
            .fetch(IMAPFetchResponse(sequenceNumber: 48213, flags: ["\\Seen"])),
            .fetch(IMAPFetchResponse(sequenceNumber: 48214, uid: 512_346, flags: [])),
            .exists(48215),
            .tagged(tag: "A0007", status: .ok, code: nil, text: "IDLE terminated (Success)"),
        ])
    }

    @Test func moveReplies() throws {
        let transcript = IMAPFixture.lines(
            "* OK [COPYUID 7 512345:512346 901:902] (Success)",
            "* 48214 EXPUNGE",
            "* 48214 EXPUNGE",
            "* 48212 EXISTS",
            "A0008 OK Success"
        )
        #expect(try IMAPFixture.parseAll(transcript) == [
            .untaggedStatus(.ok, code: .other(name: "COPYUID", argument: "7 512345:512346 901:902"), text: "(Success)"),
            .expunge(48214),
            .expunge(48214),
            .exists(48212),
            .tagged(tag: "A0008", status: .ok, code: nil, text: "Success"),
        ])
    }

    @Test(arguments: [
        "* LIST garbage",
        "* LIST (\\HasNoChildren) \"/\"",
        "* 99999999999 EXISTS",
        "* 12 EXISTS extra",
        "* 12 FETCH UID 5",
        "* 12 FETCH (UID)",
        "* 12 FETCH (UID abc)",
        "* 12 FETCH (UID 5 FLAGS \\Seen)",
        "* 12 FETCH (UID 5",
        "* SEARCH 1 two 3",
        "* ENABLED UTF8=ACCEPT",
        "* ID (\"name\" \"GImap\")",
        "*",
        "* ",
        "A0001 MAYBE later",
        "A0001",
        "",
        "garbage without status",
        "* LIST (\\HasNoChildren) \"/\" \"unterminated",
    ])
    func malformedOrUnmodelledLinesBecomeOther(_ line: String) throws {
        #expect(try IMAPFixture.parseAll(IMAPFixture.lines(line)) == [.other(line)])
    }

    @Test func deeplyNestedListsAreRejectedWithoutRecursingUnbounded() throws {
        let depth = 100_000
        let line = "* 1 FETCH (X " + String(repeating: "(", count: depth) + String(repeating: ")", count: depth) + " UID 5)"
        var parser = IMAPResponseParser()
        #expect(try parser.feed(Data(IMAPFixture.lines(line).utf8)) == [.other(line)])

        let allowed = "* 1 FETCH (X " + String(repeating: "(", count: 31) + String(repeating: ")", count: 31) + " UID 5)"
        #expect(try IMAPFixture.parseAll(IMAPFixture.lines(allowed)) == [.fetch(IMAPFetchResponse(sequenceNumber: 1, uid: 5))])
    }

    @Test func bareLineFeedsAreTolerated() throws {
        var parser = IMAPResponseParser()
        let responses = try parser.feed(Data("* 3 EXISTS\nA1 OK done\n* 1 FETCH (UID 4 BODY[HEADER] {2}\nab)\n".utf8))
        #expect(responses == [
            .exists(3),
            .tagged(tag: "A1", status: .ok, code: nil, text: "done"),
            .fetch(IMAPFetchResponse(sequenceNumber: 1, uid: 4, headerBytes: Data("ab".utf8))),
        ])
    }

    @Test func literalPayloadMayContainCRLFAndBraces() throws {
        let header = "Subject: weird {5}\r\nX: )\r\n\r\n"
        let transcript = "* 9 FETCH (BODY[HEADER] " + IMAPFixture.literal(header) + " UID 99)\r\n"
        #expect(try IMAPFixture.parseAll(transcript) == [
            .fetch(IMAPFetchResponse(sequenceNumber: 9, uid: 99, headerBytes: Data(header.utf8))),
        ])
    }

    @Test func multipleLiteralsInOneResponse() throws {
        let transcript = "* 1 FETCH (BODY[HEADER] " + IMAPFixture.literal("H\r\n") + " X-ONE " + IMAPFixture.literal("abc")
            + " X-ZERO ~" + IMAPFixture.literal("") + " UID 5)\r\n"
        #expect(try IMAPFixture.parseAll(transcript) == [
            .fetch(IMAPFetchResponse(sequenceNumber: 1, uid: 5, headerBytes: Data("H\r\n".utf8))),
        ])
    }

    @Test func eightBitBytesInLiteralArePreserved() throws {
        let payload: [UInt8] = Array("Subject: ".utf8) + [0xC0, 0xCE, 0xFF, 0x00, 0x0D, 0x0A]
        var bytes = Array("* 2 FETCH (UID 8 BODY[HEADER] {\(payload.count)}\r\n".utf8)
        bytes += payload
        bytes += Array(")\r\n".utf8)
        var parser = IMAPResponseParser()
        #expect(try parser.feed(bytes) == [
            .fetch(IMAPFetchResponse(sequenceNumber: 2, uid: 8, headerBytes: Data(payload))),
        ])
    }

    @Test func partialInputStaysBuffered() throws {
        var parser = IMAPResponseParser()
        #expect(try parser.feed(Data("* 5 EXI".utf8)).isEmpty)
        #expect(parser.bufferedByteCount == 7)
        #expect(try parser.feed(Data("STS\r\n* 6".utf8)) == [.exists(5)])
        #expect(parser.bufferedByteCount == 3)
        #expect(try parser.feed(Data(" EXISTS\r\n".utf8)) == [.exists(6)])
        #expect(parser.bufferedByteCount == 0)
        #expect(try parser.feed(Data()).isEmpty)
    }

    @Test func oversizedLiteralIsFatal() throws {
        var parser = IMAPResponseParser(maxLiteralSize: 100, maxResponseSize: 1000)
        #expect(throws: IMAPParseError.literalTooLarge(declared: 101)) {
            try parser.feed(Data("* 1 FETCH (BODY[HEADER] {101}\r\n".utf8))
        }
        // The parser stays failed.
        #expect(throws: IMAPParseError.literalTooLarge(declared: 101)) { try parser.feed(Data("* 1 EXISTS\r\n".utf8)) }

        var overflow = IMAPResponseParser()
        #expect(throws: IMAPParseError.literalTooLarge(declared: .max)) {
            try overflow.feed(Data("* 1 FETCH (BODY[HEADER] {99999999999999999999999}\r\n".utf8))
        }
    }

    @Test func oversizedResponseIsFatal() throws {
        var parser = IMAPResponseParser(maxLiteralSize: 100, maxResponseSize: 64)
        #expect(throws: IMAPParseError.responseTooLarge) {
            try parser.feed(Data(("* OK " + String(repeating: "x", count: 100) + "\r\n").utf8))
        }
        var literal = IMAPResponseParser(maxLiteralSize: 100, maxResponseSize: 64)
        #expect(throws: IMAPParseError.responseTooLarge) {
            try literal.feed(Data("* 1 FETCH (BODY[HEADER] {60}\r\n".utf8))
        }
        // Exactly at the limit is fine.
        var exact = IMAPResponseParser(maxLiteralSize: 100, maxResponseSize: 12)
        #expect(try exact.feed(Data("* 1 EXISTS\r\n".utf8)) == [.exists(1)])
    }
}
