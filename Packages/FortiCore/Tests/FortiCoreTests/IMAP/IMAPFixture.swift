import Foundation
@testable import FortiCore

/// Shared helpers for the IMAP/OTP tests (namespaced to avoid clashes with other test files).
enum IMAPFixture {
    /// Joins raw lines with CRLF (and a trailing CRLF).
    static func lines(_ lines: String...) -> String {
        lines.map { $0 + "\r\n" }.joined()
    }

    /// `{n}` + CRLF + payload for a literal.
    static func literal(_ payload: String) -> String {
        "{\(payload.utf8.count)}\r\n" + payload
    }

    /// Parses a complete transcript in one feed.
    static func parseAll(_ transcript: String) throws -> [IMAPResponse] {
        var parser = IMAPResponseParser()
        return try parser.feed(Data(transcript.utf8))
    }
}
