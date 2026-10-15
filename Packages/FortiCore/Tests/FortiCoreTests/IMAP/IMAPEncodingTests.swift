import Foundation
import Testing
@testable import FortiCore

@Suite("INTERNALDATE parsing")
struct IMAPInternalDateTests {
    @Test(arguments: [
        ("14-Oct-2026 10:19:00 +0900", 1_791_940_740.0),
        (" 4-Oct-2026 00:00:00 +0000", 1_791_072_000),
        ("4-Oct-2026 00:00:00 +0000", 1_791_072_000),
        ("04-oct-2026 00:00:00 +0000", 1_791_072_000),
        ("29-Feb-2024 23:59:59 -0700", 1_709_276_399),
        ("01-Jan-1970 00:00:00 +0000", 0),
        ("31-Dec-1969 23:00:00 -0130", 1800),
        ("15-OCT-2026 09:30:05 +0530", 1_792_036_805),
        ("  15-Oct-2026 09:30:05 +0530  ", 1_792_036_805),
    ])
    func validDates(_ text: String, _ epoch: TimeInterval) {
        #expect(IMAPInternalDate.parse(text) == Date(timeIntervalSince1970: epoch))
    }

    @Test(arguments: [
        "",
        "yesterday",
        "29-Feb-2026 00:00:00 +0000", // not a leap year
        "31-Apr-2026 00:00:00 +0000",
        "00-Oct-2026 00:00:00 +0000",
        "14-Okt-2026 10:19:00 +0900",
        "14-Oct-26 10:19:00 +0900",
        "14-Oct-2026 24:00:00 +0900",
        "14-Oct-2026 10:60:00 +0900",
        "14-Oct-2026 10:19 +0900",
        "14-Oct-2026 10:19:00",
        "14-Oct-2026 10:19:00 0900",
        "14-Oct-2026 10:19:00 +09:00",
        "14-Oct-2026 10:19:00 +0960",
        "14-Oct-2026 10:19:00 +0900 KST",
        "Wed, 14 Oct 2026 10:19:00 +0900",
    ])
    func invalidDates(_ text: String) {
        #expect(IMAPInternalDate.parse(text) == nil)
    }
}

@Suite("Modified UTF-7 mailbox names")
struct ModifiedUTF7Tests {
    @Test(arguments: [
        // Korean Gmail system folders.
        ("[Gmail]/&yATMtLz0rQDVaA-", "[Gmail]/\u{C804}\u{CCB4}\u{BCF4}\u{AD00}\u{D568}"), // All Mail
        ("[Gmail]/&1zTJwNG1-", "[Gmail]/\u{D734}\u{C9C0}\u{D1B5}"), // Trash
        ("[Gmail]/&wqTTONVo-", "[Gmail]/\u{C2A4}\u{D338}\u{D568}"), // Spam
        ("[Gmail]/&vPSwuNO4ycDVaA-", "[Gmail]/\u{BCF4}\u{B0B8}\u{D3B8}\u{C9C0}\u{D568}"), // Sent Mail
        ("~peter/mail/&U,BTFw-/&ZeVnLIqe-", "~peter/mail/台北/日本語"), // RFC 3501 §5.1.3
        ("Tom &- Jerry", "Tom & Jerry"),
        ("INBOX", "INBOX"),
        ("&2D3eAA-", "😀"),
    ])
    func roundTrip(_ wire: String, _ name: String) {
        #expect(ModifiedUTF7.decode(wire) == name)
        #expect(ModifiedUTF7.encode(name) == wire)
    }

    @Test(arguments: [
        "&yATMtLz0rQDVaA",   // unterminated
        "&yAT!-",            // invalid base64 character
        "&yA/-",             // "/" is not modified base64
        "&yA-",              // odd number of bytes
        "&2D0-",             // lone surrogate
        "tab\there",         // not printable ASCII
        "\u{D55C}\u{AE00}",  // raw 8-bit
    ])
    func invalidNamesDoNotDecode(_ wire: String) {
        #expect(ModifiedUTF7.decode(wire) == nil)
    }

    @Test func displayNameFallsBackToWireForm() {
        let entry = IMAPListEntry(attributes: [], delimiter: "/", name: "&broken")
        #expect(entry.displayName == "&broken")
    }
}
