import Foundation
import Testing
@testable import FortiCore

@Suite("OAuth loopback: request parsing")
struct LoopbackHTTPRequestTests {
    private func parse(_ text: String) -> LoopbackHTTPRequest.ParseResult {
        LoopbackHTTPRequest.parse(Data(text.utf8))
    }

    @Test func googleRedirect() {
        let head = "GET /?state=abc_123&code=4%2F0AVG7fiQ-x_y&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.modify HTTP/1.1\r\n"
            + "Host: 127.0.0.1:53122\r\nUser-Agent: Mozilla/5.0\r\nAccept: text/html\r\n\r\n"
        #expect(parse(head) == .request(LoopbackHTTPRequest(method: "GET", path: "/", queryItems: [
            .init("state", "abc_123"),
            .init("code", "4/0AVG7fiQ-x_y"),
            .init("scope", "https://www.googleapis.com/auth/gmail.modify"),
        ])))
    }

    @Test("Incomplete heads wait for more bytes", arguments: [
        "", "GET", "GET /?code=1 HTTP/1.1\r\n", "GET /?code=1 HTTP/1.1\r\nHost: x\r\n", "GET / HTTP/1.1\r\n\r",
    ])
    func incomplete(text: String) {
        #expect(parse(text) == .incomplete)
    }

    @Test func bareLineFeedsAndTrailingBodyBytes() {
        #expect(parse("GET /favicon.ico HTTP/1.0\n\nextra") == .request(LoopbackHTTPRequest(method: "GET", path: "/favicon.ico")))
    }

    @Test func formDecodingAndFragments() {
        let result = parse("GET /?a=x+y&b=%E2%9C%93&c&=v&d=1=2#frag HTTP/1.1\r\n\r\n")
        #expect(result == .request(LoopbackHTTPRequest(method: "GET", path: "/", queryItems: [
            .init("a", "x y"), .init("b", "\u{2713}"), .init("c", ""), .init("", "v"), .init("d", "1=2#frag"),
        ])))
        #expect(parse("GET /#x HTTP/1.1\r\n\r\n") == .request(LoopbackHTTPRequest(method: "GET", path: "/")))
    }

    @Test("Malformed request lines", arguments: [
        "\r\n\r\n",
        "GET\r\n\r\n",
        "GET / \r\n\r\n",
        "GET /  HTTP/1.1\r\n\r\n",
        "GET http://127.0.0.1/ HTTP/1.1\r\n\r\n",
        "GET / HTTP/2\r\n\r\n",
        "GET /?code=%ZZ HTTP/1.1\r\n\r\n",
        "GET /?code=%E2%9C HTTP/1.1\r\n\r\n",
        "GET /?code=%4 HTTP/1.1\r\n\r\n",
    ])
    func malformed(text: String) {
        #expect(parse(text) == .malformed)
    }

    @Test func nonASCIIRequestLineIsMalformed() {
        var bytes = Data("GET /?x=".utf8)
        bytes.append(0xFF)
        bytes.append(contentsOf: Data(" HTTP/1.1\r\n\r\n".utf8))
        #expect(LoopbackHTTPRequest.parse(bytes) == .malformed)
    }

    @Test func sizeLimit() {
        let limit = LoopbackHTTPRequest.maxHeadBytes
        let filler = String(repeating: "a", count: limit)
        #expect(parse("GET /?x=\(filler) HTTP/1.1\r\n") == .malformed)
        #expect(parse("GET /?x=" + String(repeating: "a", count: 3000)) == .incomplete)
        let fits = "GET / HTTP/1.1\r\nX: " + String(repeating: "b", count: limit - 23) + "\r\n\r\n"
        #expect(fits.utf8.count == limit)
        #expect(parse(fits) == .request(LoopbackHTTPRequest(method: "GET", path: "/")))
        #expect(parse("GET / HTTP/1.1\r\nX: " + String(repeating: "b", count: limit - 22) + "\r\n\r\n") == .malformed)
    }
}

@Suite("OAuth loopback: callback decisions")
struct OAuthLoopbackCallbackTests {
    private func decide(_ target: String, method: String = "GET", state: String = "expected-state") -> OAuthLoopbackCallback.Decision {
        guard case let .request(request) = LoopbackHTTPRequest.parse(Data("\(method) \(target) HTTP/1.1\r\n\r\n".utf8)) else {
            Issue.record("unparsable \(target)")
            return .rejected
        }
        return OAuthLoopbackCallback.decide(request, expectedState: state)
    }

    @Test func authorized() {
        #expect(decide("/?state=expected-state&code=4%2F0Ab-cd_EF&scope=x") == .authorized(code: "4/0Ab-cd_EF"))
        #expect(decide("/?code=abc&state=expected-state") == .authorized(code: "abc"))
    }

    @Test func denied() {
        #expect(decide("/?error=access_denied&state=expected-state") == .denied(error: "access_denied"))
        #expect(decide("/?error=%3Cscript%3E&state=expected-state") == .denied(error: "unknown_error"))
        #expect(decide("/?error=&state=expected-state") == .denied(error: "unknown_error"))
        // An error wins over a code.
        #expect(decide("/?error=access_denied&code=abc&state=expected-state") == .denied(error: "access_denied"))
    }

    @Test("Wrong or missing state, or no code, is rejected", arguments: [
        "/?code=abc",
        "/?code=abc&state=other",
        "/?code=abc&state=expected-state-but-longer",
        "/?code=abc&state=",
        "/?error=access_denied&state=other",
        "/?state=expected-state",
        "/?state=expected-state&code=",
        "/?state=expected-state&code=a+b",
        "/?state=expected-state&code=%0A",
        "/",
    ])
    func rejected(target: String) {
        #expect(decide(target) == .rejected)
    }

    @Test func firstStateValueCounts() {
        #expect(decide("/?state=other&state=expected-state&code=abc") == .rejected)
    }

    @Test func otherPathsAreNotFound() {
        #expect(decide("/favicon.ico") == .notFound)
        #expect(decide("/callback?state=expected-state&code=abc") == .notFound)
    }

    @Test func onlyGET() {
        #expect(decide("/?state=expected-state&code=abc", method: "POST") == .rejected)
        #expect(decide("/?state=expected-state&code=abc", method: "HEAD") == .rejected)
    }

    @Test func emptyExpectedStateNeverMatches() {
        #expect(decide("/?state=&code=abc", state: "") == .rejected)
    }

    @Test func overlongCodeIsRejected() {
        let long = String(repeating: "a", count: OAuthLoopbackCallback.maxCodeLength + 1)
        #expect(decide("/?state=expected-state&code=\(long)") == .rejected)
    }
}

@Suite("OAuth loopback: responses")
struct LoopbackHTTPResponseTests {
    @Test func htmlResponse() throws {
        let data = LoopbackHTTPResponse.html(status: 200, reason: "OK", title: "FortiVPNManager",
                                             message: "FortiVPNManager: you can close this window.")
        let text = String(decoding: data, as: UTF8.self)
        let parts = text.components(separatedBy: "\r\n\r\n")
        #expect(parts.count == 2)
        let head = try #require(parts.first)
        let body = try #require(parts.last)
        #expect(head.hasPrefix("HTTP/1.1 200 OK\r\n"))
        #expect(head.contains("Content-Type: text/html; charset=utf-8"))
        #expect(head.contains("Content-Length: \(body.utf8.count)"))
        #expect(head.contains("Connection: close"))
        #expect(head.contains("Cache-Control: no-store"))
        #expect(body.contains("<p>FortiVPNManager: you can close this window.</p>"))
    }

    @Test func escapesMarkup() {
        #expect(LoopbackHTTPResponse.escapeHTML(#"<a href="x">'&'</a>"#)
            == "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;")
        let data = LoopbackHTTPResponse.html(status: 400, reason: "Bad Request", title: "<t>", message: "\u{D55C} <b>")
        let text = String(decoding: data, as: UTF8.self)
        #expect(text.contains("<title>&lt;t&gt;</title>"))
        #expect(text.contains("<p>\u{D55C} &lt;b&gt;</p>"))
        #expect(text.hasPrefix("HTTP/1.1 400 Bad Request\r\n"))
    }
}
