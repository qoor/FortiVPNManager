import Foundation

/// A transport-level failure talking to Google (DNS, TLS, timeout, offline). The text is for logs
/// and error messages; it never contains request bodies or tokens.
nonisolated struct GoogleTransportError: Error, Sendable, Equatable {
    var detail: String
}

/// HTTPS to Google's OAuth and Gmail endpoints with one ephemeral `URLSession` (no cookies, no cache)
/// and a 10-second budget per request. Only the async URLSession API is used, so no completion
/// closure crosses into the main-actor app code.
nonisolated struct GoogleHTTP: Sendable {
    static let requestTimeout: TimeInterval = 10
    static let shared = GoogleHTTP()

    struct Response: Sendable {
        var status: Int
        var body: Data
    }

    let session: URLSession

    init() {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = Self.requestTimeout
        configuration.timeoutIntervalForResource = Self.requestTimeout
        configuration.waitsForConnectivity = false
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        session = URLSession(configuration: configuration)
    }

    func get(_ url: URL, bearer: String) async throws(GoogleTransportError) -> Response {
        var request = URLRequest(url: url, timeoutInterval: Self.requestTimeout)
        request.httpMethod = "GET"
        request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        return try await send(request)
    }

    /// POST without a body (e.g. `messages.trash`).
    func post(_ url: URL, bearer: String) async throws(GoogleTransportError) -> Response {
        var request = URLRequest(url: url, timeoutInterval: Self.requestTimeout)
        request.httpMethod = "POST"
        request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("0", forHTTPHeaderField: "Content-Length")
        return try await send(request)
    }

    /// POST of an `application/x-www-form-urlencoded` body (token and revocation endpoints).
    func postForm(_ url: URL, body: Data) async throws(GoogleTransportError) -> Response {
        var request = URLRequest(url: url, timeoutInterval: Self.requestTimeout)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = body
        return try await send(request)
    }

    private func send(_ request: URLRequest) async throws(GoogleTransportError) -> Response {
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch let error as URLError {
            throw GoogleTransportError(detail: "\(error.localizedDescription) (URLError \(error.code.rawValue))")
        } catch {
            throw GoogleTransportError(detail: error.localizedDescription)
        }
        guard let http = response as? HTTPURLResponse else {
            throw GoogleTransportError(detail: "not an HTTP response")
        }
        return Response(status: http.statusCode, body: data)
    }
}
