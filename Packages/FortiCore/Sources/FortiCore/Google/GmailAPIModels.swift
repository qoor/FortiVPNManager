import Foundation

/// Gmail REST API v1 (`https://gmail.googleapis.com/gmail/v1/users/me/...`) payloads the OTP session
/// reads. Decoding is lenient: absent arrays decode as empty, unknown fields are ignored.
public enum GmailAPI {
    public static let baseURL = URL(string: "https://gmail.googleapis.com/gmail/v1/users/me/")!

    /// Message IDs and page tokens go into URL paths and queries; Gmail's are URL-safe already.
    public static func isSafeIdentifier(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= 512 && value.utf8.allSatisfy { byte in
            switch byte {
            case 0x41...0x5A, 0x61...0x7A, 0x30...0x39, 0x2D, 0x5F: true
            default: false
            }
        }
    }

    /// History IDs are unsigned 64-bit integers sent as decimal strings.
    public static func isValidHistoryID(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= 20 && UInt64(value) != nil
    }
}

/// `users.getProfile`.
public struct GmailProfile: Decodable, Sendable, Equatable {
    public var emailAddress: String
    /// Current mailbox history ID; `history.list` from here reports only later changes.
    public var historyId: String

    public init(emailAddress: String, historyId: String) {
        self.emailAddress = emailAddress
        self.historyId = historyId
    }
}

/// A message reference as it appears in `history.list` and `messages.list`.
public struct GmailMessageRef: Decodable, Sendable, Equatable, Hashable {
    public var id: String
    public var threadId: String?
    public var labelIds: [String]

    public init(id: String, threadId: String? = nil, labelIds: [String] = []) {
        self.id = id
        self.threadId = threadId
        self.labelIds = labelIds
    }

    enum CodingKeys: String, CodingKey {
        case id, threadId, labelIds
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        threadId = try container.decodeIfPresent(String.self, forKey: .threadId)
        labelIds = try container.decodeIfPresent([String].self, forKey: .labelIds) ?? []
    }
}

/// `users.history.list` with `historyTypes=messageAdded`.
public struct GmailHistoryList: Decodable, Sendable, Equatable {
    public struct Record: Decodable, Sendable, Equatable {
        public var id: String?
        public var messagesAdded: [GmailMessageRef]

        public init(id: String? = nil, messagesAdded: [GmailMessageRef]) {
            self.id = id
            self.messagesAdded = messagesAdded
        }

        private struct Added: Decodable {
            var message: GmailMessageRef
        }

        enum CodingKeys: String, CodingKey {
            case id, messagesAdded
        }

        public init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            id = try container.decodeIfPresent(String.self, forKey: .id)
            messagesAdded = try container.decodeIfPresent([Added].self, forKey: .messagesAdded)?.map(\.message) ?? []
        }
    }

    public var history: [Record]
    /// The mailbox's current history ID (the next `startHistoryId`).
    public var historyId: String?
    public var nextPageToken: String?

    public init(history: [Record] = [], historyId: String? = nil, nextPageToken: String? = nil) {
        self.history = history
        self.historyId = historyId
        self.nextPageToken = nextPageToken
    }

    enum CodingKeys: String, CodingKey {
        case history, historyId, nextPageToken
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        history = try container.decodeIfPresent([Record].self, forKey: .history) ?? []
        historyId = try container.decodeIfPresent(String.self, forKey: .historyId)
        nextPageToken = try container.decodeIfPresent(String.self, forKey: .nextPageToken)
    }

    /// Every added message in history order, first occurrence only.
    public var addedMessages: [GmailMessageRef] {
        var seen = Set<String>()
        return history.flatMap(\.messagesAdded).filter { seen.insert($0.id).inserted }
    }
}

/// `users.messages.list`.
public struct GmailMessageList: Decodable, Sendable, Equatable {
    public var messages: [GmailMessageRef]
    public var nextPageToken: String?

    public init(messages: [GmailMessageRef] = [], nextPageToken: String? = nil) {
        self.messages = messages
        self.nextPageToken = nextPageToken
    }

    enum CodingKeys: String, CodingKey {
        case messages, nextPageToken
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        messages = try container.decodeIfPresent([GmailMessageRef].self, forKey: .messages) ?? []
        nextPageToken = try container.decodeIfPresent(String.self, forKey: .nextPageToken)
    }
}

/// `users.messages.get` with `format=metadata` (headers only, no body).
public struct GmailMessageMetadata: Decodable, Sendable, Equatable {
    public struct Header: Decodable, Sendable, Equatable {
        public var name: String
        public var value: String

        public init(name: String, value: String) {
            self.name = name
            self.value = value
        }
    }

    public var id: String
    public var labelIds: [String]
    /// When Gmail received the message (`internalDate`, milliseconds since the epoch).
    public var internalDate: Date?
    public var headers: [Header]

    public init(id: String, labelIds: [String] = [], internalDate: Date?, headers: [Header]) {
        self.id = id
        self.labelIds = labelIds
        self.internalDate = internalDate
        self.headers = headers
    }

    private struct Payload: Decodable {
        var headers: [Header]?
    }

    enum CodingKeys: String, CodingKey {
        case id, labelIds, internalDate, payload
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        labelIds = try container.decodeIfPresent([String].self, forKey: .labelIds) ?? []
        if let text = try? container.decodeIfPresent(String.self, forKey: .internalDate) {
            internalDate = Self.date(millisecondsText: text)
        } else if let number = try? container.decodeIfPresent(Int64.self, forKey: .internalDate) {
            internalDate = Date(timeIntervalSince1970: TimeInterval(number) / 1000)
        } else {
            internalDate = nil
        }
        headers = try container.decodeIfPresent(Payload.self, forKey: .payload)?.headers ?? []
    }

    /// `"1791940743123"` → that instant; `nil` unless the text is a non-negative decimal integer.
    public static func date(millisecondsText text: String) -> Date? {
        guard !text.isEmpty, text.utf8.allSatisfy({ (0x30...0x39).contains($0) }), let value = Int64(text) else {
            return nil
        }
        return Date(timeIntervalSince1970: TimeInterval(value) / 1000)
    }

    /// The first header named `name` (case-insensitive), RFC 2047-decoded when it carries encoded-words.
    public func header(_ name: String) -> String? {
        headers.first { $0.name.caseInsensitiveCompare(name) == .orderedSame }
            .map { RFC2047.decode($0.value.trimmingCharacters(in: .whitespaces)) }
    }

    public var subject: String? { header("Subject") }
}

/// How a Gmail API call failed, classified from the HTTP status and Google's error body.
public enum GmailAPIError: Error, Sendable, Equatable {
    /// 401: the access token is missing, expired or revoked.
    case unauthorized
    /// 403 for a reason other than a rate limit, e.g. `insufficientPermissions`,
    /// `ACCESS_TOKEN_SCOPE_INSUFFICIENT` or `accessNotConfigured` (Gmail API disabled in the project).
    case forbidden(reason: String)
    /// 404: unknown message, or a `startHistoryId` that is too old.
    case notFound
    /// 429, or 403 with a rate-limit reason.
    case rateLimited
    /// Transport failure (DNS, TLS, timeout, offline). The text is for logs.
    case network(String)
    /// Any other status (5xx, unexpected 4xx) or a 2xx body that could not be decoded.
    case server(status: Int)

    static let rateLimitReasons: Set<String> = [
        "rateLimitExceeded", "userRateLimitExceeded", "dailyLimitExceeded", "quotaExceeded", "RATE_LIMIT_EXCEEDED",
    ]

    private struct ErrorBody: Decodable {
        struct Inner: Decodable {
            struct Item: Decodable {
                var reason: String?
            }

            struct Detail: Decodable {
                var reason: String?
            }

            var status: String?
            var errors: [Item]?
            var details: [Detail]?
        }

        var error: Inner?
    }

    /// `nil` for 2xx; otherwise the classified error.
    public static func classify(status: Int, body: Data) -> GmailAPIError? {
        switch status {
        case 200...299:
            return nil
        case 401:
            return .unauthorized
        case 404:
            return .notFound
        case 429:
            return .rateLimited
        case 403:
            let reason = reason(in: body) ?? "forbidden"
            return rateLimitReasons.contains(reason) ? .rateLimited : .forbidden(reason: reason)
        default:
            return .server(status: status)
        }
    }

    /// The most specific reason in a Google error body: `errors[].reason`, then `details[].reason`,
    /// then `status`. Sanitized to `[A-Za-z0-9_]` so it is safe to log.
    static func reason(in body: Data) -> String? {
        guard let inner = (try? JSONDecoder().decode(ErrorBody.self, from: body))?.error else { return nil }
        let candidates = (inner.errors ?? []).compactMap(\.reason) + (inner.details ?? []).compactMap(\.reason)
            + [inner.status].compactMap(\.self)
        return candidates.lazy.map(sanitized).first { !$0.isEmpty }
    }

    private static func sanitized(_ text: String) -> String {
        String(String.UnicodeScalarView(text.unicodeScalars.prefix(64).filter { scalar in
            switch scalar.value {
            case 0x41...0x5A, 0x61...0x7A, 0x30...0x39, 0x5F: true
            default: false
            }
        }))
    }
}
