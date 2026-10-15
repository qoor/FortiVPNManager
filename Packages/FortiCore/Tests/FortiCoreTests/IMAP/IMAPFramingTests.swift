import Foundation
import Testing
@testable import FortiCore

/// Feeding the same bytes in any chunking must yield the same responses.
@Suite("IMAP response framing")
struct IMAPFramingTests {
    /// A session-shaped transcript with literals (one empty, one containing CRLF and `{n}` text),
    /// quoted escapes, and untagged updates.
    static let transcript: [UInt8] = {
        let header1 = "Subject: AuthCode: 482913\r\nDate: Wed, 14 Oct 2026 10:19:00 +0900\r\n\r\n"
        let header2 = "Subject: =?UTF-8?B?7J247Kad7L2U65OcIEF1dGhDb2RlOiA3NzExMjI=?=\r\n {7}\r\n\r\n"
        var text = IMAPFixture.lines(
            "* OK Gimap ready",
            "A0001 OK [CAPABILITY IMAP4rev1 IDLE MOVE] authenticated (Success)",
            #"* LIST (\HasNoChildren \All) "/" "[Gmail]/&yATMtLz0rQDVaA-""#
        )
        text += #"* LIST (\HasNoChildren \Trash) "/" "# + IMAPFixture.literal("[Gmail]/&1zTJwNG1-") + "\r\n"
        text += IMAPFixture.lines(
            "* OK [UIDVALIDITY 11] UIDs valid.",
            "* OK [UIDNEXT 500] Predicted next UID.",
            "+ idling",
            "* 499 EXISTS",
            #"* 498 FETCH (FLAGS (\Seen))"#,
            "* SEARCH 499 500"
        )
        text += #"* 499 FETCH (UID 500 INTERNALDATE "14-Oct-2026 10:19:00 +0900" BODY[HEADER.FIELDS (SUBJECT DATE)] "#
            + IMAPFixture.literal(header1) + ")\r\n"
        text += #"* 500 FETCH (UID 501 INTERNALDATE "14-Oct-2026 10:19:30 +0900" BODY[HEADER.FIELDS (SUBJECT DATE)] "#
            + IMAPFixture.literal(header2) + " X-EMPTY " + IMAPFixture.literal("") + ")\r\n"
        text += IMAPFixture.lines(
            #"* LIST (\HasNoChildren) "/" "a\"b""#,
            "A0009 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)",
            "* BYE bye"
        )
        return Array(text.utf8)
    }()

    static func expected() throws -> [IMAPResponse] {
        var parser = IMAPResponseParser()
        return try parser.feed(transcript)
    }

    @Test func oneShotParseIsComplete() throws {
        let responses = try Self.expected()
        #expect(responses.count == 15)
        #expect(!responses.contains { if case .other = $0 { true } else { false } })
        guard case .fetch(let second) = responses[11] else {
            Issue.record("expected fetch, got \(responses[11])")
            return
        }
        #expect(second.uid == 501)
        #expect(FetchedHeaderMessage(second)?.subject == "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 771122 {7}")
    }

    @Test func splitAtEveryOffset() throws {
        let expected = try Self.expected()
        for offset in 0...Self.transcript.count {
            var parser = IMAPResponseParser()
            var responses = try parser.feed(Self.transcript[..<offset])
            responses += try parser.feed(Self.transcript[offset...])
            #expect(responses == expected, "split at \(offset)")
            #expect(parser.bufferedByteCount == 0)
        }
    }

    @Test func byteByByte() throws {
        var parser = IMAPResponseParser()
        var responses: [IMAPResponse] = []
        for byte in Self.transcript {
            responses += try parser.feed(CollectionOfOne(byte))
        }
        #expect(responses == (try Self.expected()))
    }

    @Test(arguments: [UInt64(1), 7, 42, 1234, 99_999])
    func randomChunkings(seed: UInt64) throws {
        let expected = try Self.expected()
        var rng = SplitMix64(seed: seed)
        for _ in 0..<50 {
            var parser = IMAPResponseParser()
            var responses: [IMAPResponse] = []
            var index = 0
            while index < Self.transcript.count {
                let length = Int(rng.next() % 17) // 0...16, empty chunks included
                let end = min(index + length, Self.transcript.count)
                responses += try parser.feed(Data(Self.transcript[index..<end]))
                index = end
            }
            #expect(responses == expected)
        }
    }

    @Test func literalSplitInsideCountAndCRLF() throws {
        // Splits right inside "{1", "}\r", "\r\n" and the payload.
        let line = Array(("* 1 FETCH (UID 2 BODY[HEADER] {12}\r\nSubject: x\r\n)\r\n").utf8)
        let expected = [IMAPResponse.fetch(IMAPFetchResponse(sequenceNumber: 1, uid: 2, headerBytes: Data("Subject: x\r\n".utf8)))]
        for first in 0...line.count {
            for second in first...line.count {
                var parser = IMAPResponseParser()
                var responses = try parser.feed(line[..<first])
                responses += try parser.feed(line[first..<second])
                responses += try parser.feed(line[second...])
                #expect(responses == expected, "splits at \(first), \(second)")
            }
        }
    }

    /// Small deterministic PRNG for reproducible chunkings.
    private struct SplitMix64 {
        private var state: UInt64

        init(seed: UInt64) {
            state = seed
        }

        mutating func next() -> UInt64 {
            state &+= 0x9E37_79B9_7F4A_7C15
            var z = state
            z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
            z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
            return z ^ (z >> 31)
        }
    }
}
