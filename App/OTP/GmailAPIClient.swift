import Foundation
import FortiCore

/// The Gmail REST API calls the OTP session and the Account tab use. Each call takes a fresh access
/// token (see `GoogleOAuthService.accessToken(for:)`) and fails with a typed `GmailAPIError`.
nonisolated struct GmailAPIClient: Sendable {
    var http: GoogleHTTP = .shared
    var baseURL: URL = GmailAPI.baseURL

    /// `users.getProfile`: the signed-in address and the current history ID.
    func profile(accessToken: String) async throws(GmailAPIError) -> GmailProfile {
        try await getJSON(GmailProfile.self, path: "profile", query: [], accessToken: accessToken)
    }

    /// `users.history.list` of `messageAdded` records after `startHistoryID`. A too-old or invalid
    /// history ID fails with `.notFound`.
    func history(accessToken: String, startHistoryID: String, pageToken: String? = nil) async throws(GmailAPIError) -> GmailHistoryList {
        guard GmailAPI.isValidHistoryID(startHistoryID) else { throw .notFound }
        var query = [("startHistoryId", startHistoryID), ("historyTypes", "messageAdded"), ("maxResults", "100")]
        if let pageToken { query.append(("pageToken", pageToken)) }
        return try await getJSON(GmailHistoryList.self, path: "history", query: query, accessToken: accessToken)
    }

    /// `users.messages.get` with `format=metadata`: labels, `internalDate` and the Subject/Date headers.
    func messageMetadata(accessToken: String, id: String) async throws(GmailAPIError) -> GmailMessageMetadata {
        guard GmailAPI.isSafeIdentifier(id) else { throw .notFound }
        let query = [("format", "metadata"), ("metadataHeaders", "Subject"), ("metadataHeaders", "Date")]
        return try await getJSON(GmailMessageMetadata.self, path: "messages/\(id)", query: query, accessToken: accessToken)
    }

    /// `users.messages.trash`: moves the message to Trash (Gmail deletes it after 30 days).
    func trash(accessToken: String, id: String) async throws(GmailAPIError) {
        guard GmailAPI.isSafeIdentifier(id) else { throw .notFound }
        let response: GoogleHTTP.Response
        do {
            response = try await http.post(url(path: "messages/\(id)/trash", query: []), bearer: accessToken)
        } catch {
            throw .network(error.detail)
        }
        if let error = GmailAPIError.classify(status: response.status, body: response.body) { throw error }
    }

    /// `users.messages.list` for a Gmail search query (newest first).
    func listMessages(
        accessToken: String, query searchQuery: String, includeSpamTrash: Bool = false, maxResults: Int = 10
    ) async throws(GmailAPIError) -> GmailMessageList {
        let query = [
            ("q", searchQuery),
            ("maxResults", String(maxResults)),
            ("includeSpamTrash", includeSpamTrash ? "true" : "false"),
        ]
        return try await getJSON(GmailMessageList.self, path: "messages", query: query, accessToken: accessToken)
    }

    // MARK: - Plumbing

    private func getJSON<T: Decodable & Sendable>(
        _ type: T.Type, path: String, query: [(String, String)], accessToken: String
    ) async throws(GmailAPIError) -> T {
        let response: GoogleHTTP.Response
        do {
            response = try await http.get(url(path: path, query: query), bearer: accessToken)
        } catch {
            throw .network(error.detail)
        }
        if let error = GmailAPIError.classify(status: response.status, body: response.body) { throw error }
        do {
            return try JSONDecoder().decode(T.self, from: response.body)
        } catch {
            throw .server(status: response.status)
        }
    }

    private func url(path: String, query: [(String, String)]) -> URL {
        let base = baseURL.appending(path: path)
        guard !query.isEmpty else { return base }
        return URL(string: base.absoluteString + "?" + GoogleOAuth.formEncoded(query)) ?? base
    }
}
