import Foundation
import Testing
@testable import FortiCore

@Suite("Mail header decoding")
struct MailHeaderDecodingTests {
    private func subject(_ block: String) -> String? {
        MailHeader.fieldValue("Subject", in: Data(block.utf8))
    }

    @Test func plainSubjectWithDate() {
        #expect(subject("Subject: AuthCode: 482913\r\nDate: Wed, 14 Oct 2026 10:19:00 +0900\r\n\r\n") == "AuthCode: 482913")
        #expect(MailHeader.fieldValue("date", in: Data("Subject: x\r\nDATE: Wed, 14 Oct 2026\r\n\r\n".utf8)) == "Wed, 14 Oct 2026")
    }

    @Test func fieldNamesAreCaseInsensitiveAndFirstWins() {
        #expect(subject("SUBJECT:AuthCode: 1\r\nsubject: second\r\n") == "AuthCode: 1")
        #expect(subject("Date: x\r\nsubject :  spaced  \r\n") == "spaced")
    }

    @Test func foldedSubjectIsUnfolded() {
        #expect(subject("Subject: Your one-time\r\n\tpassword\r\n AuthCode: 123456\r\nDate: x\r\n") == "Your one-time\tpassword AuthCode: 123456")
        #expect(subject("Date: x\nSubject: bare\n  LF\n\n") == "bare  LF")
    }

    @Test func missingSubjectAndHeaderEnd() {
        #expect(subject("Date: Wed, 14 Oct 2026 10:19:00 +0900\r\n\r\n") == nil)
        #expect(subject("") == nil)
        #expect(subject("Date: x\r\n\r\nSubject: in the body\r\n") == nil)
        #expect(subject("X-Subject: no\r\n") == nil)
    }

    // "\u{C778}\u{C99D}\u{CF54}\u{B4DC}" is Korean for "verification code".
    @Test func rawEightBitSubjects() {
        #expect(subject("Subject: \u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 123456\r\n") == "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 123456")
        let latin1 = Data("Subject: caf".utf8) + Data([0xE9]) + Data(" AuthCode: 654321\r\n".utf8)
        #expect(MailHeader.fieldValue("Subject", in: latin1) == "café AuthCode: 654321")
    }

    @Test(arguments: [
        ("=?UTF-8?B?7J247Kad7L2U65OcIEF1dGhDb2RlOiA0ODI5MTM=?=", "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 482913"),
        ("=?utf-8?b?7J247Kad7L2U65OcIEF1dGhDb2RlOiA0ODI5MTM?=", "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 482913"), // lowercase, no padding
        ("=?UTF-8?Q?AuthCode:_123456?=", "AuthCode: 123456"),
        ("=?UTF-8?Q?=EC=9D=B8=EC=A6=9D_AuthCode:_123456?=", "\u{C778}\u{C99D} AuthCode: 123456"),
        ("=?utf-8?q?=ec=9d=b8?=", "\u{C778}"),
        ("=?EUC-KR?B?wM7B9cTateUgQXV0aENvZGU6IDQ4MjkxMw==?=", "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 482913"),
        ("=?euc-kr?B?wM7B9cTateUgQXV0aENvZGU6IDQ4MjkxMw==?=", "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 482913"),
        ("=?ks_c_5601-1987?B?wM7B9cTateUgQXV0aENvZGU6IDQ4MjkxMw==?=", "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 482913"),
        ("=?ISO-8859-1?Q?caf=E9?= AuthCode: 1", "café AuthCode: 1"),
        ("=?UTF-8*ko?B?7J247Kad?=", "\u{C778}\u{C99D}"), // RFC 2231 language suffix
        // Whitespace (including folding) between adjacent encoded-words is dropped.
        ("=?UTF-8?Q?AuthCode:?= =?UTF-8?Q?_123456?=", "AuthCode: 123456"),
        ("=?UTF-8?Q?Auth?=\r\n =?UTF-8?Q?Code:_1?=", "AuthCode: 1"),
        // A multi-byte character split across adjacent words still decodes.
        ("=?UTF-8?B?7J247A==?= =?UTF-8?B?pp0=?=", "\u{C778}\u{C99D}"),
        // Adjacent words in different charsets.
        ("=?EUC-KR?B?wM4=?= =?UTF-8?B?7Kad?=", "\u{C778}\u{C99D}"),
        // Whitespace between an encoded-word and plain text is kept.
        ("=?UTF-8?Q?AuthCode:?= 123456", "AuthCode: 123456"),
        ("Re: =?UTF-8?B?7J247Kad?= AuthCode: 1", "Re: \u{C778}\u{C99D} AuthCode: 1"),
        // Lenient: encoded-word glued to text.
        ("AuthCode:=?UTF-8?Q?_777888?=", "AuthCode: 777888"),
        ("=?UTF-8?Q??=", ""),
    ])
    func encodedWordsAreDecoded(_ encoded: String, _ decoded: String) {
        #expect(RFC2047.decode(encoded) == decoded)
    }

    @Test(arguments: [
        "AuthCode: 123456",
        "100% =? sure",
        "=?UTF-8?B?7J247Kad",                // unterminated
        "=?x-unknown-charset?B?QUJD?=",      // unknown charset
        "=?UTF-8?X?QUJD?=",                  // unknown encoding
        "=?UTF-8?Q?bad=ZZ?=",                // bad hex
        "=?UTF-8?Q?a=E?=",                   // truncated hex
        "=?UTF-8?B?Q?=",                     // impossible base64 length
        "=?UTF-8?B?/w==?=",                  // decodes to invalid UTF-8
        "=?UTF-8?Q?has space?=",             // spaces are not allowed inside
        "=??B?QUJD?=",                       // empty charset
    ])
    func malformedEncodedWordsAreLeftVerbatim(_ text: String) {
        #expect(RFC2047.decode(text) == text)
    }

    @Test func encodedSubjectInsideHeaderBlock() {
        let block = "Subject: =?UTF-8?B?7J247Kad7L2U65OcIA==?=\r\n =?UTF-8?B?QXV0aENvZGU6IDQ4MjkxMw==?=\r\nDate: x\r\n\r\n"
        #expect(subject(block) == "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 482913")
    }

    @Test(arguments: ["UTF-8", "utf-8", "utf8", "EUC-KR", "euc-kr", "ISO-8859-1", "us-ascii", "ks_c_5601-1987", "CP949", "windows-1252"])
    func knownCharsets(_ charset: String) {
        #expect(RFC2047.encoding(forCharset: charset) != nil)
    }

    @Test func charsetMapping() {
        #expect(RFC2047.encoding(forCharset: "UTF-8") == .utf8)
        #expect(RFC2047.encoding(forCharset: "ISO-8859-1") == .isoLatin1)
        #expect(RFC2047.encoding(forCharset: "x-no-such-charset") == nil)
    }
}
