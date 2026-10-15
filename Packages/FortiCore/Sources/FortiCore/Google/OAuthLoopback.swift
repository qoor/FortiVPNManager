import Foundation

/// The request head the browser sends to the loopback redirect listener (only what OAuth needs).
public struct LoopbackHTTPRequest: Sendable, Equatable {
    public struct QueryItem: Sendable, Equatable {
        public var name: String
        public var value: String

        public init(_ name: String, _ value: String) {
            self.name = name
            self.value = value
        }
    }

    public enum ParseResult: Sendable, Equatable {
        /// The blank line ending the head has not arrived yet.
        case incomplete
        case request(LoopbackHTTPRequest)
        /// Not an HTTP/1.x request line, bad percent-encoding, or a head over ``maxHeadBytes``.
        case malformed
    }

    /// Larger heads are refused (a real callback is well under 2 KiB).
    public static let maxHeadBytes = 8 * 1024

    public var method: String
    /// Path of the request target without the query, e.g. `/` or `/favicon.ico`.
    public var path: String
    public var queryItems: [QueryItem]

    public init(method: String, path: String, queryItems: [QueryItem] = []) {
        self.method = method
        self.path = path
        self.queryItems = queryItems
    }

    /// The first value of `name`.
    public func value(_ name: String) -> String? {
        queryItems.first { $0.name == name }?.value
    }

    /// Parses the bytes received so far. Only the request line is interpreted; header fields are skipped.
    public static func parse(_ data: Data) -> ParseResult {
        let bytes = [UInt8](data.prefix(maxHeadBytes + 4))
        guard let end = headEnd(in: bytes) else {
            return bytes.count > maxHeadBytes ? .malformed : .incomplete
        }
        guard end <= maxHeadBytes else { return .malformed }
        var lineEnd = bytes.firstIndex(of: 0x0A) ?? end
        if lineEnd > 0, bytes[lineEnd - 1] == 0x0D { lineEnd -= 1 }
        guard let line = String(bytes: bytes[..<lineEnd], encoding: .ascii) else { return .malformed }
        let parts = line.split(separator: " ", omittingEmptySubsequences: false)
        guard parts.count == 3, !parts[0].isEmpty, parts[2].hasPrefix("HTTP/1."), parts[1].hasPrefix("/") else {
            return .malformed
        }
        let target = parts[1]
        let path: Substring
        var items: [QueryItem] = []
        if let question = target.firstIndex(of: "?") {
            path = target[..<question]
            let query = target[target.index(after: question)...]
            for pair in query.split(separator: "&") where !pair.isEmpty {
                let (rawName, rawValue) = pair.firstIndex(of: "=").map { (pair[..<$0], pair[pair.index(after: $0)...]) }
                    ?? (pair[...], Substring(""))
                guard let name = formDecoded(rawName), let value = formDecoded(rawValue) else { return .malformed }
                items.append(QueryItem(name, value))
            }
        } else {
            path = target[...]
        }
        if let hash = path.firstIndex(of: "#") {
            return .request(LoopbackHTTPRequest(method: String(parts[0]), path: String(path[..<hash]), queryItems: items))
        }
        return .request(LoopbackHTTPRequest(method: String(parts[0]), path: String(path), queryItems: items))
    }

    /// Index just past `CRLF CRLF` (or `LF LF`), if present.
    private static func headEnd(in bytes: [UInt8]) -> Int? {
        var index = 0
        while index < bytes.count {
            if bytes[index] == 0x0A {
                if index + 1 < bytes.count, bytes[index + 1] == 0x0A { return index + 2 }
                if index + 2 < bytes.count, bytes[index + 1] == 0x0D, bytes[index + 2] == 0x0A { return index + 3 }
            }
            index += 1
        }
        return nil
    }

    /// `application/x-www-form-urlencoded` component: `+` is a space, `%XX` a UTF-8 byte.
    private static func formDecoded(_ text: Substring) -> String? {
        var bytes: [UInt8] = []
        var iterator = Array(text.utf8).makeIterator()
        while let byte = iterator.next() {
            switch byte {
            case 0x2B:
                bytes.append(0x20)
            case 0x25:
                guard let high = iterator.next().flatMap(hexValue), let low = iterator.next().flatMap(hexValue) else {
                    return nil
                }
                bytes.append(high << 4 | low)
            default:
                bytes.append(byte)
            }
        }
        return String(bytes: bytes, encoding: .utf8)
    }

    private static func hexValue(_ byte: UInt8) -> UInt8? {
        switch byte {
        case 0x30...0x39: byte - 0x30
        case 0x41...0x46: byte - 0x41 + 10
        case 0x61...0x66: byte - 0x61 + 10
        default: nil
        }
    }
}

/// What the loopback listener does with one browser request during the consent flow.
public enum OAuthLoopbackCallback {
    public enum Decision: Sendable, Equatable {
        /// The redirect with a matching `state` and a `code`: answer 200 and finish the flow.
        case authorized(code: String)
        /// The redirect with a matching `state` and `error` (e.g. `access_denied`): answer and fail the flow.
        case denied(error: String)
        /// A redirect without the expected `state`, or with neither code nor error: answer 400, keep waiting.
        case rejected
        /// Any other path (e.g. `/favicon.ico`): answer 404, keep waiting.
        case notFound
    }

    /// Longest accepted authorization code (Google's are about 73 characters).
    public static let maxCodeLength = 2048

    public static func decide(_ request: LoopbackHTTPRequest, expectedState: String) -> Decision {
        guard request.path == "/" else { return .notFound }
        guard request.method == "GET", !expectedState.isEmpty, request.value("state") == expectedState else {
            return .rejected
        }
        if let error = request.value("error") {
            return .denied(error: sanitizedErrorCode(error))
        }
        guard let code = request.value("code"), !code.isEmpty, code.utf8.count <= maxCodeLength,
              code.unicodeScalars.allSatisfy({ (0x21...0x7E).contains($0.value) })
        else { return .rejected }
        return .authorized(code: code)
    }

    /// OAuth error codes are `[A-Za-z0-9_]`; anything else (or a very long value) becomes `unknown_error`
    /// so it is safe to show and log.
    static func sanitizedErrorCode(_ raw: String) -> String {
        let valid = !raw.isEmpty && raw.utf8.count <= 64 && raw.utf8.allSatisfy { byte in
            switch byte {
            case 0x41...0x5A, 0x61...0x7A, 0x30...0x39, 0x5F: true
            default: false
            }
        }
        return valid ? raw : "unknown_error"
    }
}

/// Minimal HTTP/1.1 answers for the loopback listener.
public enum LoopbackHTTPResponse {
    /// A complete response with a tiny HTML page showing `message`. The connection is closed afterwards.
    public static func html(status: Int, reason: String, title: String, message: String) -> Data {
        let body = """
            <!doctype html>
            <html><head><meta charset="utf-8"><title>\(escapeHTML(title))</title></head>
            <body style="font-family: -apple-system, sans-serif; margin: 3em;"><p>\(escapeHTML(message))</p></body></html>

            """
        let bodyBytes = Data(body.utf8)
        let head = "HTTP/1.1 \(status) \(reason)\r\n"
            + "Content-Type: text/html; charset=utf-8\r\n"
            + "Content-Length: \(bodyBytes.count)\r\n"
            + "Cache-Control: no-store\r\n"
            + "Referrer-Policy: no-referrer\r\n"
            + "Connection: close\r\n\r\n"
        return Data(head.utf8) + bodyBytes
    }

    public static func escapeHTML(_ text: String) -> String {
        var out = ""
        for character in text {
            switch character {
            case "&": out += "&amp;"
            case "<": out += "&lt;"
            case ">": out += "&gt;"
            case "\"": out += "&quot;"
            case "'": out += "&#39;"
            default: out.append(character)
            }
        }
        return out
    }
}
