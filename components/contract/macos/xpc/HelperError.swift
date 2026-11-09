import Foundation

/// Error returned in XPC replies.
struct HelperError: Error, Codable, Sendable, Equatable {
    enum Code: String, Codable, Sendable {
        case busy
        case invalidRequest
        case unknownAttempt
        case wrongPhase
        case binaryUnavailable
        case notAuthorized
        case internalError
    }

    var code: Code
    var message: String

    init(_ code: Code, _ message: String) {
        self.code = code
        self.message = message
    }
}
