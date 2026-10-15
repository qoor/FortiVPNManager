import Foundation
import Testing
@testable import FortiCore

@Suite("Gmail API payloads")
struct GmailAPIModelsTests {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(json.utf8))
    }

    @Test func profile() throws {
        let json = #"{"emailAddress": "otp@example.com", "messagesTotal": 1234, "threadsTotal": 900, "historyId": "9876543"}"#
        #expect(try decode(GmailProfile.self, json) == GmailProfile(emailAddress: "otp@example.com", historyId: "9876543"))
    }

    @Test func historyList() throws {
        let json = """
            {
              "history": [
                {"id": "100", "messages": [{"id": "a1", "threadId": "a1"}],
                 "messagesAdded": [{"message": {"id": "19a0000000000001", "threadId": "t1", "labelIds": ["UNREAD", "INBOX"]}}]},
                {"id": "101", "labelsAdded": [{"message": {"id": "x"}, "labelIds": ["STARRED"]}]},
                {"id": "102", "messagesAdded": [
                  {"message": {"id": "19a0000000000002", "threadId": "t2", "labelIds": ["SPAM"]}},
                  {"message": {"id": "19a0000000000001", "threadId": "t1"}}
                ]}
              ],
              "nextPageToken": "page2",
              "historyId": "103"
            }
            """
        let list = try decode(GmailHistoryList.self, json)
        #expect(list.historyId == "103")
        #expect(list.nextPageToken == "page2")
        #expect(list.history.count == 3)
        #expect(list.addedMessages == [
            GmailMessageRef(id: "19a0000000000001", threadId: "t1", labelIds: ["UNREAD", "INBOX"]),
            GmailMessageRef(id: "19a0000000000002", threadId: "t2", labelIds: ["SPAM"]),
        ])
    }

    @Test func emptyHistoryList() throws {
        let list = try decode(GmailHistoryList.self, #"{"historyId": "555"}"#)
        #expect(list == GmailHistoryList(history: [], historyId: "555", nextPageToken: nil))
        #expect(list.addedMessages.isEmpty)
        #expect(try decode(GmailHistoryList.self, "{}").historyId == nil)
    }

    @Test func messageList() throws {
        let list = try decode(GmailMessageList.self, """
            {"messages": [{"id": "m2", "threadId": "t"}, {"id": "m1", "threadId": "t"}], "resultSizeEstimate": 2}
            """)
        #expect(list.messages.map(\.id) == ["m2", "m1"])
        #expect(list.nextPageToken == nil)
        #expect(try decode(GmailMessageList.self, #"{"resultSizeEstimate": 0}"#).messages.isEmpty)
    }

    @Test func messageMetadata() throws {
        let json = """
            {
              "id": "19a1b2c3d4e5f607",
              "threadId": "19a1b2c3d4e5f607",
              "labelIds": ["UNREAD", "IMPORTANT", "CATEGORY_UPDATES", "INBOX"],
              "snippet": "",
              "payload": {
                "partId": "", "mimeType": "text/plain", "filename": "",
                "headers": [
                  {"name": "Date", "value": "Wed, 14 Oct 2026 10:19:03 +0900"},
                  {"name": "SUBJECT", "value": " =?UTF-8?Q?AuthCode:_482913?= "}
                ]
              },
              "sizeEstimate": 3051,
              "historyId": "9876600",
              "internalDate": "1791940743123"
            }
            """
        let message = try decode(GmailMessageMetadata.self, json)
        #expect(message.id == "19a1b2c3d4e5f607")
        #expect(message.labelIds == ["UNREAD", "IMPORTANT", "CATEGORY_UPDATES", "INBOX"])
        #expect(message.internalDate == Date(timeIntervalSince1970: 1_791_940_743.123))
        #expect(message.subject == "AuthCode: 482913")
        #expect(message.header("date") == "Wed, 14 Oct 2026 10:19:03 +0900")
        #expect(message.header("From") == nil)
    }

    @Test func sparseMetadata() throws {
        let message = try decode(GmailMessageMetadata.self, #"{"id": "m", "internalDate": 1791940743000}"#)
        #expect(message.internalDate == Date(timeIntervalSince1970: 1_791_940_743))
        #expect(message.labelIds.isEmpty)
        #expect(message.headers.isEmpty)
        #expect(message.subject == nil)
        #expect(try decode(GmailMessageMetadata.self, #"{"id": "m", "internalDate": "-5"}"#).internalDate == nil)
        #expect(try decode(GmailMessageMetadata.self, #"{"id": "m", "internalDate": "12x"}"#).internalDate == nil)
        #expect(try decode(GmailMessageMetadata.self, #"{"id": "m", "payload": {}}"#).headers.isEmpty)
        #expect(throws: DecodingError.self) { try decode(GmailMessageMetadata.self, #"{"labelIds": []}"#) }
    }

    @Test(arguments: [
        ("19a1b2c3d4e5f607", true), ("abc-DEF_09", true), ("", false), ("../profile", false), ("a/b", false),
        ("a b", false), ("a%2F", false), (String(repeating: "a", count: 513), false),
    ])
    func safeIdentifiers(value: String, safe: Bool) {
        #expect(GmailAPI.isSafeIdentifier(value) == safe)
    }

    @Test(arguments: [
        ("1", true), ("18446744073709551615", true), ("18446744073709551616", false), ("", false), ("-1", false),
        ("1 ", false), ("0x10", false),
    ])
    func historyIDs(value: String, valid: Bool) {
        #expect(GmailAPI.isValidHistoryID(value) == valid)
    }
}

@Suite("Gmail API error classification")
struct GmailAPIErrorTests {
    private func classify(_ status: Int, _ body: String = "") -> GmailAPIError? {
        GmailAPIError.classify(status: status, body: Data(body.utf8))
    }

    @Test func successIsNoError() {
        #expect(classify(200) == nil)
        #expect(classify(204) == nil)
    }

    @Test func statusClasses() {
        let unauthorized = #"{"error": {"code": 401, "message": "Request had invalid authentication credentials.", "status": "UNAUTHENTICATED"}}"#
        #expect(classify(401, unauthorized) == .unauthorized)
        #expect(classify(404, #"{"error": {"code": 404, "message": "Requested entity was not found.", "status": "NOT_FOUND"}}"#) == .notFound)
        #expect(classify(429) == .rateLimited)
        #expect(classify(500) == .server(status: 500))
        #expect(classify(503, "<html>") == .server(status: 503))
        #expect(classify(400, #"{"error": {"code": 400, "status": "INVALID_ARGUMENT"}}"#) == .server(status: 400))
    }

    @Test("403 reasons", arguments: [
        (#"{"error": {"code": 403, "errors": [{"domain": "usageLimits", "reason": "rateLimitExceeded"}], "status": "PERMISSION_DENIED"}}"#,
         GmailAPIError.rateLimited),
        (#"{"error": {"errors": [{"reason": "userRateLimitExceeded"}]}}"#, .rateLimited),
        (#"{"error": {"code": 403, "errors": [{"domain": "global", "reason": "insufficientPermissions"}], "status": "PERMISSION_DENIED"}}"#,
         .forbidden(reason: "insufficientPermissions")),
        (#"{"error": {"code": 403, "status": "PERMISSION_DENIED", "details": [{"@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "ACCESS_TOKEN_SCOPE_INSUFFICIENT"}]}}"#,
         .forbidden(reason: "ACCESS_TOKEN_SCOPE_INSUFFICIENT")),
        (#"{"error": {"code": 403, "status": "PERMISSION_DENIED", "details": [{"reason": "SERVICE_DISABLED"}], "errors": [{"reason": "accessNotConfigured"}]}}"#,
         .forbidden(reason: "accessNotConfigured")),
        (#"{"error": {"code": 403, "status": "PERMISSION_DENIED"}}"#, .forbidden(reason: "PERMISSION_DENIED")),
        (#"{"error": {"errors": [{"reason": "bad reason <x>"}]}}"#, .forbidden(reason: "badreasonx")),
        (#"{"error": {"errors": [{"reason": "!!"}], "status": "PERMISSION_DENIED"}}"#, .forbidden(reason: "PERMISSION_DENIED")),
        ("", .forbidden(reason: "forbidden")),
        ("not json", .forbidden(reason: "forbidden")),
    ])
    func forbidden(body: String, expected: GmailAPIError) {
        #expect(classify(403, body) == expected)
    }
}
